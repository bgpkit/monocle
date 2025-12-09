//! AS2Rel data handling utility.
//!
//! Data source: BGPKIT AS relationship data from data.bgpkit.com.
//! The data is loaded from the BGPKIT data server and cached in a local SQLite database.
//!
//! Relationship semantics:
//! - `rel=0`: asn1 and asn2 are connected (seen together on AS paths)
//! - `rel=1`: asn1 is the upstream of asn2 (asn2 is downstream of asn1)
//!
//! Column definitions:
//! - `connected`: Percentage of collectors that see any connection between the AS pair
//! - `peer`: Pure peering only (connected - as1_upstream - as2_upstream)
//! - `as1_upstream`: Percentage of collectors that see asn1 as upstream of asn2
//! - `as2_upstream`: Percentage of collectors that see asn2 as upstream of asn1
//!
//! Percentages are calculated as (count / max_peers_count * 100%).

use crate::database::MonocleDatabase;

use anyhow::{anyhow, Result};
use chrono::{TimeZone, Utc};
use chrono_humanize::HumanTime;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tabled::Tabled;
use tracing::info;

/// Default URL for the AS2Rel data file
pub const BGPKIT_AS2REL_URL: &str = "https://data.bgpkit.com/as2rel/as2rel-latest.json.bz2";

/// Number of seconds in 7 days (for cache expiration check)
const SEVEN_DAYS_SECS: u64 = 7 * 24 * 60 * 60;

/// AS relationship entry from the JSON data file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct As2relEntry {
    pub asn1: u32,
    pub asn2: u32,
    pub paths_count: u32,
    pub peers_count: u32,
    /// Relationship type: 0 = peer, 1 = asn1 is upstream of asn2
    pub rel: i32,
}

/// Search result showing relationship between two ASNs
/// Percentages are based on count / max_peers_count
#[derive(Debug, Clone, Tabled)]
pub struct As2relSearchResult {
    pub asn1: u32,
    pub asn2: u32,
    #[tabled(skip)]
    pub asn2_name: Option<String>,
    /// Percentage of collectors seeing any connection between asn1 and asn2
    pub connected: String,
    #[tabled(skip)]
    pub connected_pct: f64, // For sorting
    /// Percentage of collectors seeing pure peering (connected - upstream - downstream)
    pub peer: String,
    /// Percentage of collectors seeing asn1 as upstream of asn2
    pub as1_upstream: String,
    /// Percentage of collectors seeing asn2 as upstream of asn1
    pub as2_upstream: String,
}

/// Search result with asn2_name column visible
#[derive(Debug, Clone, Tabled)]
pub struct As2relSearchResultWithName {
    pub asn1: u32,
    pub asn2: u32,
    pub asn2_name: String,
    pub connected: String,
    pub peer: String,
    pub as1_upstream: String,
    pub as2_upstream: String,
}

impl As2relSearchResult {
    pub fn with_name(self) -> As2relSearchResultWithName {
        let name = self.asn2_name.unwrap_or_default();
        // Truncate to 20 characters (UTF-8 safe)
        let truncated = if name.chars().count() > 20 {
            let truncated_str: String = name.chars().take(17).collect();
            format!("{}...", truncated_str)
        } else {
            name
        };
        As2relSearchResultWithName {
            asn1: self.asn1,
            asn2: self.asn2,
            asn2_name: truncated,
            connected: self.connected,
            peer: self.peer,
            as1_upstream: self.as1_upstream,
            as2_upstream: self.as2_upstream,
        }
    }
}

/// Sort order for search results
#[derive(Debug, Clone, Copy, Default)]
pub enum As2relSortOrder {
    /// Sort by connected percentage descending (default)
    #[default]
    ConnectedDesc,
    /// Sort by asn2 ascending
    Asn2Asc,
}

/// Aggregated relationship data for an AS pair
#[derive(Debug, Clone, Default)]
struct AggregatedRelationship {
    asn1: u32,
    asn2: u32,
    asn2_name: Option<String>, // org_name for asn2 from as2org
    connected_count: u32,      // peers_count for rel=0 (any connection)
    as1_upstream_count: u32,   // peers_count where asn1 is upstream of asn2
    as2_upstream_count: u32,   // peers_count where asn2 is upstream of asn1
}

impl AggregatedRelationship {
    fn to_search_result(&self, max_peers: u32) -> As2relSearchResult {
        let calc_pct = |count: u32| -> f64 {
            if count == 0 || max_peers == 0 {
                0.0
            } else {
                (count as f64 / max_peers as f64) * 100.0
            }
        };

        let format_pct = |pct: f64| -> String {
            if pct == 0.0 {
                String::new()
            } else {
                format!("{:.1}%", pct)
            }
        };

        // Pure peer count = connected - upstream - downstream
        let peer_count = self
            .connected_count
            .saturating_sub(self.as1_upstream_count)
            .saturating_sub(self.as2_upstream_count);

        let connected_pct = calc_pct(self.connected_count);

        As2relSearchResult {
            asn1: self.asn1,
            asn2: self.asn2,
            asn2_name: self.asn2_name.clone(),
            connected: format_pct(connected_pct),
            connected_pct,
            peer: format_pct(calc_pct(peer_count)),
            as1_upstream: format_pct(calc_pct(self.as1_upstream_count)),
            as2_upstream: format_pct(calc_pct(self.as2_upstream_count)),
        }
    }
}

/// AS2Rel database handler
pub struct As2rel {
    db: MonocleDatabase,
}

impl As2rel {
    /// Create a new As2rel instance with the given database path
    pub fn new(db_path: &Option<String>) -> Result<As2rel> {
        let mut db = MonocleDatabase::new(db_path)?;
        As2rel::initialize_db(&mut db)?;
        Ok(As2rel { db })
    }

    fn initialize_db(db: &mut MonocleDatabase) -> Result<()> {
        // Create meta table for tracking data source, update time, and max peers count
        db.conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS as2rel_meta (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                file_url TEXT NOT NULL,
                last_updated INTEGER NOT NULL,
                max_peers_count INTEGER NOT NULL DEFAULT 0
            );
            "#,
            [],
        )?;

        // Create main data table - allows multiple entries per AS pair with different rel values
        db.conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS as2rel (
                asn1 INTEGER NOT NULL,
                asn2 INTEGER NOT NULL,
                paths_count INTEGER NOT NULL,
                peers_count INTEGER NOT NULL,
                rel INTEGER NOT NULL,
                PRIMARY KEY (asn1, asn2, rel)
            );
            "#,
            [],
        )?;

        // Add indexes for better query performance
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_as2rel_asn1 ON as2rel(asn1)",
            [],
        )?;
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_as2rel_asn2 ON as2rel(asn2)",
            [],
        )?;

        // Enable SQLite performance optimizations
        let _: String = db
            .conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        db.conn.execute("PRAGMA synchronous=NORMAL", [])?;
        db.conn.execute("PRAGMA cache_size=100000", [])?;
        db.conn.execute("PRAGMA temp_store=MEMORY", [])?;

        Ok(())
    }

    /// Check if the database is empty
    pub fn is_db_empty(&self) -> bool {
        let count: u32 = self
            .db
            .conn
            .query_row("SELECT COUNT(*) FROM as2rel", [], |row| row.get(0))
            .unwrap_or(0);
        count == 0
    }

    /// Check if the data should be updated (older than 7 days or empty)
    pub fn should_update(&self) -> bool {
        if self.is_db_empty() {
            return true;
        }

        let last_updated: Result<u64, _> = self.db.conn.query_row(
            "SELECT last_updated FROM as2rel_meta WHERE id = 1",
            [],
            |row| row.get(0),
        );

        match last_updated {
            Ok(timestamp) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                now - timestamp > SEVEN_DAYS_SECS
            }
            Err(_) => true,
        }
    }

    /// Get the last update timestamp
    pub fn get_last_updated(&self) -> Option<u64> {
        self.db
            .conn
            .query_row(
                "SELECT last_updated FROM as2rel_meta WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .ok()
    }

    /// Get the data source URL
    pub fn get_data_source(&self) -> Option<String> {
        self.db
            .conn
            .query_row("SELECT file_url FROM as2rel_meta WHERE id = 1", [], |row| {
                row.get(0)
            })
            .ok()
    }

    /// Get the max peers count from meta data
    pub fn get_max_peers_count(&self) -> u32 {
        self.db
            .conn
            .query_row(
                "SELECT max_peers_count FROM as2rel_meta WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
    }

    /// Clear all data from the database
    pub fn clear_db(&self) -> Result<()> {
        self.db.conn.execute("DELETE FROM as2rel", [])?;
        self.db.conn.execute("DELETE FROM as2rel_meta", [])?;
        Ok(())
    }

    /// Update the database with data from the default URL
    pub fn update(&self) -> Result<()> {
        self.update_with(BGPKIT_AS2REL_URL)
    }

    /// Update the database with data from a custom URL or file path
    pub fn update_with(&self, url: &str) -> Result<()> {
        self.clear_db()?;

        info!("loading AS2Rel data from {}...", url);

        let entries: Vec<As2relEntry> = oneio::read_json_struct(url)
            .map_err(|e| anyhow!("Failed to load AS2Rel data from {}: {}", url, e))?;

        info!(
            "loaded {} AS relationship entries, inserting to sqlite db now",
            entries.len()
        );

        // Calculate max peers_count across all entries
        let max_peers_count = entries.iter().map(|e| e.peers_count).max().unwrap_or(0);
        info!("max peers_count in dataset: {}", max_peers_count);

        // Use a transaction for all inserts
        let tx = self.db.conn.unchecked_transaction()?;

        {
            // Use INSERT OR REPLACE to handle duplicate (asn1, asn2, rel) combinations
            // by keeping the latest values
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            for entry in &entries {
                stmt.execute((
                    entry.asn1,
                    entry.asn2,
                    entry.paths_count,
                    entry.peers_count,
                    entry.rel,
                ))?;
            }

            // Update meta table with max_peers_count
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            tx.execute(
                "INSERT OR REPLACE INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, ?1, ?2, ?3)",
                (url, now as i64, max_peers_count),
            )?;
        }

        tx.commit()?;
        info!(
            "AS2Rel data loading finished: {} relationship entries, max_peers_count: {}",
            entries.len(),
            max_peers_count
        );
        Ok(())
    }

    /// Aggregate relationship entries for a given ASN pair, computing percentages
    fn aggregate_relationships(
        &self,
        query_asn: u32,
        entries: Vec<As2relEntry>,
        asn2_names: Option<&HashMap<u32, String>>,
    ) -> Vec<As2relSearchResult> {
        let max_peers = self.get_max_peers_count();

        // Group entries by the AS pair, normalizing so query_asn is always asn1
        let mut aggregated: HashMap<(u32, u32), AggregatedRelationship> = HashMap::new();

        for entry in entries {
            // Normalize so that query_asn is always asn1 in the result
            let (asn1, asn2, is_query_asn1) = if entry.asn1 == query_asn {
                (entry.asn1, entry.asn2, true)
            } else {
                (query_asn, entry.asn1, false)
            };

            let agg = aggregated.entry((asn1, asn2)).or_insert_with(|| {
                let name = asn2_names.and_then(|m| m.get(&asn2).cloned());
                AggregatedRelationship {
                    asn1,
                    asn2,
                    asn2_name: name,
                    ..Default::default()
                }
            });

            match entry.rel {
                0 => {
                    // Connected relationship (any connection seen on AS paths)
                    agg.connected_count += entry.peers_count;
                }
                1 => {
                    // entry.asn1 is upstream of entry.asn2
                    if is_query_asn1 {
                        // query_asn (asn1) is upstream of entry.asn2 (asn2)
                        agg.as1_upstream_count += entry.peers_count;
                    } else {
                        // entry.asn1 is upstream of query_asn (entry.asn2)
                        // From normalized view: asn2 (entry.asn1) is upstream of asn1 (query_asn)
                        agg.as2_upstream_count += entry.peers_count;
                    }
                }
                _ => {}
            }
        }

        aggregated
            .into_values()
            .map(|agg| agg.to_search_result(max_peers))
            .collect()
    }

    /// Sort results by the specified order
    pub fn sort_results(results: &mut [As2relSearchResult], order: As2relSortOrder) {
        match order {
            As2relSortOrder::ConnectedDesc => {
                results.sort_by(|a, b| {
                    b.connected_pct
                        .partial_cmp(&a.connected_pct)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            As2relSortOrder::Asn2Asc => {
                results.sort_by_key(|r| r.asn2);
            }
        }
    }

    /// Search for relationships involving one ASN
    pub fn search_asn(&self, asn: u32) -> Result<Vec<As2relSearchResult>> {
        self.search_asn_impl(asn, false)
    }

    /// Search for relationships involving one ASN, with optional org_name lookup via JOIN
    pub fn search_asn_with_names(&self, asn: u32) -> Result<Vec<As2relSearchResult>> {
        self.search_asn_impl(asn, true)
    }

    /// Internal implementation for search_asn with optional name lookup
    fn search_asn_impl(&self, asn: u32, include_names: bool) -> Result<Vec<As2relSearchResult>> {
        // Query with LEFT JOIN to get org_name for the "other" ASN
        let query = if include_names {
            r#"
            SELECT r.asn1, r.asn2, r.paths_count, r.peers_count, r.rel,
                   CASE WHEN r.asn1 = ?1 THEN o2.name ELSE o1.name END as other_org_name
            FROM as2rel r
            LEFT JOIN as2org_org o1 ON o1.org_id = (SELECT org_id FROM as2org_as WHERE asn = r.asn1)
            LEFT JOIN as2org_org o2 ON o2.org_id = (SELECT org_id FROM as2org_as WHERE asn = r.asn2)
            WHERE r.asn1 = ?1 OR r.asn2 = ?1
            "#
        } else {
            "SELECT asn1, asn2, paths_count, peers_count, rel, NULL as other_org_name FROM as2rel WHERE asn1 = ?1 OR asn2 = ?1"
        };

        let mut stmt = self.db.conn.prepare(query)?;

        // Collect entries and names together
        let mut entries: Vec<As2relEntry> = Vec::new();
        let mut asn2_names: HashMap<u32, String> = HashMap::new();

        let rows = stmt.query_map([asn], |row| {
            let asn1: u32 = row.get(0)?;
            let asn2: u32 = row.get(1)?;
            let other_name: Option<String> = row.get(5)?;

            // Determine which ASN is "asn2" from the query perspective
            let other_asn = if asn1 == asn { asn2 } else { asn1 };

            Ok((
                As2relEntry {
                    asn1,
                    asn2,
                    paths_count: row.get(2)?,
                    peers_count: row.get(3)?,
                    rel: row.get(4)?,
                },
                other_asn,
                other_name,
            ))
        })?;

        for row in rows.flatten() {
            entries.push(row.0);
            if let Some(name) = row.2 {
                asn2_names.insert(row.1, name);
            }
        }

        let names_map = if include_names {
            Some(&asn2_names)
        } else {
            None
        };

        Ok(self.aggregate_relationships(asn, entries, names_map))
    }

    /// Search for relationship between two specific ASNs
    pub fn search_pair(&self, asn1: u32, asn2: u32) -> Result<Vec<As2relSearchResult>> {
        self.search_pair_impl(asn1, asn2, false)
    }

    /// Search for relationship between two specific ASNs, with org_name lookup
    pub fn search_pair_with_names(&self, asn1: u32, asn2: u32) -> Result<Vec<As2relSearchResult>> {
        self.search_pair_impl(asn1, asn2, true)
    }

    /// Internal implementation for search_pair with optional name lookup
    fn search_pair_impl(
        &self,
        asn1: u32,
        asn2: u32,
        include_names: bool,
    ) -> Result<Vec<As2relSearchResult>> {
        let query = if include_names {
            r#"
            SELECT r.asn1, r.asn2, r.paths_count, r.peers_count, r.rel,
                   o.name as asn2_org_name
            FROM as2rel r
            LEFT JOIN as2org_as a ON a.asn = ?2
            LEFT JOIN as2org_org o ON o.org_id = a.org_id
            WHERE (r.asn1 = ?1 AND r.asn2 = ?2) OR (r.asn1 = ?2 AND r.asn2 = ?1)
            "#
        } else {
            "SELECT asn1, asn2, paths_count, peers_count, rel, NULL as asn2_org_name FROM as2rel WHERE (asn1 = ?1 AND asn2 = ?2) OR (asn1 = ?2 AND asn2 = ?1)"
        };

        let mut stmt = self.db.conn.prepare(query)?;

        let mut entries: Vec<As2relEntry> = Vec::new();
        let mut asn2_names: HashMap<u32, String> = HashMap::new();

        let rows = stmt.query_map([asn1, asn2], |row| {
            let asn2_name: Option<String> = row.get(5)?;
            Ok((
                As2relEntry {
                    asn1: row.get(0)?,
                    asn2: row.get(1)?,
                    paths_count: row.get(2)?,
                    peers_count: row.get(3)?,
                    rel: row.get(4)?,
                },
                asn2_name,
            ))
        })?;

        for row in rows.flatten() {
            entries.push(row.0);
            if let Some(name) = row.1 {
                asn2_names.insert(asn2, name);
            }
        }

        let names_map = if include_names {
            Some(&asn2_names)
        } else {
            None
        };

        Ok(self.aggregate_relationships(asn1, entries, names_map))
    }

    /// Count total relationship entries in the database
    pub fn count_relationships(&self) -> u32 {
        self.db
            .conn
            .query_row("SELECT COUNT(*) FROM as2rel", [], |row| row.get(0))
            .unwrap_or(0)
    }

    /// Get the explanation text for the output
    pub fn get_explanation(&self) -> String {
        let max_peers = self.get_max_peers_count();
        let last_updated_str = match self.get_last_updated() {
            Some(ts) => {
                let dt = Utc.timestamp_opt(ts as i64, 0).single();
                match dt {
                    Some(datetime) => {
                        let ht = HumanTime::from(datetime);
                        format!("{} ({})", datetime.to_rfc3339(), ht)
                    }
                    None => "unknown".to_string(),
                }
            }
            None => "unknown".to_string(),
        };

        format!(
            r#"
Relationship data from BGPKIT (data.bgpkit.com/as2rel).
Last updated: {}

Column explanation:
- asn1, asn2: The AS pair being queried
- connected: Percentage of route collectors ({} max) that see any connection between asn1 and asn2
- peer: Percentage seeing pure peering only (connected - as1_upstream - as2_upstream)
- as1_upstream: Percentage of route collectors that see asn1 as an upstream of asn2
- as2_upstream: Percentage of route collectors that see asn2 as an upstream of asn1

Percentages are calculated as: (count / max_peers_count) * 100%
where max_peers_count = {} (the maximum peers_count observed in the dataset).
"#,
            last_updated_str, max_peers, max_peers
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creating_db() {
        let as2rel = As2rel::new(&Some("./test_as2rel.sqlite3".to_string())).unwrap();
        as2rel.clear_db().unwrap();
        assert!(as2rel.is_db_empty());
        // Clean up
        std::fs::remove_file("./test_as2rel.sqlite3").ok();
    }

    #[test]
    fn test_should_update_empty_db() {
        let as2rel = As2rel::new(&None).unwrap();
        assert!(as2rel.should_update());
    }

    #[test]
    fn test_search_empty() {
        let as2rel = As2rel::new(&None).unwrap();
        let results = as2rel.search_asn(12345).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_pair_empty() {
        let as2rel = As2rel::new(&None).unwrap();
        let results = as2rel.search_pair(12345, 67890).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_insert_and_query_with_percentages() {
        let as2rel = As2rel::new(&None).unwrap();

        // Set up meta with max_peers_count = 100
        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test', 0, 100)",
                [],
            )
            .unwrap();

        // Insert test data: AS 100 and AS 200 have both connected and upstream relationships
        // Connected relationship with 60 peers_count
        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (100, 200, 100, 60, 0)",
                [],
            )
            .unwrap();
        // Upstream relationship (100 is upstream of 200) with 40 peers_count
        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (100, 200, 80, 40, 1)",
                [],
            )
            .unwrap();

        // Test search_pair from AS 100's perspective
        let results = as2rel.search_pair(100, 200).unwrap();
        assert_eq!(results.len(), 1);

        let result = &results[0];
        assert_eq!(result.asn1, 100);
        assert_eq!(result.asn2, 200);
        // max_peers = 100
        // Connected: 60/100 = 60%
        // as1_upstream: 40/100 = 40% (100 is upstream of 200)
        // Peer: (60 - 40 - 0) / 100 = 20%
        assert_eq!(result.connected, "60.0%");
        assert_eq!(result.peer, "20.0%");
        assert_eq!(result.as1_upstream, "40.0%");
        assert!(result.as2_upstream.is_empty());
    }

    #[test]
    fn test_insert_and_query_reverse_perspective() {
        let as2rel = As2rel::new(&None).unwrap();

        // Set up meta with max_peers_count = 100
        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test', 0, 100)",
                [],
            )
            .unwrap();

        // Insert: AS 300 is upstream of AS 400 (rel=1, asn1=300 is upstream of asn2=400)
        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (300, 400, 50, 50, 1)",
                [],
            )
            .unwrap();

        // Query from AS 400's perspective (400 as asn1)
        let results = as2rel.search_pair(400, 300).unwrap();
        assert_eq!(results.len(), 1);

        let result = &results[0];
        assert_eq!(result.asn1, 400);
        assert_eq!(result.asn2, 300);
        // From 400's perspective as asn1, 300 (asn2) is upstream of 400 (asn1)
        // So as2_upstream should be 50%
        // No connected entry, so connected and peer are empty
        assert!(result.connected.is_empty());
        assert!(result.peer.is_empty());
        assert!(result.as1_upstream.is_empty());
        assert_eq!(result.as2_upstream, "50.0%");

        // Query from AS 300's perspective (300 as asn1)
        let results = as2rel.search_pair(300, 400).unwrap();
        assert_eq!(results.len(), 1);

        let result = &results[0];
        assert_eq!(result.asn1, 300);
        assert_eq!(result.asn2, 400);
        // From 300's perspective as asn1, 300 IS upstream of 400
        // So as1_upstream should be 50%
        assert!(result.connected.is_empty());
        assert!(result.peer.is_empty());
        assert_eq!(result.as1_upstream, "50.0%");
        assert!(result.as2_upstream.is_empty());
    }

    #[test]
    fn test_meta_tracking() {
        let as2rel = As2rel::new(&None).unwrap();

        // Initially no meta data
        assert!(as2rel.get_last_updated().is_none());
        assert!(as2rel.get_data_source().is_none());
        assert_eq!(as2rel.get_max_peers_count(), 0);

        // Insert meta data manually
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test-url', ?1, 500)",
                [now as i64],
            )
            .unwrap();

        assert_eq!(as2rel.get_data_source(), Some("test-url".to_string()));
        assert!(as2rel.get_last_updated().is_some());
        assert_eq!(as2rel.get_max_peers_count(), 500);

        // Should not need update if just updated
        // Insert some data to make it non-empty
        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES (1, 2, 1, 1, 0)",
                [],
            )
            .unwrap();
        assert!(!as2rel.should_update());
    }

    #[test]
    fn test_as2rel_entry_serialization() {
        let entry = As2relEntry {
            asn1: 100,
            asn2: 200,
            paths_count: 50,
            peers_count: 25,
            rel: 0,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: As2relEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.asn1, 100);
        assert_eq!(parsed.asn2, 200);
        assert_eq!(parsed.paths_count, 50);
        assert_eq!(parsed.peers_count, 25);
        assert_eq!(parsed.rel, 0);
    }

    #[test]
    fn test_aggregated_relationship() {
        let agg = AggregatedRelationship {
            asn1: 100,
            asn2: 200,
            asn2_name: None,
            connected_count: 80,
            as1_upstream_count: 30,
            as2_upstream_count: 20,
        };

        // With max_peers = 100
        // connected = 80/100 = 80%
        // peer = (80 - 30 - 20) / 100 = 30%
        let result = agg.to_search_result(100);
        assert_eq!(result.connected, "80.0%");
        assert_eq!(result.peer, "30.0%");
        assert_eq!(result.as1_upstream, "30.0%");
        assert_eq!(result.as2_upstream, "20.0%");

        // With zero counts
        let agg_zero = AggregatedRelationship {
            asn1: 100,
            asn2: 200,
            asn2_name: None,
            connected_count: 0,
            as1_upstream_count: 0,
            as2_upstream_count: 0,
        };
        let result = agg_zero.to_search_result(100);
        assert!(result.connected.is_empty());
        assert!(result.peer.is_empty());
        assert!(result.as1_upstream.is_empty());
        assert!(result.as2_upstream.is_empty());
    }

    #[test]
    fn test_get_explanation() {
        let as2rel = As2rel::new(&None).unwrap();

        // Set up meta with max_peers_count
        as2rel
            .db
            .conn
            .execute(
                "INSERT INTO as2rel_meta (id, file_url, last_updated, max_peers_count) VALUES (1, 'test', 0, 800)",
                [],
            )
            .unwrap();

        let explanation = as2rel.get_explanation();
        assert!(explanation.contains("800"));
        assert!(explanation.contains("connected"));
        assert!(explanation.contains("peer"));
        assert!(explanation.contains("as1_upstream"));
        assert!(explanation.contains("as2_upstream"));
        assert!(explanation.contains("Last updated"));
    }

    #[test]
    #[ignore] // This test requires network access
    fn test_load_from_url() {
        let as2rel = As2rel::new(&None).unwrap();
        as2rel.update().unwrap();

        assert!(!as2rel.is_db_empty());
        assert!(!as2rel.should_update());
        assert!(as2rel.count_relationships() > 0);
        assert!(as2rel.get_max_peers_count() > 0);

        // Test searching for a well-known ASN (Hurricane Electric)
        let results = as2rel.search_asn(6939).unwrap();
        assert!(!results.is_empty());
    }
}
