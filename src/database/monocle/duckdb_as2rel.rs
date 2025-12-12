//! AS2Rel repository for the DuckDB database
//!
//! This module provides data access operations for AS-level relationships.
//! Data is sourced from BGPKIT's AS2Rel dataset.

use anyhow::{anyhow, Result};
use duckdb::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

use crate::database::core::DuckDbConn;

/// Default URL for AS2Rel data
pub const DUCKDB_BGPKIT_AS2REL_URL: &str = "https://data.bgpkit.com/as2rel/as2rel-latest.json.bz2";

/// Seven days in seconds (for staleness check)
const SEVEN_DAYS_SECS: u64 = 7 * 24 * 60 * 60;

/// Repository for AS2Rel data operations (DuckDB version)
///
/// Provides methods for querying and updating AS-level relationship data
/// in the DuckDB database.
pub struct DuckDbAs2relRepository<'a> {
    conn: &'a DuckDbConn,
}

/// An entry in the AS2Rel dataset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuckDbAs2relEntry {
    pub asn1: u32,
    pub asn2: u32,
    pub paths_count: u32,
    pub peers_count: u32,
    #[serde(rename = "relationship")]
    pub rel: i8,
}

/// A record from the AS2Rel database
#[derive(Debug, Clone)]
pub struct DuckDbAs2relRecord {
    pub asn1: u32,
    pub asn2: u32,
    pub paths_count: u32,
    pub peers_count: u32,
    pub rel: i8,
}

/// Aggregated relationship between two ASNs
#[derive(Debug, Clone)]
pub struct DuckDbAggregatedRelationship {
    pub asn1: u32,
    pub asn2: u32,
    pub asn2_name: Option<String>,
    pub connected_count: u32,
    pub as1_upstream_count: u32,
    pub as2_upstream_count: u32,
}

/// Metadata about the AS2Rel data
#[derive(Debug, Clone)]
pub struct DuckDbAs2relMeta {
    pub file_url: String,
    pub last_updated: u64,
    pub max_peers_count: u32,
}

impl<'a> DuckDbAs2relRepository<'a> {
    /// Create a new AS2Rel repository
    pub fn new(conn: &'a DuckDbConn) -> Self {
        Self { conn }
    }

    /// Check if the AS2Rel data is empty
    pub fn is_empty(&self) -> bool {
        self.conn.table_count("as2rel").unwrap_or(0) == 0
    }

    /// Get the count of relationship entries
    pub fn count(&self) -> Result<u64> {
        self.conn.table_count("as2rel")
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
    pub fn get_meta(&self) -> Result<Option<DuckDbAs2relMeta>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT file_url, epoch(last_updated)::BIGINT, max_peers_count FROM as2rel_meta WHERE id = 1",
        )?;

        let result = stmt.query_row([], |row| {
            Ok(DuckDbAs2relMeta {
                file_url: row.get(0)?,
                last_updated: row.get::<_, i64>(1)? as u64,
                max_peers_count: row.get(2)?,
            })
        });

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
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
    pub fn search_asn(&self, asn: u32) -> Result<Vec<DuckDbAs2relRecord>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT asn1, asn2, paths_count, peers_count, rel
             FROM as2rel
             WHERE asn1 = ? OR asn2 = ?",
        )?;

        let mut rows = stmt
            .query(params![asn, asn])
            .map_err(|e| anyhow!("Failed to search ASN: {}", e))?;

        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(DuckDbAs2relRecord {
                asn1: row.get(0)?,
                asn2: row.get(1)?,
                paths_count: row.get(2)?,
                peers_count: row.get(3)?,
                rel: row.get(4)?,
            });
        }

        Ok(records)
    }

    /// Search for relationship between two specific ASNs
    pub fn search_pair(&self, asn1: u32, asn2: u32) -> Result<Vec<DuckDbAs2relRecord>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT asn1, asn2, paths_count, peers_count, rel
             FROM as2rel
             WHERE (asn1 = ? AND asn2 = ?) OR (asn1 = ? AND asn2 = ?)",
        )?;

        let mut rows = stmt
            .query(params![asn1, asn2, asn2, asn1])
            .map_err(|e| anyhow!("Failed to search pair: {}", e))?;

        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(DuckDbAs2relRecord {
                asn1: row.get(0)?,
                asn2: row.get(1)?,
                paths_count: row.get(2)?,
                peers_count: row.get(3)?,
                rel: row.get(4)?,
            });
        }

        Ok(records)
    }

    /// Search for relationships of an ASN with organization names from as2org
    pub fn search_asn_with_names(&self, asn: u32) -> Result<Vec<DuckDbAggregatedRelationship>> {
        // First get the raw relationships
        let records = self.search_asn(asn)?;
        self.aggregate_with_names(records, asn)
    }

    /// Search for relationship between two ASNs with organization names
    pub fn search_pair_with_names(
        &self,
        asn1: u32,
        asn2: u32,
    ) -> Result<Vec<DuckDbAggregatedRelationship>> {
        let records = self.search_pair(asn1, asn2)?;
        self.aggregate_with_names(records, asn1)
    }

    /// Aggregate relationship records and add organization names
    fn aggregate_with_names(
        &self,
        records: Vec<DuckDbAs2relRecord>,
        perspective_asn: u32,
    ) -> Result<Vec<DuckDbAggregatedRelationship>> {
        // Aggregate by peer ASN
        let mut aggregated: HashMap<u32, DuckDbAggregatedRelationship> = HashMap::new();

        for record in records {
            let (peer_asn, is_asn1) = if record.asn1 == perspective_asn {
                (record.asn2, true)
            } else {
                (record.asn1, false)
            };

            let entry = aggregated
                .entry(peer_asn)
                .or_insert(DuckDbAggregatedRelationship {
                    asn1: perspective_asn,
                    asn2: peer_asn,
                    asn2_name: None,
                    connected_count: 0,
                    as1_upstream_count: 0,
                    as2_upstream_count: 0,
                });

            entry.connected_count += record.peers_count;

            // rel: -1 = asn1 is customer of asn2
            //       0 = peers
            //       1 = asn1 is provider of asn2
            match record.rel {
                -1 => {
                    if is_asn1 {
                        entry.as2_upstream_count += record.peers_count;
                    } else {
                        entry.as1_upstream_count += record.peers_count;
                    }
                }
                1 => {
                    if is_asn1 {
                        entry.as1_upstream_count += record.peers_count;
                    } else {
                        entry.as2_upstream_count += record.peers_count;
                    }
                }
                _ => {} // peer relationship (0)
            }
        }

        // Try to add organization names using a JOIN with as2org
        let mut results: Vec<DuckDbAggregatedRelationship> = aggregated.into_values().collect();

        if !results.is_empty() {
            // Build a map of ASN -> org_name
            let asns: Vec<u32> = results.iter().map(|r| r.asn2).collect();
            let placeholders: Vec<String> = asns.iter().map(|_| "?".to_string()).collect();
            let query = format!(
                "SELECT asn, org_name FROM as2org WHERE asn IN ({})",
                placeholders.join(",")
            );

            if let Ok(mut stmt) = self.conn.conn.prepare(&query) {
                let params: Vec<&dyn duckdb::ToSql> =
                    asns.iter().map(|a| a as &dyn duckdb::ToSql).collect();

                if let Ok(mut rows) = stmt.query(params.as_slice()) {
                    let mut name_map: HashMap<u32, String> = HashMap::new();
                    while let Ok(Some(row)) = rows.next() {
                        if let (Ok(asn), Ok(name)) = (row.get::<_, u32>(0), row.get::<_, String>(1))
                        {
                            name_map.insert(asn, name);
                        }
                    }
                    for result in &mut results {
                        result.asn2_name = name_map.get(&result.asn2).cloned();
                    }
                }
            }
        }

        Ok(results)
    }

    /// Clear all AS2Rel data
    pub fn clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM as2rel")
            .map_err(|e| anyhow!("Failed to clear as2rel: {}", e))?;
        self.conn
            .execute("DELETE FROM as2rel_meta")
            .map_err(|e| anyhow!("Failed to clear as2rel_meta: {}", e))?;
        Ok(())
    }

    /// Load AS2Rel data from the default URL
    pub fn load_from_url(&self) -> Result<usize> {
        self.load_from_path(DUCKDB_BGPKIT_AS2REL_URL)
    }

    /// Load AS2Rel data from a custom path (file or URL)
    pub fn load_from_path(&self, path: &str) -> Result<usize> {
        self.clear()?;

        info!("Loading AS2Rel data from {}...", path);

        // Load entries from the JSON file
        let entries: Vec<DuckDbAs2relEntry> = oneio::read_json_struct(path)
            .map_err(|e| anyhow!("Failed to read AS2Rel data from {}: {}", path, e))?;

        info!("Loaded {} entries, inserting into DuckDB...", entries.len());

        // Find max peers count for normalization
        let max_peers = entries.iter().map(|e| e.peers_count).max().unwrap_or(0);

        // Use a transaction for all inserts
        self.conn.transaction()?;

        let entry_count = entries.len();

        {
            let mut stmt = self.conn.conn.prepare(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel)
                 VALUES (?, ?, ?, ?, ?)",
            )?;

            for entry in &entries {
                stmt.execute(params![
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

            self.conn.conn.execute(
                "INSERT OR REPLACE INTO as2rel_meta (id, file_url, last_updated, max_peers_count)
                 VALUES (1, ?, to_timestamp(?), ?)",
                params![path, now as i64, max_peers],
            )?;
        }

        self.conn.commit()?;

        info!("AS2Rel data loading finished: {} entries", entry_count);

        Ok(entry_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::core::{DuckDbConn, DuckDbSchemaManager};

    fn setup_test_db() -> DuckDbConn {
        let conn = DuckDbConn::open_in_memory().unwrap();
        let schema = DuckDbSchemaManager::new(&conn);
        schema.initialize_core().unwrap();
        conn
    }

    #[test]
    fn test_is_empty() {
        let conn = setup_test_db();
        let repo = DuckDbAs2relRepository::new(&conn);
        assert!(repo.is_empty());
    }

    #[test]
    fn test_insert_and_search() {
        let conn = setup_test_db();

        // Insert test data directly
        conn.execute(
            "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65001, 100, 10, 0)",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65002, 200, 20, 1)",
        )
        .unwrap();

        let repo = DuckDbAs2relRepository::new(&conn);

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
        let conn = setup_test_db();

        // Insert meta data
        conn.execute(
            "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test.json', to_timestamp(1234567890), 100)",
        )
        .unwrap();

        let repo = DuckDbAs2relRepository::new(&conn);
        let meta = repo.get_meta().unwrap().unwrap();

        assert_eq!(meta.file_url, "test.json");
        assert_eq!(meta.last_updated, 1234567890);
        assert_eq!(meta.max_peers_count, 100);
    }

    #[test]
    fn test_clear() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65001, 100, 10, 0)",
        )
        .unwrap();

        let repo = DuckDbAs2relRepository::new(&conn);
        assert!(!repo.is_empty());

        repo.clear().unwrap();
        assert!(repo.is_empty());
    }

    #[test]
    fn test_should_update() {
        let conn = setup_test_db();
        let repo = DuckDbAs2relRepository::new(&conn);

        // Empty database should need update
        assert!(repo.should_update());

        // Insert data with old timestamp
        conn.execute(
            "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65001, 100, 10, 0)",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test.json', to_timestamp(1), 100)",
        )
        .unwrap();

        // Old data should need update
        assert!(repo.should_update());
    }

    #[test]
    fn test_aggregate_relationships() {
        let conn = setup_test_db();

        // Insert as2org data for name lookup
        conn.execute(
            "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source) VALUES
             (65001, 'Peer AS', 'PEER-ORG', 'Peer Organization', 'US', 'test')",
        )
        .unwrap();

        // Insert relationship data
        conn.execute(
            "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (65000, 65001, 100, 10, 0)",
        )
        .unwrap();

        let repo = DuckDbAs2relRepository::new(&conn);
        let results = repo.search_asn_with_names(65000).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].asn1, 65000);
        assert_eq!(results[0].asn2, 65001);
        assert_eq!(results[0].asn2_name, Some("Peer Organization".to_string()));
    }
}
