//! Pfx2as repository for the shared database
//!
//! This module provides SQLite-based storage for prefix-to-ASN mappings,
//! with efficient prefix query support using blob-based IP range storage.
//!
//! # IP Address Storage
//!
//! IP prefixes are stored as two 16-byte columns (start and end addresses).
//! IPv4 addresses are converted to IPv6-mapped format (::ffff:x.x.x.x) for
//! uniform storage and comparison.
//!
//! # Query Modes
//!
//! - **Exact match**: Find prefixes that exactly match the query prefix
//! - **Longest prefix match**: Find the most specific prefix covering the query
//! - **Covering prefixes**: Find all prefixes that cover the query prefix
//! - **Covered prefixes**: Find all prefixes covered by the query prefix

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use ipnet::IpNet;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use tabled::Tabled;
use tracing::info;

/// Default TTL for Pfx2as cache (24 hours)
pub const DEFAULT_PFX2AS_CACHE_TTL: Duration = Duration::hours(24);

/// Pfx2as record for database storage
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct Pfx2asDbRecord {
    /// IP prefix string (e.g., "1.1.1.0/24")
    pub prefix: String,
    /// Origin ASN
    pub origin_asn: u32,
}

/// Pfx2as query result with match information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asQueryResult {
    /// The matched prefix
    pub prefix: String,
    /// Origin ASNs for this prefix
    pub origin_asns: Vec<u32>,
    /// Match type (exact, longest, covering, covered)
    pub match_type: String,
}

/// Pfx2as cache metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asCacheDbMetadata {
    /// When the cache was last updated
    pub updated_at: DateTime<Utc>,
    /// Data source URL or identifier
    pub source: String,
    /// Number of unique prefixes
    pub prefix_count: u64,
    /// Number of prefix-ASN pairs (total records)
    pub record_count: u64,
}

/// SQL schema definitions for Pfx2as tables
pub struct Pfx2asSchemaDefinitions;

impl Pfx2asSchemaDefinitions {
    /// SQL for creating the Pfx2as main table
    pub const PFX2AS_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS pfx2as (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            prefix_start BLOB NOT NULL,
            prefix_end BLOB NOT NULL,
            prefix_length INTEGER NOT NULL,
            origin_asn INTEGER NOT NULL,
            prefix_str TEXT NOT NULL
        );
    "#;

    /// SQL for creating the Pfx2as metadata table
    pub const PFX2AS_META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS pfx2as_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            updated_at INTEGER NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            prefix_count INTEGER NOT NULL DEFAULT 0,
            record_count INTEGER NOT NULL DEFAULT 0
        );
    "#;

    /// SQL for creating Pfx2as indexes
    pub const PFX2AS_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_pfx2as_prefix_range ON pfx2as(prefix_start, prefix_end)",
        "CREATE INDEX IF NOT EXISTS idx_pfx2as_origin_asn ON pfx2as(origin_asn)",
        "CREATE INDEX IF NOT EXISTS idx_pfx2as_prefix_length ON pfx2as(prefix_length)",
        "CREATE INDEX IF NOT EXISTS idx_pfx2as_prefix_str ON pfx2as(prefix_str)",
    ];
}

/// Repository for Pfx2as data operations
pub struct Pfx2asRepository<'a> {
    conn: &'a Connection,
}

impl<'a> Pfx2asRepository<'a> {
    /// Create a new Pfx2as repository
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Initialize the Pfx2as schema (create tables if not exist)
    pub fn initialize_schema(&self) -> Result<()> {
        self.conn
            .execute(Pfx2asSchemaDefinitions::PFX2AS_TABLE, [])
            .map_err(|e| anyhow!("Failed to create pfx2as table: {}", e))?;

        self.conn
            .execute(Pfx2asSchemaDefinitions::PFX2AS_META_TABLE, [])
            .map_err(|e| anyhow!("Failed to create pfx2as_meta table: {}", e))?;

        for index_sql in Pfx2asSchemaDefinitions::PFX2AS_INDEXES {
            self.conn
                .execute(index_sql, [])
                .map_err(|e| anyhow!("Failed to create Pfx2as index: {}", e))?;
        }

        Ok(())
    }

    /// Check if Pfx2as tables exist
    pub fn tables_exist(&self) -> bool {
        let exists: i32 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='pfx2as'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        exists > 0
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        if !self.tables_exist() {
            return true;
        }
        self.record_count().unwrap_or(0) == 0
    }

    /// Check if the cache needs refresh based on TTL
    pub fn needs_refresh(&self, ttl: Duration) -> bool {
        if !self.tables_exist() || self.is_empty() {
            return true;
        }

        match self.get_metadata() {
            Ok(Some(meta)) => {
                let age = Utc::now().signed_duration_since(meta.updated_at);
                age > ttl
            }
            _ => true,
        }
    }

    /// Get cache metadata
    pub fn get_metadata(&self) -> Result<Option<Pfx2asCacheDbMetadata>> {
        if !self.tables_exist() {
            return Ok(None);
        }

        let result = self.conn.query_row(
            "SELECT updated_at, source, prefix_count, record_count FROM pfx2as_meta WHERE id = 1",
            [],
            |row| {
                let timestamp: i64 = row.get(0)?;
                let updated_at = DateTime::from_timestamp(timestamp, 0)
                    .unwrap_or_else(|| DateTime::from_timestamp(0, 0).unwrap());
                Ok(Pfx2asCacheDbMetadata {
                    updated_at,
                    source: row.get(1)?,
                    prefix_count: row.get(2)?,
                    record_count: row.get(3)?,
                })
            },
        );

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get Pfx2as metadata: {}", e)),
        }
    }

    /// Clear all Pfx2as data
    pub fn clear(&self) -> Result<()> {
        if !self.tables_exist() {
            return Ok(());
        }

        self.conn
            .execute("DELETE FROM pfx2as", [])
            .map_err(|e| anyhow!("Failed to clear pfx2as table: {}", e))?;

        self.conn
            .execute("DELETE FROM pfx2as_meta", [])
            .map_err(|e| anyhow!("Failed to clear pfx2as_meta table: {}", e))?;

        info!("Cleared Pfx2as database");
        Ok(())
    }

    /// Store Pfx2as records
    pub fn store(&self, records: &[Pfx2asDbRecord], source: &str) -> Result<()> {
        // Ensure schema exists
        self.initialize_schema()?;

        // Clear existing data
        self.clear()?;

        // Optimize for batch insert performance
        self.conn
            .execute("PRAGMA synchronous = OFF", [])
            .map_err(|e| anyhow!("Failed to set synchronous mode: {}", e))?;
        self.conn
            .query_row("PRAGMA journal_mode = MEMORY", [], |_| Ok(()))
            .map_err(|e| anyhow!("Failed to set journal mode: {}", e))?;
        self.conn
            .execute("PRAGMA cache_size = -64000", [])
            .map_err(|e| anyhow!("Failed to set cache size: {}", e))?; // 64MB cache

        // Begin transaction for batch insert
        self.conn.execute("BEGIN TRANSACTION", [])?;

        // Insert records
        let mut stmt = self.conn.prepare(
            "INSERT INTO pfx2as (prefix_start, prefix_end, prefix_length, origin_asn, prefix_str)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        let mut inserted = 0usize;
        let mut unique_prefixes = HashSet::new();

        for record in records {
            if let Ok((start, end, prefix_len)) = parse_prefix_to_range(&record.prefix) {
                stmt.execute(params![
                    start.as_slice(),
                    end.as_slice(),
                    prefix_len,
                    record.origin_asn,
                    record.prefix,
                ])?;
                unique_prefixes.insert(record.prefix.clone());
                inserted += 1;
            }
        }

        // Update metadata
        let now = Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO pfx2as_meta (id, updated_at, source, prefix_count, record_count) VALUES (1, ?1, ?2, ?3, ?4)",
            params![now, source, unique_prefixes.len(), inserted],
        )?;

        self.conn.execute("COMMIT", [])?;

        // Restore default settings for safety
        self.conn
            .execute("PRAGMA synchronous = FULL", [])
            .map_err(|e| anyhow!("Failed to restore synchronous mode: {}", e))?;
        self.conn
            .query_row("PRAGMA journal_mode = DELETE", [], |_| Ok(()))
            .map_err(|e| anyhow!("Failed to restore journal mode: {}", e))?;

        info!(
            "Stored {} Pfx2as records ({} unique prefixes) in database",
            inserted,
            unique_prefixes.len()
        );

        Ok(())
    }

    /// Get all records (limited for safety)
    pub fn get_all(&self, limit: Option<usize>) -> Result<Vec<Pfx2asDbRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let limit_clause = match limit {
            Some(n) => format!(" LIMIT {}", n),
            None => String::new(),
        };

        let mut stmt = self.conn.prepare(&format!(
            "SELECT prefix_str, origin_asn FROM pfx2as{}",
            limit_clause
        ))?;

        let rows = stmt.query_map([], |row| {
            Ok(Pfx2asDbRecord {
                prefix: row.get(0)?,
                origin_asn: row.get(1)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Get records by origin ASN
    pub fn get_by_asn(&self, asn: u32) -> Result<Vec<Pfx2asDbRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let mut stmt = self
            .conn
            .prepare("SELECT prefix_str, origin_asn FROM pfx2as WHERE origin_asn = ?1")?;

        let rows = stmt.query_map([asn], |row| {
            Ok(Pfx2asDbRecord {
                prefix: row.get(0)?,
                origin_asn: row.get(1)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Exact match: find the prefix that exactly matches the query
    pub fn lookup_exact(&self, prefix: &str) -> Result<Vec<u32>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT origin_asn FROM pfx2as WHERE prefix_str = ?1")?;

        let rows = stmt.query_map([prefix], |row| row.get::<_, u32>(0))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Longest prefix match: find the most specific prefix covering the query
    ///
    /// This finds all prefixes that cover the query address and returns
    /// the one with the longest prefix length.
    pub fn lookup_longest(&self, prefix: &str) -> Result<Pfx2asQueryResult> {
        if !self.tables_exist() {
            return Ok(Pfx2asQueryResult {
                prefix: prefix.to_string(),
                origin_asns: Vec::new(),
                match_type: "longest".to_string(),
            });
        }

        // Parse query prefix to get its start address
        let (query_start, _query_end, _query_len) = parse_prefix_to_range(prefix)?;

        // Find all covering prefixes and pick the longest one
        // A prefix covers our query if: prefix_start <= query_start AND prefix_end >= query_start
        let mut stmt = self.conn.prepare(
            "SELECT prefix_str, origin_asn, prefix_length FROM pfx2as
             WHERE prefix_start <= ?1 AND prefix_end >= ?1
             ORDER BY prefix_length DESC",
        )?;

        let rows = stmt.query_map([query_start.as_slice()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u8>(2)?,
            ))
        })?;

        let mut best_prefix: Option<String> = None;
        let mut best_length: u8 = 0;
        let mut asns: Vec<u32> = Vec::new();

        for row in rows {
            let (pfx, asn, len) = row?;
            if best_prefix.is_none() {
                best_prefix = Some(pfx.clone());
                best_length = len;
                asns.push(asn);
            } else if len == best_length && best_prefix.as_ref() == Some(&pfx) {
                // Same prefix, additional ASN
                if !asns.contains(&asn) {
                    asns.push(asn);
                }
            }
            // Stop once we move to shorter prefixes
            if len < best_length {
                break;
            }
        }

        Ok(Pfx2asQueryResult {
            prefix: best_prefix.unwrap_or_else(|| prefix.to_string()),
            origin_asns: asns,
            match_type: "longest".to_string(),
        })
    }

    /// Find all prefixes that cover the query prefix (supernets)
    pub fn lookup_covering(&self, prefix: &str) -> Result<Vec<Pfx2asQueryResult>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let (query_start, query_end, query_len) = parse_prefix_to_range(prefix)?;

        // A prefix covers our query if:
        // - prefix_start <= query_start AND prefix_end >= query_end
        // - prefix_length <= query_length (must be less specific or equal)
        let mut stmt = self.conn.prepare(
            "SELECT prefix_str, origin_asn FROM pfx2as
             WHERE prefix_start <= ?1 AND prefix_end >= ?2 AND prefix_length <= ?3
             ORDER BY prefix_length ASC",
        )?;

        let rows = stmt.query_map(
            params![query_start.as_slice(), query_end.as_slice(), query_len],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
        )?;

        // Group by prefix
        let mut prefix_map: std::collections::HashMap<String, Vec<u32>> =
            std::collections::HashMap::new();

        for row in rows {
            let (pfx, asn) = row?;
            prefix_map.entry(pfx).or_default().push(asn);
        }

        let results: Vec<Pfx2asQueryResult> = prefix_map
            .into_iter()
            .map(|(prefix, asns)| Pfx2asQueryResult {
                prefix,
                origin_asns: asns,
                match_type: "covering".to_string(),
            })
            .collect();

        Ok(results)
    }

    /// Find all prefixes covered by the query prefix (subnets)
    pub fn lookup_covered(&self, prefix: &str) -> Result<Vec<Pfx2asQueryResult>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let (query_start, query_end, query_len) = parse_prefix_to_range(prefix)?;

        // A prefix is covered by our query if:
        // - prefix_start >= query_start AND prefix_end <= query_end
        // - prefix_length >= query_length (must be more specific or equal)
        let mut stmt = self.conn.prepare(
            "SELECT prefix_str, origin_asn FROM pfx2as
             WHERE prefix_start >= ?1 AND prefix_end <= ?2 AND prefix_length >= ?3
             ORDER BY prefix_length ASC",
        )?;

        let rows = stmt.query_map(
            params![query_start.as_slice(), query_end.as_slice(), query_len],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
        )?;

        // Group by prefix
        let mut prefix_map: std::collections::HashMap<String, Vec<u32>> =
            std::collections::HashMap::new();

        for row in rows {
            let (pfx, asn) = row?;
            prefix_map.entry(pfx).or_default().push(asn);
        }

        let results: Vec<Pfx2asQueryResult> = prefix_map
            .into_iter()
            .map(|(prefix, asns)| Pfx2asQueryResult {
                prefix,
                origin_asns: asns,
                match_type: "covered".to_string(),
            })
            .collect();

        Ok(results)
    }

    /// Get the total number of records
    pub fn record_count(&self) -> Result<u64> {
        if !self.tables_exist() {
            return Ok(0);
        }

        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM pfx2as", [], |row| row.get(0))?;

        Ok(count as u64)
    }

    /// Get the number of unique prefixes
    pub fn prefix_count(&self) -> Result<u64> {
        if !self.tables_exist() {
            return Ok(0);
        }

        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(DISTINCT prefix_str) FROM pfx2as", [], |row| {
                    row.get(0)
                })?;

        Ok(count as u64)
    }
}

// =============================================================================
// Helper functions for IP address handling
// =============================================================================

/// Convert an IPv4 address to IPv6-mapped format
fn ipv4_to_ipv6_mapped(ipv4: Ipv4Addr) -> Ipv6Addr {
    ipv4.to_ipv6_mapped()
}

/// Convert an IP address to 16-byte representation
fn ip_to_bytes(ip: IpAddr) -> [u8; 16] {
    match ip {
        IpAddr::V4(v4) => ipv4_to_ipv6_mapped(v4).octets(),
        IpAddr::V6(v6) => v6.octets(),
    }
}

/// Parse a prefix string and return (start_bytes, end_bytes, prefix_length)
fn parse_prefix_to_range(prefix: &str) -> Result<([u8; 16], [u8; 16], u8)> {
    let net = IpNet::from_str(prefix).map_err(|e| anyhow!("Invalid prefix '{}': {}", prefix, e))?;

    let start = ip_to_bytes(net.network());
    let end = ip_to_bytes(net.broadcast());
    let prefix_len = net.prefix_len();

    Ok((start, end, prefix_len))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn
    }

    #[test]
    fn test_schema_initialization() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        assert!(repo.initialize_schema().is_ok());
        assert!(repo.tables_exist());
    }

    #[test]
    fn test_store_and_retrieve() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![
            Pfx2asDbRecord {
                prefix: "1.1.1.0/24".to_string(),
                origin_asn: 13335,
            },
            Pfx2asDbRecord {
                prefix: "8.8.8.0/24".to_string(),
                origin_asn: 15169,
            },
            Pfx2asDbRecord {
                prefix: "1.1.1.0/24".to_string(),
                origin_asn: 13336, // Same prefix, different ASN
            },
        ];

        repo.store(&records, "test").unwrap();

        assert_eq!(repo.record_count().unwrap(), 3);
        assert_eq!(repo.prefix_count().unwrap(), 2);

        // Test exact lookup
        let asns = repo.lookup_exact("1.1.1.0/24").unwrap();
        assert_eq!(asns.len(), 2);
        assert!(asns.contains(&13335));
        assert!(asns.contains(&13336));
    }

    #[test]
    fn test_lookup_longest() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![
            Pfx2asDbRecord {
                prefix: "1.0.0.0/8".to_string(),
                origin_asn: 1000,
            },
            Pfx2asDbRecord {
                prefix: "1.1.0.0/16".to_string(),
                origin_asn: 1100,
            },
            Pfx2asDbRecord {
                prefix: "1.1.1.0/24".to_string(),
                origin_asn: 13335,
            },
        ];

        repo.store(&records, "test").unwrap();

        // Query for 1.1.1.128/32 should match 1.1.1.0/24 (longest match)
        let result = repo.lookup_longest("1.1.1.128/32").unwrap();
        assert_eq!(result.prefix, "1.1.1.0/24");
        assert!(result.origin_asns.contains(&13335));

        // Query for 1.1.2.0/24 should match 1.1.0.0/16
        let result = repo.lookup_longest("1.1.2.0/24").unwrap();
        assert_eq!(result.prefix, "1.1.0.0/16");
        assert!(result.origin_asns.contains(&1100));

        // Query for 1.2.0.0/16 should match 1.0.0.0/8
        let result = repo.lookup_longest("1.2.0.0/16").unwrap();
        assert_eq!(result.prefix, "1.0.0.0/8");
        assert!(result.origin_asns.contains(&1000));
    }

    #[test]
    fn test_lookup_covering() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![
            Pfx2asDbRecord {
                prefix: "1.0.0.0/8".to_string(),
                origin_asn: 1000,
            },
            Pfx2asDbRecord {
                prefix: "1.1.0.0/16".to_string(),
                origin_asn: 1100,
            },
            Pfx2asDbRecord {
                prefix: "1.1.1.0/24".to_string(),
                origin_asn: 13335,
            },
        ];

        repo.store(&records, "test").unwrap();

        // Query for 1.1.1.0/24 should find all covering prefixes
        let results = repo.lookup_covering("1.1.1.0/24").unwrap();
        assert_eq!(results.len(), 3);

        let prefixes: Vec<&str> = results.iter().map(|r| r.prefix.as_str()).collect();
        assert!(prefixes.contains(&"1.0.0.0/8"));
        assert!(prefixes.contains(&"1.1.0.0/16"));
        assert!(prefixes.contains(&"1.1.1.0/24"));
    }

    #[test]
    fn test_lookup_covered() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![
            Pfx2asDbRecord {
                prefix: "1.0.0.0/8".to_string(),
                origin_asn: 1000,
            },
            Pfx2asDbRecord {
                prefix: "1.1.0.0/16".to_string(),
                origin_asn: 1100,
            },
            Pfx2asDbRecord {
                prefix: "1.1.1.0/24".to_string(),
                origin_asn: 13335,
            },
            Pfx2asDbRecord {
                prefix: "2.0.0.0/8".to_string(),
                origin_asn: 2000,
            },
        ];

        repo.store(&records, "test").unwrap();

        // Query for 1.0.0.0/8 should find all covered prefixes
        let results = repo.lookup_covered("1.0.0.0/8").unwrap();
        assert_eq!(results.len(), 3);

        let prefixes: Vec<&str> = results.iter().map(|r| r.prefix.as_str()).collect();
        assert!(prefixes.contains(&"1.0.0.0/8"));
        assert!(prefixes.contains(&"1.1.0.0/16"));
        assert!(prefixes.contains(&"1.1.1.0/24"));
        assert!(!prefixes.contains(&"2.0.0.0/8"));
    }

    #[test]
    fn test_metadata() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![
            Pfx2asDbRecord {
                prefix: "1.1.1.0/24".to_string(),
                origin_asn: 13335,
            },
            Pfx2asDbRecord {
                prefix: "8.8.8.0/24".to_string(),
                origin_asn: 15169,
            },
        ];

        repo.store(&records, "https://example.com/data.json")
            .unwrap();

        let meta = repo.get_metadata().unwrap().unwrap();
        assert_eq!(meta.source, "https://example.com/data.json");
        assert_eq!(meta.prefix_count, 2);
        assert_eq!(meta.record_count, 2);
    }

    #[test]
    fn test_needs_refresh() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        // Empty database should need refresh
        assert!(repo.needs_refresh(DEFAULT_PFX2AS_CACHE_TTL));

        let records = vec![Pfx2asDbRecord {
            prefix: "1.1.1.0/24".to_string(),
            origin_asn: 13335,
        }];

        repo.store(&records, "test").unwrap();

        // Just stored, should not need refresh
        assert!(!repo.needs_refresh(DEFAULT_PFX2AS_CACHE_TTL));

        // With 0 TTL, should need refresh
        assert!(repo.needs_refresh(Duration::zero()));
    }

    #[test]
    fn test_ipv6_prefix() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![
            Pfx2asDbRecord {
                prefix: "2001:db8::/32".to_string(),
                origin_asn: 65000,
            },
            Pfx2asDbRecord {
                prefix: "2001:db8:1::/48".to_string(),
                origin_asn: 65001,
            },
        ];

        repo.store(&records, "test").unwrap();

        // Exact match
        let asns = repo.lookup_exact("2001:db8::/32").unwrap();
        assert_eq!(asns, vec![65000]);

        // Longest match
        let result = repo.lookup_longest("2001:db8:1:1::/64").unwrap();
        assert_eq!(result.prefix, "2001:db8:1::/48");
        assert!(result.origin_asns.contains(&65001));
    }

    #[test]
    fn test_clear() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![Pfx2asDbRecord {
            prefix: "1.1.1.0/24".to_string(),
            origin_asn: 13335,
        }];

        repo.store(&records, "test").unwrap();
        assert!(!repo.is_empty());

        repo.clear().unwrap();
        assert!(repo.is_empty());
    }

    #[test]
    fn test_get_by_asn() {
        let conn = create_test_db();
        let repo = Pfx2asRepository::new(&conn);

        let records = vec![
            Pfx2asDbRecord {
                prefix: "1.1.1.0/24".to_string(),
                origin_asn: 13335,
            },
            Pfx2asDbRecord {
                prefix: "104.16.0.0/12".to_string(),
                origin_asn: 13335,
            },
            Pfx2asDbRecord {
                prefix: "8.8.8.0/24".to_string(),
                origin_asn: 15169,
            },
        ];

        repo.store(&records, "test").unwrap();

        let cloudflare_prefixes = repo.get_by_asn(13335).unwrap();
        assert_eq!(cloudflare_prefixes.len(), 2);

        let google_prefixes = repo.get_by_asn(15169).unwrap();
        assert_eq!(google_prefixes.len(), 1);
        assert_eq!(google_prefixes[0].prefix, "8.8.8.0/24");
    }
}
