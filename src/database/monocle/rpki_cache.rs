//! RPKI cache repository for the DuckDB database
//!
//! This module provides caching for RPKI data (ROAs and ASPAs) to avoid
//! reloading from external sources on every command. The cache includes
//! freshness tracking with configurable TTL.

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate, Utc};
use duckdb::params;
use std::time::Duration;
use tracing::info;

use crate::database::core::DuckDbConn;

/// Default TTL for "current" RPKI data (1 hour)
pub const DEFAULT_RPKI_CURRENT_TTL: Duration = Duration::from_secs(60 * 60);

/// ROA (Route Origin Authorization) record
#[derive(Debug, Clone)]
pub struct RoaRecord {
    pub prefix: String,
    pub max_length: u32,
    pub origin_asn: u32,
    pub ta: Option<String>,
}

/// ASPA (Autonomous System Provider Authorization) record
#[derive(Debug, Clone)]
pub struct AspaRecord {
    pub customer_asn: u32,
    pub provider_asns: Vec<u32>,
}

/// Cache metadata for RPKI data
#[derive(Debug, Clone)]
pub struct RpkiCacheMeta {
    pub id: i64,
    pub data_type: String,
    pub data_source: String,
    pub data_date: Option<NaiveDate>,
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

/// Repository for RPKI cache operations (ROAs and ASPAs)
pub struct RpkiCacheRepository<'a> {
    conn: &'a DuckDbConn,
}

impl<'a> RpkiCacheRepository<'a> {
    /// Create a new RPKI cache repository
    pub fn new(conn: &'a DuckDbConn) -> Self {
        Self { conn }
    }

    // =========================================================================
    // Cache Metadata Operations
    // =========================================================================

    /// Get cache metadata for a specific data type, source, and date
    pub fn get_cache_meta(
        &self,
        data_type: &str,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Option<RpkiCacheMeta>> {
        let query = if data_date.is_some() {
            "SELECT id, data_type, data_source, data_date::TEXT, loaded_at::TEXT, record_count
             FROM rpki_cache_meta
             WHERE data_type = ? AND data_source = ? AND data_date = ?"
        } else {
            "SELECT id, data_type, data_source, data_date::TEXT, loaded_at::TEXT, record_count
             FROM rpki_cache_meta
             WHERE data_type = ? AND data_source = ? AND data_date IS NULL"
        };

        let mut stmt = self.conn.conn.prepare(query)?;

        let result = if let Some(date) = data_date {
            stmt.query_row(params![data_type, data_source, date.to_string()], |row| {
                let loaded_at_str: String = row.get(4)?;
                let loaded_at = DateTime::parse_from_rfc3339(&loaded_at_str)
                    .or_else(|_| {
                        // Try parsing as "YYYY-MM-DD HH:MM:SS" format
                        chrono::NaiveDateTime::parse_from_str(&loaded_at_str, "%Y-%m-%d %H:%M:%S")
                            .map(|dt| dt.and_utc().fixed_offset())
                    })
                    .unwrap_or_else(|_| Utc::now().fixed_offset());

                Ok(RpkiCacheMeta {
                    id: row.get(0)?,
                    data_type: row.get(1)?,
                    data_source: row.get(2)?,
                    data_date: row
                        .get::<_, Option<String>>(3)?
                        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                    loaded_at: loaded_at.with_timezone(&Utc),
                    record_count: row.get(5)?,
                })
            })
        } else {
            stmt.query_row(params![data_type, data_source], |row| {
                let loaded_at_str: String = row.get(4)?;
                let loaded_at = DateTime::parse_from_rfc3339(&loaded_at_str)
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(&loaded_at_str, "%Y-%m-%d %H:%M:%S")
                            .map(|dt| dt.and_utc().fixed_offset())
                    })
                    .unwrap_or_else(|_| Utc::now().fixed_offset());

                Ok(RpkiCacheMeta {
                    id: row.get(0)?,
                    data_type: row.get(1)?,
                    data_source: row.get(2)?,
                    data_date: row
                        .get::<_, Option<String>>(3)?
                        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                    loaded_at: loaded_at.with_timezone(&Utc),
                    record_count: row.get(5)?,
                })
            })
        };

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get RPKI cache metadata: {}", e)),
        }
    }

    /// Check if cached data is fresh (within TTL)
    pub fn is_cache_fresh(
        &self,
        data_type: &str,
        data_source: &str,
        data_date: Option<NaiveDate>,
        ttl: Duration,
    ) -> bool {
        // Historical data never expires
        if data_date.is_some() {
            return self
                .get_cache_meta(data_type, data_source, data_date)
                .ok()
                .flatten()
                .is_some();
        }

        // Check if "current" data is within TTL
        if let Ok(Some(meta)) = self.get_cache_meta(data_type, data_source, None) {
            let age = Utc::now().signed_duration_since(meta.loaded_at);
            return age.num_seconds() < ttl.as_secs() as i64;
        }

        false
    }

    /// Get the latest cache ID for a data type and source
    fn get_latest_cache_id(
        &self,
        data_type: &str,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Option<i64>> {
        if let Some(meta) = self.get_cache_meta(data_type, data_source, data_date)? {
            Ok(Some(meta.id))
        } else {
            Ok(None)
        }
    }

    /// Create a new cache entry and return its ID
    fn create_cache_entry(
        &self,
        data_type: &str,
        data_source: &str,
        data_date: Option<NaiveDate>,
        record_count: u64,
    ) -> Result<i64> {
        // Get next ID
        let next_id: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(id), 0) + 1 FROM rpki_cache_meta",
                |row| row.get(0),
            )
            .unwrap_or(1);

        let query = if data_date.is_some() {
            "INSERT INTO rpki_cache_meta (id, data_type, data_source, data_date, loaded_at, record_count)
             VALUES (?, ?, ?, ?, current_timestamp, ?)"
        } else {
            "INSERT INTO rpki_cache_meta (id, data_type, data_source, data_date, loaded_at, record_count)
             VALUES (?, ?, ?, NULL, current_timestamp, ?)"
        };

        if let Some(date) = data_date {
            self.conn.conn.execute(
                query,
                params![
                    next_id,
                    data_type,
                    data_source,
                    date.to_string(),
                    record_count
                ],
            )?;
        } else {
            self.conn.conn.execute(
                query,
                params![next_id, data_type, data_source, record_count],
            )?;
        }

        Ok(next_id)
    }

    // =========================================================================
    // ROA Operations
    // =========================================================================

    /// Get ROA count for a specific cache
    pub fn roa_count(&self, cache_id: Option<i64>) -> Result<u64> {
        let query = match cache_id {
            Some(id) => format!("SELECT COUNT(*) FROM rpki_roas WHERE cache_id = {}", id),
            None => "SELECT COUNT(*) FROM rpki_roas".to_string(),
        };
        self.conn.query_row(&query, |row| row.get(0))
    }

    /// Store ROAs in the cache
    pub fn store_roas(
        &self,
        data_source: &str,
        data_date: Option<NaiveDate>,
        roas: &[RoaRecord],
    ) -> Result<i64> {
        info!(
            "Storing {} ROAs from {} (date: {:?})",
            roas.len(),
            data_source,
            data_date
        );

        // Clear existing cache for this source/date
        self.clear_roas(data_source, data_date)?;

        // Create new cache entry
        let cache_id =
            self.create_cache_entry("roas", data_source, data_date, roas.len() as u64)?;

        // Use transaction for bulk insert
        self.conn.transaction()?;

        {
            let mut stmt = self.conn.conn.prepare(
                "INSERT INTO rpki_roas (prefix, max_length, origin_asn, ta, cache_id)
                 VALUES (?::INET, ?, ?, ?, ?)",
            )?;

            for roa in roas {
                stmt.execute(params![
                    &roa.prefix,
                    roa.max_length,
                    roa.origin_asn,
                    &roa.ta,
                    cache_id,
                ])?;
            }
        }

        self.conn.commit()?;

        info!("Stored {} ROAs with cache_id {}", roas.len(), cache_id);
        Ok(cache_id)
    }

    /// Clear ROAs for a specific source and date
    pub fn clear_roas(&self, data_source: &str, data_date: Option<NaiveDate>) -> Result<()> {
        if let Some(cache_id) = self.get_latest_cache_id("roas", data_source, data_date)? {
            self.conn.execute(&format!(
                "DELETE FROM rpki_roas WHERE cache_id = {}",
                cache_id
            ))?;
            self.conn.execute(&format!(
                "DELETE FROM rpki_cache_meta WHERE id = {}",
                cache_id
            ))?;
        }
        Ok(())
    }

    /// Query ROAs by origin ASN
    pub fn query_roas_by_origin(
        &self,
        origin_asn: u32,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Vec<RoaRecord>> {
        let cache_id = match self.get_latest_cache_id("roas", data_source, data_date)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, max_length, origin_asn, ta
             FROM rpki_roas
             WHERE origin_asn = ? AND cache_id = ?",
        )?;

        let mut rows = stmt.query(params![origin_asn, cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            records.push(RoaRecord {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            });
        }

        Ok(records)
    }

    /// Query ROAs by prefix (exact match)
    pub fn query_roas_by_prefix(
        &self,
        prefix: &str,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Vec<RoaRecord>> {
        let cache_id = match self.get_latest_cache_id("roas", data_source, data_date)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, max_length, origin_asn, ta
             FROM rpki_roas
             WHERE prefix = ?::INET AND cache_id = ?",
        )?;

        let mut rows = stmt.query(params![prefix, cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            records.push(RoaRecord {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            });
        }

        Ok(records)
    }

    /// Query ROAs that cover a given prefix (super-prefixes)
    pub fn query_roas_covering_prefix(
        &self,
        prefix: &str,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Vec<RoaRecord>> {
        let cache_id = match self.get_latest_cache_id("roas", data_source, data_date)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, max_length, origin_asn, ta
             FROM rpki_roas
             WHERE prefix >>= ?::INET AND cache_id = ?",
        )?;

        let mut rows = stmt.query(params![prefix, cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            records.push(RoaRecord {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            });
        }

        Ok(records)
    }

    /// Get all ROAs from cache
    pub fn get_all_roas(
        &self,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Vec<RoaRecord>> {
        let cache_id = match self.get_latest_cache_id("roas", data_source, data_date)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT prefix::TEXT, max_length, origin_asn, ta
             FROM rpki_roas
             WHERE cache_id = ?",
        )?;

        let mut rows = stmt.query(params![cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            records.push(RoaRecord {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            });
        }

        Ok(records)
    }

    // =========================================================================
    // ASPA Operations
    // =========================================================================

    /// Get ASPA count for a specific cache
    pub fn aspa_count(&self, cache_id: Option<i64>) -> Result<u64> {
        let query = match cache_id {
            Some(id) => format!("SELECT COUNT(*) FROM rpki_aspas WHERE cache_id = {}", id),
            None => "SELECT COUNT(*) FROM rpki_aspas".to_string(),
        };
        self.conn.query_row(&query, |row| row.get(0))
    }

    /// Store ASPAs in the cache
    pub fn store_aspas(
        &self,
        data_source: &str,
        data_date: Option<NaiveDate>,
        aspas: &[AspaRecord],
    ) -> Result<i64> {
        info!(
            "Storing {} ASPAs from {} (date: {:?})",
            aspas.len(),
            data_source,
            data_date
        );

        // Clear existing cache for this source/date
        self.clear_aspas(data_source, data_date)?;

        // Create new cache entry
        let cache_id =
            self.create_cache_entry("aspas", data_source, data_date, aspas.len() as u64)?;

        // Use transaction for bulk insert
        self.conn.transaction()?;

        {
            let mut stmt = self.conn.conn.prepare(
                "INSERT INTO rpki_aspas (customer_asn, provider_asns, cache_id)
                 VALUES (?, ?, ?)",
            )?;

            for aspa in aspas {
                // Convert provider_asns to DuckDB array format
                let providers_str = format!(
                    "[{}]",
                    aspa.provider_asns
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );

                stmt.execute(params![aspa.customer_asn, providers_str, cache_id,])?;
            }
        }

        self.conn.commit()?;

        info!("Stored {} ASPAs with cache_id {}", aspas.len(), cache_id);
        Ok(cache_id)
    }

    /// Clear ASPAs for a specific source and date
    pub fn clear_aspas(&self, data_source: &str, data_date: Option<NaiveDate>) -> Result<()> {
        if let Some(cache_id) = self.get_latest_cache_id("aspas", data_source, data_date)? {
            self.conn.execute(&format!(
                "DELETE FROM rpki_aspas WHERE cache_id = {}",
                cache_id
            ))?;
            self.conn.execute(&format!(
                "DELETE FROM rpki_cache_meta WHERE id = {}",
                cache_id
            ))?;
        }
        Ok(())
    }

    /// Query ASPA by customer ASN
    pub fn query_aspa_by_customer(
        &self,
        customer_asn: u32,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Option<AspaRecord>> {
        let cache_id = match self.get_latest_cache_id("aspas", data_source, data_date)? {
            Some(id) => id,
            None => return Ok(None),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT customer_asn, provider_asns::TEXT
             FROM rpki_aspas
             WHERE customer_asn = ? AND cache_id = ?",
        )?;

        let result = stmt.query_row(params![customer_asn, cache_id], |row| {
            let customer: u32 = row.get(0)?;
            let providers_str: String = row.get(1)?;
            let providers = parse_array_string(&providers_str);
            Ok(AspaRecord {
                customer_asn: customer,
                provider_asns: providers,
            })
        });

        match result {
            Ok(record) => Ok(Some(record)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to query ASPA: {}", e)),
        }
    }

    /// Query ASPAs by provider ASN (find all ASPAs where this ASN is a provider)
    pub fn query_aspas_by_provider(
        &self,
        provider_asn: u32,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Vec<AspaRecord>> {
        let cache_id = match self.get_latest_cache_id("aspas", data_source, data_date)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT customer_asn, provider_asns::TEXT
             FROM rpki_aspas
             WHERE list_contains(provider_asns, ?) AND cache_id = ?",
        )?;

        let mut rows = stmt.query(params![provider_asn, cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let providers_str: String = row.get(1)?;
            records.push(AspaRecord {
                customer_asn: row.get(0)?,
                provider_asns: parse_array_string(&providers_str),
            });
        }

        Ok(records)
    }

    /// Get all ASPAs from cache
    pub fn get_all_aspas(
        &self,
        data_source: &str,
        data_date: Option<NaiveDate>,
    ) -> Result<Vec<AspaRecord>> {
        let cache_id = match self.get_latest_cache_id("aspas", data_source, data_date)? {
            Some(id) => id,
            None => return Ok(Vec::new()),
        };

        let mut stmt = self.conn.conn.prepare(
            "SELECT customer_asn, provider_asns::TEXT
             FROM rpki_aspas
             WHERE cache_id = ?",
        )?;

        let mut rows = stmt.query(params![cache_id])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let providers_str: String = row.get(1)?;
            records.push(AspaRecord {
                customer_asn: row.get(0)?,
                provider_asns: parse_array_string(&providers_str),
            });
        }

        Ok(records)
    }

    // =========================================================================
    // Cache Management
    // =========================================================================

    /// Get all cache metadata entries
    pub fn get_all_cache_meta(&self) -> Result<Vec<RpkiCacheMeta>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT id, data_type, data_source, data_date::TEXT, loaded_at::TEXT, record_count
             FROM rpki_cache_meta
             ORDER BY loaded_at DESC",
        )?;

        let mut rows = stmt.query([])?;
        let mut records = Vec::new();

        while let Some(row) = rows.next()? {
            let loaded_at_str: String = row.get(4)?;
            let loaded_at = DateTime::parse_from_rfc3339(&loaded_at_str)
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(&loaded_at_str, "%Y-%m-%d %H:%M:%S")
                        .map(|dt| dt.and_utc().fixed_offset())
                })
                .unwrap_or_else(|_| Utc::now().fixed_offset());

            records.push(RpkiCacheMeta {
                id: row.get(0)?,
                data_type: row.get(1)?,
                data_source: row.get(2)?,
                data_date: row
                    .get::<_, Option<String>>(3)?
                    .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                loaded_at: loaded_at.with_timezone(&Utc),
                record_count: row.get(5)?,
            });
        }

        Ok(records)
    }

    /// Clear all RPKI cache data
    pub fn clear_all(&self) -> Result<()> {
        self.conn.execute("DELETE FROM rpki_roas")?;
        self.conn.execute("DELETE FROM rpki_aspas")?;
        self.conn
            .execute("DELETE FROM rpki_cache_meta WHERE data_type IN ('roas', 'aspas')")?;
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
    fn test_store_and_query_roas() {
        let conn = setup_test_db();
        let repo = RpkiCacheRepository::new(&conn);

        let roas = vec![
            RoaRecord {
                prefix: "10.0.0.0/8".to_string(),
                max_length: 24,
                origin_asn: 65000,
                ta: Some("RIPE".to_string()),
            },
            RoaRecord {
                prefix: "192.168.0.0/16".to_string(),
                max_length: 24,
                origin_asn: 65001,
                ta: Some("ARIN".to_string()),
            },
            RoaRecord {
                prefix: "10.1.0.0/16".to_string(),
                max_length: 24,
                origin_asn: 65000,
                ta: Some("RIPE".to_string()),
            },
        ];

        let cache_id = repo.store_roas("test", None, &roas).unwrap();
        assert!(cache_id > 0);

        // Query by origin ASN
        let results = repo.query_roas_by_origin(65000, "test", None).unwrap();
        assert_eq!(results.len(), 2);

        // Query by prefix
        let results = repo
            .query_roas_by_prefix("10.0.0.0/8", "test", None)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].origin_asn, 65000);

        // Query covering prefix
        let results = repo
            .query_roas_covering_prefix("10.1.1.0/24", "test", None)
            .unwrap();
        assert_eq!(results.len(), 2); // Both /8 and /16 cover /24

        // Get all ROAs
        let all_roas = repo.get_all_roas("test", None).unwrap();
        assert_eq!(all_roas.len(), 3);
    }

    #[test]
    fn test_store_and_query_aspas() {
        let conn = setup_test_db();
        let repo = RpkiCacheRepository::new(&conn);

        let aspas = vec![
            AspaRecord {
                customer_asn: 65000,
                provider_asns: vec![65001, 65002, 65003],
            },
            AspaRecord {
                customer_asn: 65010,
                provider_asns: vec![65001, 65020],
            },
        ];

        let cache_id = repo.store_aspas("test", None, &aspas).unwrap();
        assert!(cache_id > 0);

        // Query by customer ASN
        let result = repo.query_aspa_by_customer(65000, "test", None).unwrap();
        assert!(result.is_some());
        let aspa = result.unwrap();
        assert_eq!(aspa.provider_asns.len(), 3);

        // Query by provider ASN
        let results = repo.query_aspas_by_provider(65001, "test", None).unwrap();
        assert_eq!(results.len(), 2); // Both customers have 65001 as provider

        // Get all ASPAs
        let all_aspas = repo.get_all_aspas("test", None).unwrap();
        assert_eq!(all_aspas.len(), 2);
    }

    #[test]
    fn test_cache_freshness() {
        let conn = setup_test_db();
        let repo = RpkiCacheRepository::new(&conn);

        // Cache is not fresh when empty
        assert!(!repo.is_cache_fresh("roas", "test", None, DEFAULT_RPKI_CURRENT_TTL));

        // Store some data
        let roas = vec![RoaRecord {
            prefix: "10.0.0.0/8".to_string(),
            max_length: 24,
            origin_asn: 65000,
            ta: None,
        }];
        repo.store_roas("test", None, &roas).unwrap();

        // Cache should be fresh now
        assert!(repo.is_cache_fresh("roas", "test", None, DEFAULT_RPKI_CURRENT_TTL));

        // Cache should not be fresh with 0 TTL
        assert!(!repo.is_cache_fresh("roas", "test", None, Duration::from_secs(0)));
    }

    #[test]
    fn test_historical_cache() {
        let conn = setup_test_db();
        let repo = RpkiCacheRepository::new(&conn);

        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

        let roas = vec![RoaRecord {
            prefix: "10.0.0.0/8".to_string(),
            max_length: 24,
            origin_asn: 65000,
            ta: None,
        }];

        repo.store_roas("test", Some(date), &roas).unwrap();

        // Historical data is always fresh
        assert!(repo.is_cache_fresh("roas", "test", Some(date), Duration::from_secs(0)));

        // Query with date
        let results = repo
            .query_roas_by_origin(65000, "test", Some(date))
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_cache_metadata() {
        let conn = setup_test_db();
        let repo = RpkiCacheRepository::new(&conn);

        let roas = vec![RoaRecord {
            prefix: "10.0.0.0/8".to_string(),
            max_length: 24,
            origin_asn: 65000,
            ta: None,
        }];
        repo.store_roas("cloudflare", None, &roas).unwrap();

        let aspas = vec![AspaRecord {
            customer_asn: 65000,
            provider_asns: vec![65001],
        }];
        repo.store_aspas("cloudflare", None, &aspas).unwrap();

        let all_meta = repo.get_all_cache_meta().unwrap();
        assert_eq!(all_meta.len(), 2);
    }

    #[test]
    fn test_clear_cache() {
        let conn = setup_test_db();
        let repo = RpkiCacheRepository::new(&conn);

        let roas = vec![RoaRecord {
            prefix: "10.0.0.0/8".to_string(),
            max_length: 24,
            origin_asn: 65000,
            ta: None,
        }];
        repo.store_roas("test", None, &roas).unwrap();

        assert_eq!(repo.roa_count(None).unwrap(), 1);

        repo.clear_roas("test", None).unwrap();
        assert_eq!(repo.roa_count(None).unwrap(), 0);
    }
}
