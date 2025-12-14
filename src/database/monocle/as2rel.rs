//! AS2Rel repository for the shared database
//!
//! This module provides data access operations for AS-level relationships.
//! Data is sourced from BGPKIT's AS2Rel dataset.

use anyhow::{anyhow, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

/// Default URL for AS2Rel data
pub const BGPKIT_AS2REL_URL: &str = "https://data.bgpkit.com/as2rel/as2rel-latest.json.bz2";

/// Seven days in seconds (for staleness check)
const SEVEN_DAYS_SECS: u64 = 7 * 24 * 60 * 60;

/// Repository for AS2Rel data operations
///
/// Provides methods for querying and updating AS-level relationship data
/// in the shared database.
pub struct As2relRepository<'a> {
    conn: &'a Connection,
}

/// An entry in the AS2Rel dataset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct As2relEntry {
    pub asn1: u32,
    pub asn2: u32,
    pub paths_count: u32,
    pub peers_count: u32,
    pub rel: i8,
}

/// A record from the AS2Rel database
#[derive(Debug, Clone)]
pub struct As2relRecord {
    pub asn1: u32,
    pub asn2: u32,
    pub paths_count: u32,
    pub peers_count: u32,
    pub rel: i8,
}

/// Aggregated relationship between two ASNs
#[derive(Debug, Clone)]
pub struct AggregatedRelationship {
    pub asn1: u32,
    pub asn2: u32,
    pub asn2_name: Option<String>,
    pub connected_count: u32,
    pub as1_upstream_count: u32,
    pub as2_upstream_count: u32,
}

/// Metadata about the AS2Rel data
#[derive(Debug, Clone)]
pub struct As2relMeta {
    pub file_url: String,
    pub last_updated: u64,
    pub max_peers_count: u32,
}

impl<'a> As2relRepository<'a> {
    /// Create a new AS2Rel repository
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Check if the AS2Rel data is empty
    pub fn is_empty(&self) -> bool {
        let count: u32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM as2rel", [], |row| row.get(0))
            .unwrap_or(0);
        count == 0
    }

    /// Get the count of relationship entries
    pub fn count(&self) -> Result<u64> {
        let count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM as2rel", [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to get relationship count: {}", e))?;
        Ok(count)
    }

    /// Check if the data should be updated (empty or older than 7 days)
    pub fn should_update(&self) -> bool {
        if self.is_empty() {
            return true;
        }

        // Check if data is older than 7 days
        match self.get_meta() {
            Ok(Some(meta)) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                now.saturating_sub(meta.last_updated) > SEVEN_DAYS_SECS
            }
            _ => true,
        }
    }

    /// Get metadata about the AS2Rel data
    pub fn get_meta(&self) -> Result<Option<As2relMeta>> {
        let result = self.conn.query_row(
            "SELECT file_url, last_updated, max_peers_count FROM as2rel_meta WHERE id = 1",
            [],
            |row| {
                Ok(As2relMeta {
                    file_url: row.get(0)?,
                    last_updated: row.get(1)?,
                    max_peers_count: row.get(2)?,
                })
            },
        );

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get AS2Rel metadata: {}", e)),
        }
    }

    /// Get the maximum peers count (used for percentage calculation)
    pub fn get_max_peers_count(&self) -> u32 {
        self.get_meta()
            .ok()
            .flatten()
            .map(|m| m.max_peers_count)
            .unwrap_or(0)
    }

    /// Search for all relationships of a single ASN
    pub fn search_asn(&self, asn: u32) -> Result<Vec<As2relRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT asn1, asn2, paths_count, peers_count, rel
             FROM as2rel
             WHERE asn1 = ?1 OR asn2 = ?1",
        )?;

        let rows = stmt
            .query_map([asn], |row| {
                Ok(As2relRecord {
                    asn1: row.get(0)?,
                    asn2: row.get(1)?,
                    paths_count: row.get(2)?,
                    peers_count: row.get(3)?,
                    rel: row.get(4)?,
                })
            })
            .map_err(|e| anyhow!("Failed to search ASN: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Search for relationship between two specific ASNs
    pub fn search_pair(&self, asn1: u32, asn2: u32) -> Result<Vec<As2relRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT asn1, asn2, paths_count, peers_count, rel
             FROM as2rel
             WHERE (asn1 = ?1 AND asn2 = ?2) OR (asn1 = ?2 AND asn2 = ?1)",
        )?;

        let rows = stmt
            .query_map([asn1, asn2], |row| {
                Ok(As2relRecord {
                    asn1: row.get(0)?,
                    asn2: row.get(1)?,
                    paths_count: row.get(2)?,
                    peers_count: row.get(3)?,
                    rel: row.get(4)?,
                })
            })
            .map_err(|e| anyhow!("Failed to search pair: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Search for relationships of an ASN with organization names from as2org
    /// Uses SQL aggregation and JOIN for efficiency
    pub fn search_asn_with_names(&self, asn: u32) -> Result<Vec<AggregatedRelationship>> {
        // Use SQL to aggregate and join with as2org in one query
        // The query normalizes the perspective so asn1 is always the query ASN
        let query = r#"
            SELECT
                :asn as asn1,
                CASE WHEN r.asn1 = :asn THEN r.asn2 ELSE r.asn1 END as asn2,
                o.org_name as asn2_name,
                MAX(CASE WHEN r.rel = 0 THEN r.peers_count ELSE 0 END) as connected_count,
                SUM(CASE
                    WHEN r.asn1 = :asn AND r.rel = 1 THEN r.peers_count
                    WHEN r.asn2 = :asn AND r.rel = -1 THEN r.peers_count
                    ELSE 0
                END) as as1_upstream_count,
                SUM(CASE
                    WHEN r.asn1 = :asn AND r.rel = -1 THEN r.peers_count
                    WHEN r.asn2 = :asn AND r.rel = 1 THEN r.peers_count
                    ELSE 0
                END) as as2_upstream_count
            FROM as2rel r
            LEFT JOIN as2org_all o ON o.asn = CASE WHEN r.asn1 = :asn THEN r.asn2 ELSE r.asn1 END
            WHERE r.asn1 = :asn OR r.asn2 = :asn
            GROUP BY CASE WHEN r.asn1 = :asn THEN r.asn2 ELSE r.asn1 END
        "#;

        let mut stmt = self.conn.prepare(query)?;
        let rows = stmt
            .query_map(rusqlite::named_params! { ":asn": asn }, |row| {
                Ok(AggregatedRelationship {
                    asn1: row.get(0)?,
                    asn2: row.get(1)?,
                    asn2_name: row.get(2)?,
                    connected_count: row.get(3)?,
                    as1_upstream_count: row.get(4)?,
                    as2_upstream_count: row.get(5)?,
                })
            })
            .map_err(|e| anyhow!("Failed to search ASN with names: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Search for relationship between two ASNs with organization names
    /// Uses SQL aggregation and JOIN for efficiency
    pub fn search_pair_with_names(
        &self,
        asn1: u32,
        asn2: u32,
    ) -> Result<Vec<AggregatedRelationship>> {
        // Use SQL to aggregate and join with as2org in one query
        // Perspective is from asn1's point of view
        let query = r#"
            SELECT
                :asn1 as asn1,
                :asn2 as asn2,
                o.org_name as asn2_name,
                MAX(CASE WHEN r.rel = 0 THEN r.peers_count ELSE 0 END) as connected_count,
                SUM(CASE
                    WHEN r.asn1 = :asn1 AND r.rel = 1 THEN r.peers_count
                    WHEN r.asn2 = :asn1 AND r.rel = -1 THEN r.peers_count
                    ELSE 0
                END) as as1_upstream_count,
                SUM(CASE
                    WHEN r.asn1 = :asn1 AND r.rel = -1 THEN r.peers_count
                    WHEN r.asn2 = :asn1 AND r.rel = 1 THEN r.peers_count
                    ELSE 0
                END) as as2_upstream_count
            FROM as2rel r
            LEFT JOIN as2org_all o ON o.asn = :asn2
            WHERE (r.asn1 = :asn1 AND r.asn2 = :asn2) OR (r.asn1 = :asn2 AND r.asn2 = :asn1)
        "#;

        let mut stmt = self.conn.prepare(query)?;
        let rows = stmt
            .query_map(
                rusqlite::named_params! { ":asn1": asn1, ":asn2": asn2 },
                |row| {
                    Ok(AggregatedRelationship {
                        asn1: row.get(0)?,
                        asn2: row.get(1)?,
                        asn2_name: row.get(2)?,
                        connected_count: row.get(3)?,
                        as1_upstream_count: row.get(4)?,
                        as2_upstream_count: row.get(5)?,
                    })
                },
            )
            .map_err(|e| anyhow!("Failed to search pair with names: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Clear all AS2Rel data
    pub fn clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM as2rel", [])
            .map_err(|e| anyhow!("Failed to clear as2rel: {}", e))?;
        self.conn
            .execute("DELETE FROM as2rel_meta", [])
            .map_err(|e| anyhow!("Failed to clear as2rel_meta: {}", e))?;
        Ok(())
    }

    /// Load AS2Rel data from the default URL
    pub fn load_from_url(&self) -> Result<usize> {
        self.load_from_path(BGPKIT_AS2REL_URL)
    }

    /// Load AS2Rel data from a custom path (file or URL)
    ///
    /// Uses optimized batch insert with:
    /// - Disabled synchronous writes for performance
    /// - Memory-based journal mode
    /// - Single transaction for all inserts
    pub fn load_from_path(&self, path: &str) -> Result<usize> {
        self.clear()?;

        info!("Loading AS2Rel data from {}...", path);

        // Load entries from the JSON file
        let entries: Vec<As2relEntry> = oneio::read_json_struct(path)
            .map_err(|e| anyhow!("Failed to read AS2Rel data from {}: {}", path, e))?;

        info!(
            "Loaded {} entries, inserting into database...",
            entries.len()
        );

        // Find max peers count for normalization
        let max_peers = entries.iter().map(|e| e.peers_count).max().unwrap_or(0);

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

        // Use a transaction for all inserts
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("Failed to begin transaction: {}", e))?;

        let entry_count = entries.len();

        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO as2rel (asn1, asn2, paths_count, peers_count, rel)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            for entry in &entries {
                stmt.execute(rusqlite::params![
                    entry.asn1,
                    entry.asn2,
                    entry.paths_count,
                    entry.peers_count,
                    entry.rel,
                ])?;
            }

            // Update metadata
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            tx.execute(
                "INSERT OR REPLACE INTO as2rel_meta (id, file_url, last_updated, max_peers_count)
                 VALUES (1, ?1, ?2, ?3)",
                rusqlite::params![path, now, max_peers],
            )?;
        }

        tx.commit()
            .map_err(|e| anyhow!("Failed to commit transaction: {}", e))?;

        // Restore default settings for safety
        self.conn
            .execute("PRAGMA synchronous = FULL", [])
            .map_err(|e| anyhow!("Failed to restore synchronous mode: {}", e))?;
        self.conn
            .query_row("PRAGMA journal_mode = DELETE", [], |_| Ok(()))
            .map_err(|e| anyhow!("Failed to restore journal mode: {}", e))?;

        info!("AS2Rel data loading finished: {} entries", entry_count);

        Ok(entry_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::core::{DatabaseConn, SchemaManager};

    fn setup_test_db() -> DatabaseConn {
        let db = DatabaseConn::open_in_memory().unwrap();
        let schema = SchemaManager::new(&db.conn);
        schema.initialize().unwrap();
        db
    }

    #[test]
    fn test_is_empty() {
        let db = setup_test_db();
        let repo = As2relRepository::new(&db.conn);
        assert!(repo.is_empty());
    }

    #[test]
    fn test_insert_and_search() {
        let db = setup_test_db();

        // Insert test data directly
        db.conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65001, 100, 10, 0)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65002, 200, 20, 1)",
                [],
            )
            .unwrap();

        let repo = As2relRepository::new(&db.conn);

        // Test is_empty
        assert!(!repo.is_empty());

        // Test count
        assert_eq!(repo.count().unwrap(), 2);

        // Test search by ASN
        let results = repo.search_asn(65000).unwrap();
        assert_eq!(results.len(), 2);

        // Test search by pair
        let results = repo.search_pair(65000, 65001).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].rel, 0); // peer relationship
    }

    #[test]
    fn test_meta() {
        let db = setup_test_db();

        // Insert meta data
        db.conn
            .execute(
                "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test.json', 1234567890, 100)",
                [],
            )
            .unwrap();

        let repo = As2relRepository::new(&db.conn);
        let meta = repo.get_meta().unwrap().unwrap();

        assert_eq!(meta.file_url, "test.json");
        assert_eq!(meta.last_updated, 1234567890);
        assert_eq!(meta.max_peers_count, 100);
    }

    #[test]
    fn test_clear() {
        let db = setup_test_db();

        db.conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65001, 100, 10, 0)",
                [],
            )
            .unwrap();

        let repo = As2relRepository::new(&db.conn);
        assert!(!repo.is_empty());

        repo.clear().unwrap();
        assert!(repo.is_empty());
    }

    #[test]
    fn test_should_update() {
        let db = setup_test_db();
        let repo = As2relRepository::new(&db.conn);

        // Empty database should need update
        assert!(repo.should_update());

        // Insert data with old timestamp
        db.conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65001, 100, 10, 0)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test.json', 1, 100)",
                [],
            )
            .unwrap();

        // Old data should need update
        assert!(repo.should_update());
    }
}
