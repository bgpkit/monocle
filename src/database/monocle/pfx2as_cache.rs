//! Pfx2as cache repository for the DuckDB database
//!
//! This module provides caching for Pfx2as data to avoid reloading from
//! external sources on every command. The cache leverages DuckDB's native
//! INET type for efficient prefix matching queries.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use duckdb::params;
use std::time::Duration;
use tracing::info;

use crate::database::core::DuckDbConn;

/// Default TTL for Pfx2as data (24 hours)
pub const DEFAULT_PFX2AS_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Pfx2as record representing a prefix-to-origin mapping
#[derive(Debug, Clone)]
pub struct Pfx2asRecord {
    pub prefix: String,
    pub origin_asns: Vec<u32>,
}

/// Cache metadata for Pfx2as data
#[derive(Debug, Clone)]
pub struct Pfx2asCacheMeta {
    pub id: i64,
    pub data_source: String,
    pub loaded_at: DateTime<Utc>,
    pub record_count: u64,
}

/// Parse a DuckDB array string like "[1, 2, 3]" into Vec<u32>
fn parse_array_string(s: &str) -> Vec<u32> {
    let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed
        .split(',')
        .filter_map(|part| part.trim().parse::<u32>().ok())
        .collect()
}

/// Repository for Pfx2as cache operations
pub struct Pfx2asCacheRepository<'a> {
    conn: &'a DuckDbConn,
}

impl<'a> Pfx2asCacheRepository<'a> {
    /// Create a new Pfx2as cache repository
    pub fn new(conn: &'a DuckDbConn) -> Self {
        Self { conn }
    }

    // =========================================================================
    // Cache Metadata Operations
    // =========================================================================

    /// Get cache metadata for a specific data source
    pub fn get_cache_meta(&self, data_source: &str) -> Result<Option<Pfx2asCacheMeta>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT id, data_source, loaded_at::TEXT, record_count
             FROM pfx2as_cache_meta
             WHERE data_source = ?
             ORDER BY loaded_at DESC
             LIMIT 1",
        )?;

        let result = stmt.query_row(params![data_source], |row| {
            let loaded_at_str: String = row.get(2)?;
            let loaded_at = DateTime::parse_from_rfc3339(&loaded_at_str)
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(&loaded_at_str, "%Y-%m-%d %H:%M:%S")
                        .map(|dt| dt.and_utc().fixed_offset())
                })
                .unwrap_or_else(|_| Utc::now().fixed_offset());

            Ok(Pfx2asCacheMeta {
                id: row.get(0)?,
                data_source: row.get(1)?,
                loaded_at: loaded_at.with_timezone(&Utc),
                record_count: row.get(3)?,
            })
        });

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get Pfx2as cache metadata: {}", e)),
        }
    }

    /// Check if cached data is fresh (within TTL)
    pub fn is_cache_fresh(&self, data_source: &str, ttl: Duration) -> bool {
        if let Ok(Some(meta)) = self.get_cache_meta(data_source) {
            let age = Utc::now().signed_duration_since(meta.loaded_at);
            return age.num_seconds() < ttl.as_secs() as i64;
        }
        false
    }

    /// Get the latest cache ID for a data source
    fn get_latest_cache_id(&self, data_source: &str) -> Result<Option<i64>> {
        if let Some(meta) = self.get_cache_meta(data_source)? {
            Ok(Some(meta.id))
        } else {
            Ok(None)
        }
    }

    /// Create a new cache entry and return its ID
    fn create_cache_entry(&self, data_source: &str, record_count: u64) -> Result<i64> {
        // Get next ID
        let next_id: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM pfx2as_cache_meta",
                |row| row.get(0),
            )
            .unwrap_or(1);

        self.conn.conn.execute(
            "INSERT INTO pfx2as_cache_meta (id, data_source, loaded_at, record_count)
             VALUES (?, ?, current_timestamp, ?)",
            params![next_id, data_source, record_count],
        )?;

        Ok(next_id)
    }

    // =========================================================================
    // Pfx2as Data Operations
    // =========================================================================

    /// Get prefix count for a specific cache
    pub fn count(&self, cache_id: Option<i64>) -> Result<u64> {
        let query = match cache_id {
            Some(id) => format!("SELECT COUNT(*) FROM pfx2as WHERE cache_id = {}", id),
            None => "SELECT COUNT(*) FROM pfx2as".to_string(),
        };
        self.conn.query_row(&query, |row| row.get(0))
    }

    /// Store Pfx2as data in the cache
    pub fn store(&self, data_source: &str, records: &[Pfx2asRecord]) -> Result<i64> {
        info!(
            "Storing {} Pfx2as records from {}",
            records.len(),
            data_source
        );

        // Clear existing cache for this source
        self.clear(data_source)?;

        // Create new cache entry
        let cache_id = self.create_cache_entry(data_source, records.len() as u64)?;

        // Use transaction for bulk insert
        self.conn.transaction()?;

        {
            let mut stmt = self.conn.conn.prepare(
                "INSERT INTO pfx2as (prefix, origin_asns, cache_id)
                 VALUES (?::INET, ?, ?)",
            )?;

            for record in records {
                // Convert origin_asns to DuckDB array format
                let asns_str = format!(
                    "[{}]",
                    record
                        .origin_asns
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );

                stmt.execute(params![&record.prefix, asns_str, cache_id,])?;
            }
        }

        self.conn.commit()?;

        info!(
            "Stored {} Pfx2as records with cache_id {}",
            records.len(),
            cache_id
        );
        Ok(cache_id)
    }

    /// Clear Pfx2as data for a specific source
    pub fn clear(&self, data_source: &str) -> Result<()> {
        if let Some(cache_id) = self.get_latest_cache_id(data_source)? {
            self.conn
                .execute(&format!("DELETE FROM pfx2as WHERE cache_id = {}", cache_id))?;
            self.conn.execute(&format!(
                "DELETE FROM pfx2as_cache_meta WHERE id = {}",
                cache_id
            ))?;
        }
        Ok(())
    }

    /// Lookup prefix by exact match
    pub fn lookup_exact(&self, prefix: &str, data_source: &str) -> Result<Option<Pfx2asRecord>> {
        let cache_id = match self.get_latest_cache_id(data_source)? {
            Some(id) => id,
            None => return Ok(None),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, origin_asns::TEXT
             FROM pfx2as
             WHERE prefix = ?::INET AND cache_id = ?",
        )?;

        let result = stmt.query_row(params![prefix, cache_id], |row| {
            let asns_str: String = row.get(1)?;
            Ok(Pfx2asRecord {
                prefix: row.get(0)?,
                origin_asns: parse_array_string(&asns_str),
            })
        });

        match result {
            Ok(record) => Ok(Some(record)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to lookup prefix: {}", e)),
        }
    }

    /// Lookup prefix using longest prefix match
    ///
    /// This finds the most specific prefix that covers the given prefix/IP.
    /// For example, if querying for "10.1.1.0/24", it might return "10.1.0.0/16"
    /// if that's the most specific covering prefix in the database.
    pub fn lookup_longest_match(
        &self,
        prefix: &str,
        data_source: &str,
    ) -> Result<Option<Pfx2asRecord>> {
        let cache_id = match self.get_latest_cache_id(data_source)? {
            Some(id) => id,
            None => return Ok(None),
        };

        // Use string extraction to get prefix length since masklen isn't available
        // The prefix is stored as "x.x.x.x/N" so we extract N for ordering
        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, origin_asns::TEXT
             FROM pfx2as
             WHERE prefix >>= ?::INET AND cache_id = ?
             ORDER BY CAST(split_part(prefix::TEXT, '/', 2) AS INTEGER) DESC
             LIMIT 1",
        )?;

        let result = stmt.query_row(params![prefix, cache_id], |row| {
            let asns_str: String = row.get(1)?;
            Ok(Pfx2asRecord {
                prefix: row.get(0)?,
                origin_asns: parse_array_string(&asns_str),
            })
        });

        match result {
            Ok(record) => Ok(Some(record)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to lookup prefix: {}", e)),
        }
    }

    /// Find all prefixes covering a given prefix (super-prefixes)
    pub fn lookup_covering(&self, prefix: &str, data_source: &str) -> Result<Vec<Pfx2asRecord>> {
        let cache_id = match self.get_latest_cache_id(data_source)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, origin_asns::TEXT
             FROM pfx2as
             WHERE prefix >>= ?::INET AND cache_id = ?
             ORDER BY CAST(split_part(prefix::TEXT, '/', 2) AS INTEGER) DESC",
        )?;

        let mut rows = stmt.query(params![prefix, cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let asns_str: String = row.get(1)?;
            records.push(Pfx2asRecord {
                prefix: row.get(0)?,
                origin_asns: parse_array_string(&asns_str),
            });
        }

        Ok(records)
    }

    /// Find all prefixes covered by a given prefix (sub-prefixes)
    pub fn lookup_covered(&self, prefix: &str, data_source: &str) -> Result<Vec<Pfx2asRecord>> {
        let cache_id = match self.get_latest_cache_id(data_source)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, origin_asns::TEXT
             FROM pfx2as
             WHERE prefix <<= ?::INET AND cache_id = ?
             ORDER BY CAST(split_part(prefix::TEXT, '/', 2) AS INTEGER)",
        )?;

        let mut rows = stmt.query(params![prefix, cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let asns_str: String = row.get(1)?;
            records.push(Pfx2asRecord {
                prefix: row.get(0)?,
                origin_asns: parse_array_string(&asns_str),
            });
        }

        Ok(records)
    }

    /// Find all prefixes originated by a specific ASN
    pub fn lookup_by_origin(
        &self,
        origin_asn: u32,
        data_source: &str,
    ) -> Result<Vec<Pfx2asRecord>> {
        let cache_id = match self.get_latest_cache_id(data_source)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, origin_asns::TEXT
             FROM pfx2as
             WHERE list_contains(origin_asns, ?) AND cache_id = ?
             ORDER BY prefix",
        )?;

        let mut rows = stmt.query(params![origin_asn, cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let asns_str: String = row.get(1)?;
            records.push(Pfx2asRecord {
                prefix: row.get(0)?,
                origin_asns: parse_array_string(&asns_str),
            });
        }

        Ok(records)
    }

    /// Get all Pfx2as records from cache
    pub fn get_all(&self, data_source: &str) -> Result<Vec<Pfx2asRecord>> {
        let cache_id = match self.get_latest_cache_id(data_source)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, origin_asns::TEXT
             FROM pfx2as
             WHERE cache_id = ?
             ORDER BY prefix",
        )?;

        let mut rows = stmt.query(params![cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let asns_str: String = row.get(1)?;
            records.push(Pfx2asRecord {
                prefix: row.get(0)?,
                origin_asns: parse_array_string(&asns_str),
            });
        }

        Ok(records)
    }

    // =========================================================================
    // Cache Management
    // =========================================================================

    /// Get all cache metadata entries
    pub fn get_all_cache_meta(&self) -> Result<Vec<Pfx2asCacheMeta>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT id, data_source, loaded_at::TEXT, record_count
             FROM pfx2as_cache_meta
             ORDER BY loaded_at DESC",
        )?;

        let mut rows = stmt.query([])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let loaded_at_str: String = row.get(2)?;
            let loaded_at = DateTime::parse_from_rfc3339(&loaded_at_str)
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(&loaded_at_str, "%Y-%m-%d %H:%M:%S")
                        .map(|dt| dt.and_utc().fixed_offset())
                })
                .unwrap_or_else(|_| Utc::now().fixed_offset());

            records.push(Pfx2asCacheMeta {
                id: row.get(0)?,
                data_source: row.get(1)?,
                loaded_at: loaded_at.with_timezone(&Utc),
                record_count: row.get(3)?,
            });
        }

        Ok(records)
    }

    /// Clear all Pfx2as cache data
    pub fn clear_all(&self) -> Result<()> {
        self.conn.execute("DELETE FROM pfx2as")?;
        self.conn.execute("DELETE FROM pfx2as_cache_meta")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::core::{DuckDbConn, DuckDbSchemaManager};

    fn setup_test_db() -> DuckDbConn {
        let conn = DuckDbConn::open_in_memory().unwrap();
        let schema = DuckDbSchemaManager::new(&conn);
        schema.initialize().unwrap();
        conn
    }

    #[test]
    fn test_store_and_lookup_exact() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "192.168.0.0/16".to_string(),
                origin_asns: vec![65001, 65002], // MOAS
            },
        ];

        let cache_id = repo.store("test", &records).unwrap();
        assert!(cache_id > 0);

        // Exact lookup - found
        let result = repo.lookup_exact("10.0.0.0/8", "test").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().origin_asns, vec![65000]);

        // Exact lookup - not found
        let result = repo.lookup_exact("10.1.0.0/16", "test").unwrap();
        assert!(result.is_none());

        // MOAS prefix
        let result = repo.lookup_exact("192.168.0.0/16", "test").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().origin_asns, vec![65001, 65002]);
    }

    #[test]
    fn test_longest_prefix_match() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "10.1.0.0/16".to_string(),
                origin_asns: vec![65001],
            },
            Pfx2asRecord {
                prefix: "10.1.1.0/24".to_string(),
                origin_asns: vec![65002],
            },
        ];

        repo.store("test", &records).unwrap();

        // Longest match for /24 should return /24
        let result = repo.lookup_longest_match("10.1.1.0/24", "test").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().prefix, "10.1.1.0/24");

        // Longest match for /25 (not in DB) should return /24
        let result = repo.lookup_longest_match("10.1.1.0/25", "test").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().prefix, "10.1.1.0/24");

        // Longest match for 10.2.0.0/24 (not covered by /16) should return /8
        let result = repo.lookup_longest_match("10.2.0.0/24", "test").unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().prefix, "10.0.0.0/8");

        // No match for completely different prefix
        let result = repo.lookup_longest_match("192.168.0.0/24", "test").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_lookup_covering() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "10.1.0.0/16".to_string(),
                origin_asns: vec![65001],
            },
            Pfx2asRecord {
                prefix: "10.1.1.0/24".to_string(),
                origin_asns: vec![65002],
            },
        ];

        repo.store("test", &records).unwrap();

        // All covering prefixes for /24
        let results = repo.lookup_covering("10.1.1.0/24", "test").unwrap();
        assert_eq!(results.len(), 3);
        // Should be ordered by mask length descending (most specific first)
        assert_eq!(results[0].prefix, "10.1.1.0/24");
        assert_eq!(results[1].prefix, "10.1.0.0/16");
        assert_eq!(results[2].prefix, "10.0.0.0/8");
    }

    #[test]
    fn test_lookup_covered() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "10.1.0.0/16".to_string(),
                origin_asns: vec![65001],
            },
            Pfx2asRecord {
                prefix: "10.1.1.0/24".to_string(),
                origin_asns: vec![65002],
            },
            Pfx2asRecord {
                prefix: "192.168.0.0/16".to_string(),
                origin_asns: vec![65003],
            },
        ];

        repo.store("test", &records).unwrap();

        // All sub-prefixes of /8
        let results = repo.lookup_covered("10.0.0.0/8", "test").unwrap();
        assert_eq!(results.len(), 3); // /8, /16, and /24
    }

    #[test]
    fn test_lookup_by_origin() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "192.168.0.0/16".to_string(),
                origin_asns: vec![65000, 65001], // MOAS
            },
            Pfx2asRecord {
                prefix: "172.16.0.0/12".to_string(),
                origin_asns: vec![65002],
            },
        ];

        repo.store("test", &records).unwrap();

        // Find all prefixes for ASN 65000
        let results = repo.lookup_by_origin(65000, "test").unwrap();
        assert_eq!(results.len(), 2);

        // Find prefixes for ASN 65001 (only in MOAS)
        let results = repo.lookup_by_origin(65001, "test").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_cache_freshness() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        // Cache is not fresh when empty
        assert!(!repo.is_cache_fresh("test", DEFAULT_PFX2AS_TTL));

        // Store some data
        let records = vec![Pfx2asRecord {
            prefix: "10.0.0.0/8".to_string(),
            origin_asns: vec![65000],
        }];
        repo.store("test", &records).unwrap();

        // Cache should be fresh now
        assert!(repo.is_cache_fresh("test", DEFAULT_PFX2AS_TTL));

        // Cache should not be fresh with 0 TTL
        assert!(!repo.is_cache_fresh("test", Duration::from_secs(0)));
    }

    #[test]
    fn test_cache_metadata() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "192.168.0.0/16".to_string(),
                origin_asns: vec![65001],
            },
        ];

        repo.store("bgpkit", &records).unwrap();

        let meta = repo.get_cache_meta("bgpkit").unwrap();
        assert!(meta.is_some());
        let meta = meta.unwrap();
        assert_eq!(meta.data_source, "bgpkit");
        assert_eq!(meta.record_count, 2);
    }

    #[test]
    fn test_clear_cache() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![Pfx2asRecord {
            prefix: "10.0.0.0/8".to_string(),
            origin_asns: vec![65000],
        }];
        repo.store("test", &records).unwrap();

        assert_eq!(repo.count(None).unwrap(), 1);

        repo.clear("test").unwrap();
        assert_eq!(repo.count(None).unwrap(), 0);
    }

    #[test]
    fn test_get_all() {
        let conn = setup_test_db();
        let repo = Pfx2asCacheRepository::new(&conn);

        let records = vec![
            Pfx2asRecord {
                prefix: "10.0.0.0/8".to_string(),
                origin_asns: vec![65000],
            },
            Pfx2asRecord {
                prefix: "192.168.0.0/16".to_string(),
                origin_asns: vec![65001],
            },
        ];

        repo.store("test", &records).unwrap();

        let all = repo.get_all("test").unwrap();
        assert_eq!(all.len(), 2);
    }
}
