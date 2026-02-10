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

/// Summary of AS connectivity (upstreams, peers, downstreams)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsConnectivitySummary {
    pub asn: u32,
    /// Upstream providers (ASes that provide transit)
    pub upstreams: ConnectivityGroup,
    /// Peers (settlement-free interconnection)
    pub peers: ConnectivityGroup,
    /// Downstream customers (ASes that receive transit)
    pub downstreams: ConnectivityGroup,
    /// Total number of neighbors
    pub total_neighbors: u32,
    /// Maximum peers count (for visibility percentage calculation)
    pub max_peers_count: u32,
}

/// A group of related ASes (upstreams, peers, or downstreams)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityGroup {
    /// Total count in this category
    pub count: u32,
    /// Percentage of total neighbors (0.0 - 100.0)
    pub percent: f64,
    /// Top N entries sorted by peers_count DESC
    pub top: Vec<ConnectivityEntry>,
}

/// A single connectivity entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityEntry {
    pub asn: u32,
    /// AS name (if available)
    pub name: Option<String>,
    /// Number of peers observing this relationship
    pub peers_count: u32,
    /// Percentage of max peers (visibility indicator, 0.0 - 100.0)
    pub peers_percent: f64,
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
                COALESCE(
                    NULLIF(p.aka, ''),
                    NULLIF(p.name_long, ''),
                    NULLIF(p.name, ''),
                    NULLIF(ai.org_name, ''),
                    NULLIF(ai.name, ''),
                    NULLIF(o.org_name, ''),
                    NULLIF(o.as_name, ''),
                    c.name
                ) as asn2_name,
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
            LEFT JOIN asinfo_core c
                ON c.asn = CASE WHEN r.asn1 = :asn THEN r.asn2 ELSE r.asn1 END
            LEFT JOIN asinfo_as2org ai
                ON ai.asn = CASE WHEN r.asn1 = :asn THEN r.asn2 ELSE r.asn1 END
            LEFT JOIN asinfo_peeringdb p
                ON p.asn = CASE WHEN r.asn1 = :asn THEN r.asn2 ELSE r.asn1 END
            LEFT JOIN as2org_all o
                ON o.asn = CASE WHEN r.asn1 = :asn THEN r.asn2 ELSE r.asn1 END
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
                COALESCE(
                    NULLIF(p.aka, ''),
                    NULLIF(p.name_long, ''),
                    NULLIF(p.name, ''),
                    NULLIF(ai.org_name, ''),
                    NULLIF(ai.name, ''),
                    NULLIF(o.org_name, ''),
                    NULLIF(o.as_name, ''),
                    c.name
                ) as asn2_name,
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
            LEFT JOIN asinfo_core c ON c.asn = :asn2
            LEFT JOIN asinfo_as2org ai ON ai.asn = :asn2
            LEFT JOIN asinfo_peeringdb p ON p.asn = :asn2
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

    /// Get connectivity summary for an ASN
    ///
    /// Returns categorized relationships (upstreams, peers, downstreams) with:
    /// - Counts and percentages for each category
    /// - Top N entries per category, sorted by peers_count DESC, then ASN ASC
    /// - AS names enriched from asinfo_core table
    ///
    /// # Arguments
    /// * `asn` - The ASN to query
    /// * `top_n` - Maximum number of entries to return per category (0 = unlimited)
    /// * `name_lookup` - Function to look up AS names by ASN
    pub fn get_connectivity_summary<F>(
        &self,
        asn: u32,
        top_n: usize,
        name_lookup: F,
    ) -> Result<Option<AsConnectivitySummary>>
    where
        F: Fn(&[u32]) -> std::collections::HashMap<u32, String>,
    {
        let relationships = self.search_asn(asn)?;

        if relationships.is_empty() {
            return Ok(None);
        }

        // Categorize relationships
        // rel: -1 = asn1 is customer of asn2 (asn2 is provider)
        // rel: 0 = peers
        // rel: 1 = asn1 is provider of asn2 (asn2 is customer)
        let mut upstreams: Vec<(u32, u32)> = Vec::new(); // (neighbor_asn, peers_count)
        let mut peers_list: Vec<(u32, u32)> = Vec::new();
        let mut downstreams: Vec<(u32, u32)> = Vec::new();

        for rel in &relationships {
            let (neighbor_asn, relationship_type) = if rel.asn1 == asn {
                (rel.asn2, rel.rel)
            } else {
                (rel.asn1, -rel.rel) // Reverse the relationship
            };

            match relationship_type {
                -1 => upstreams.push((neighbor_asn, rel.peers_count)),
                0 => peers_list.push((neighbor_asn, rel.peers_count)),
                1 => downstreams.push((neighbor_asn, rel.peers_count)),
                _ => {}
            }
        }

        // Sort by peers_count DESC, then ASN ASC
        let sort_fn = |a: &(u32, u32), b: &(u32, u32)| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0));
        upstreams.sort_by(sort_fn);
        peers_list.sort_by(sort_fn);
        downstreams.sort_by(sort_fn);

        let total = upstreams.len() + peers_list.len() + downstreams.len();
        let total_f64 = total as f64;

        let effective_top_n = if top_n > 0 { top_n } else { 100 };

        // Collect all ASNs that need name lookup
        let all_top_asns: Vec<u32> = upstreams
            .iter()
            .take(effective_top_n)
            .chain(peers_list.iter().take(effective_top_n))
            .chain(downstreams.iter().take(effective_top_n))
            .map(|(asn, _)| *asn)
            .collect();

        let names = name_lookup(&all_top_asns);

        let max_peers_count = self.get_max_peers_count();
        let max_peers_f64 = max_peers_count as f64;

        let build_group = |items: &[(u32, u32)],
                           names: &std::collections::HashMap<u32, String>|
         -> ConnectivityGroup {
            let count = items.len() as u32;
            let percent = if total > 0 {
                (count as f64 / total_f64) * 100.0
            } else {
                0.0
            };

            let top: Vec<ConnectivityEntry> = items
                .iter()
                .take(effective_top_n)
                .map(|(asn, peers_count)| {
                    let peers_percent = if max_peers_count > 0 {
                        (*peers_count as f64 / max_peers_f64) * 100.0
                    } else {
                        0.0
                    };
                    ConnectivityEntry {
                        asn: *asn,
                        name: names.get(asn).cloned(),
                        peers_count: *peers_count,
                        peers_percent,
                    }
                })
                .collect();

            ConnectivityGroup {
                count,
                percent,
                top,
            }
        };

        Ok(Some(AsConnectivitySummary {
            asn,
            upstreams: build_group(&upstreams, &names),
            peers: build_group(&peers_list, &names),
            downstreams: build_group(&downstreams, &names),
            total_neighbors: total as u32,
            max_peers_count,
        }))
    }

    /// Check if results would be truncated for connectivity summary
    pub fn would_truncate_connectivity(&self, asn: u32, top_n: usize) -> Result<bool> {
        let relationships = self.search_asn(asn)?;

        if relationships.is_empty() || top_n == 0 {
            return Ok(false);
        }

        let mut upstreams_count = 0;
        let mut peers_count = 0;
        let mut downstreams_count = 0;

        for rel in &relationships {
            let relationship_type = if rel.asn1 == asn { rel.rel } else { -rel.rel };

            match relationship_type {
                -1 => upstreams_count += 1,
                0 => peers_count += 1,
                1 => downstreams_count += 1,
                _ => {}
            }
        }

        Ok(upstreams_count > top_n || peers_count > top_n || downstreams_count > top_n)
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

    /// Find ASNs that are single-homed to a specific upstream provider
    ///
    /// A single-homed ASN has exactly one upstream provider.
    /// This finds all ASNs where `upstream_asn` is their ONLY upstream.
    ///
    /// # Arguments
    /// * `upstream_asn` - The upstream ASN to check against
    /// * `min_peers_pct` - Optional minimum visibility percentage (0-100)
    ///
    /// # Returns
    /// List of (customer_asn, peers_count, asn_name) tuples
    pub fn find_single_homed_to(
        &self,
        upstream_asn: u32,
        min_peers_pct: Option<f32>,
    ) -> Result<Vec<(u32, u32, Option<String>)>> {
        let max_peers = self.get_max_peers_count();
        let min_peers_count = min_peers_pct
            .map(|pct| ((pct / 100.0) * max_peers as f32) as u32)
            .unwrap_or(0);

        // Query to find single-homed ASNs:
        // 1. Find all ASNs that have the target ASN as upstream
        // 2. Filter to those that have exactly 1 upstream total
        let query = r#"
            WITH asn_upstreams AS (
                -- Normalize to (customer_asn, upstream_asn) pairs
                SELECT
                    CASE
                        WHEN rel = -1 THEN asn1
                        WHEN rel = 1 THEN asn2
                    END as customer_asn,
                    CASE
                        WHEN rel = -1 THEN asn2
                        WHEN rel = 1 THEN asn1
                    END as upstream_asn,
                    peers_count
                FROM as2rel
                WHERE rel IN (-1, 1)
            ),
            upstream_counts AS (
                SELECT
                    customer_asn,
                    COUNT(DISTINCT upstream_asn) as upstream_count
                FROM asn_upstreams
                GROUP BY customer_asn
            ),
            has_target_upstream AS (
                SELECT customer_asn, MAX(peers_count) as visibility
                FROM asn_upstreams
                WHERE upstream_asn = :upstream_asn
                GROUP BY customer_asn
            )
            SELECT
                h.customer_asn,
                h.visibility,
                COALESCE(
                    NULLIF(p.aka, ''),
                    NULLIF(p.name_long, ''),
                    NULLIF(p.name, ''),
                    NULLIF(ai.org_name, ''),
                    NULLIF(ai.name, ''),
                    NULLIF(o.org_name, ''),
                    NULLIF(o.as_name, ''),
                    c.name
                ) as asn_name
            FROM has_target_upstream h
            JOIN upstream_counts u ON h.customer_asn = u.customer_asn
            LEFT JOIN asinfo_core c ON c.asn = h.customer_asn
            LEFT JOIN asinfo_as2org ai ON ai.asn = h.customer_asn
            LEFT JOIN asinfo_peeringdb p ON p.asn = h.customer_asn
            LEFT JOIN as2org_all o ON o.asn = h.customer_asn
            WHERE u.upstream_count = 1
              AND h.visibility >= :min_peers
            ORDER BY h.visibility DESC
        "#;

        let mut stmt = self.conn.prepare(query)?;
        let rows = stmt
            .query_map(
                rusqlite::named_params! {
                    ":upstream_asn": upstream_asn,
                    ":min_peers": min_peers_count,
                },
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| anyhow!("Failed to find single-homed ASNs: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Count upstreams for a given ASN
    ///
    /// Returns the number of distinct upstream providers for the ASN.
    pub fn count_upstreams(&self, asn: u32) -> Result<u32> {
        let query = r#"
            SELECT COUNT(DISTINCT
                CASE
                    WHEN rel = -1 AND asn1 = :asn THEN asn2
                    WHEN rel = 1 AND asn2 = :asn THEN asn1
                END
            ) as upstream_count
            FROM as2rel
            WHERE (asn1 = :asn OR asn2 = :asn) AND rel IN (-1, 1)
        "#;

        let count: u32 = self
            .conn
            .query_row(query, rusqlite::named_params! { ":asn": asn }, |row| {
                row.get(0)
            })
            .map_err(|e| anyhow!("Failed to count upstreams: {}", e))?;

        Ok(count)
    }

    /// Search for relationships with a specific type filter
    ///
    /// # Arguments
    /// * `asn` - The ASN to query
    /// * `rel_type` - Relationship type: -1 (asn is customer), 0 (peers), 1 (asn is provider)
    pub fn search_asn_by_rel_type(&self, asn: u32, rel_type: i8) -> Result<Vec<As2relRecord>> {
        // When querying for asn's perspective:
        // - rel_type = -1: asn is customer, so (asn1=asn, rel=-1) OR (asn2=asn, rel=1)
        // - rel_type = 0: peers, so rel=0
        // - rel_type = 1: asn is provider, so (asn1=asn, rel=1) OR (asn2=asn, rel=-1)
        let query = match rel_type {
            -1 => {
                // ASN is downstream (customer) - looking for upstreams
                r#"
                    SELECT asn1, asn2, paths_count, peers_count, rel
                    FROM as2rel
                    WHERE (asn1 = ?1 AND rel = -1) OR (asn2 = ?1 AND rel = 1)
                "#
            }
            0 => {
                // Peer relationships
                r#"
                    SELECT asn1, asn2, paths_count, peers_count, rel
                    FROM as2rel
                    WHERE (asn1 = ?1 OR asn2 = ?1) AND rel = 0
                "#
            }
            1 => {
                // ASN is upstream (provider) - looking for downstreams
                r#"
                    SELECT asn1, asn2, paths_count, peers_count, rel
                    FROM as2rel
                    WHERE (asn1 = ?1 AND rel = 1) OR (asn2 = ?1 AND rel = -1)
                "#
            }
            _ => return Err(anyhow!("Invalid relationship type: {}", rel_type)),
        };

        let mut stmt = self.conn.prepare(query)?;
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
            .map_err(|e| anyhow!("Failed to search ASN by rel type: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Search for relationships with a specific type filter, with names
    ///
    /// Like `search_asn_with_names` but filtered by relationship type.
    pub fn search_asn_with_names_by_rel_type(
        &self,
        asn: u32,
        rel_type: i8,
    ) -> Result<Vec<AggregatedRelationship>> {
        // Get all relationships first, then filter
        let all_rels = self.search_asn_with_names(asn)?;

        // Filter based on relationship type
        // rel_type from ASN's perspective:
        // -1: ASN is downstream/customer (as2_upstream should be high)
        // 0: Peers (peer should be high)
        // 1: ASN is upstream/provider (as1_upstream should be high)
        let filtered: Vec<AggregatedRelationship> = all_rels
            .into_iter()
            .filter(|r| match rel_type {
                -1 => r.as2_upstream_count > 0 && r.as1_upstream_count == 0, // Other is upstream of ASN
                0 => {
                    r.connected_count > 0 && r.as1_upstream_count == 0 && r.as2_upstream_count == 0
                }
                1 => r.as1_upstream_count > 0 && r.as2_upstream_count == 0, // ASN is upstream of other
                _ => false,
            })
            .collect();

        Ok(filtered)
    }

    /// Search for all pairs among a list of ASNs
    ///
    /// Returns relationships for all pairs (asn_i, asn_j) where i < j.
    /// Results are sorted by asn1 ascending.
    pub fn search_multi_asn_pairs(&self, asns: &[u32]) -> Result<Vec<AggregatedRelationship>> {
        if asns.len() < 2 {
            return Ok(vec![]);
        }

        // Generate all unique pairs where asn1 < asn2
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for i in 0..asns.len() {
            for j in (i + 1)..asns.len() {
                let (a, b) = if asns[i] < asns[j] {
                    (asns[i], asns[j])
                } else {
                    (asns[j], asns[i])
                };
                if !pairs.contains(&(a, b)) {
                    pairs.push((a, b));
                }
            }
        }

        // Sort by first ASN
        pairs.sort_by_key(|(a, _)| *a);

        // Query each pair and collect results
        let mut results = Vec::new();
        for (asn1, asn2) in pairs {
            let pair_results = self.search_pair_with_names(asn1, asn2)?;
            for r in pair_results {
                // Ensure asn1 < asn2 in the result
                if r.asn1 <= r.asn2 {
                    results.push(r);
                } else {
                    // Swap perspective
                    results.push(AggregatedRelationship {
                        asn1: r.asn2,
                        asn2: r.asn1,
                        asn2_name: None, // We'd need to look up asn1's name
                        connected_count: r.connected_count,
                        as1_upstream_count: r.as2_upstream_count,
                        as2_upstream_count: r.as1_upstream_count,
                    });
                }
            }
        }

        Ok(results)
    }

    /// Search for all pairs among a list of ASNs with proper name lookups
    ///
    /// Returns relationships for all pairs (asn_i, asn_j) where i < j.
    /// Results are sorted by asn1 ascending.
    pub fn search_multi_asn_pairs_with_names(
        &self,
        asns: &[u32],
    ) -> Result<Vec<AggregatedRelationship>> {
        if asns.len() < 2 {
            return Ok(vec![]);
        }

        // Generate all unique pairs where asn1 < asn2
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for i in 0..asns.len() {
            for j in (i + 1)..asns.len() {
                let (a, b) = if asns[i] < asns[j] {
                    (asns[i], asns[j])
                } else {
                    (asns[j], asns[i])
                };
                if !pairs.contains(&(a, b)) {
                    pairs.push((a, b));
                }
            }
        }

        // Sort by first ASN
        pairs.sort_by_key(|(a, _)| *a);

        // Build a query for all pairs at once
        if pairs.is_empty() {
            return Ok(vec![]);
        }

        // Create WHERE clause for all pairs
        let pair_conditions: Vec<String> = pairs
            .iter()
            .map(|(a, b)| {
                format!(
                    "((r.asn1 = {} AND r.asn2 = {}) OR (r.asn1 = {} AND r.asn2 = {}))",
                    a, b, b, a
                )
            })
            .collect();
        let where_clause = pair_conditions.join(" OR ");

        let query = format!(
            r#"
            SELECT
                CASE WHEN r.asn1 < r.asn2 THEN r.asn1 ELSE r.asn2 END as asn1,
                CASE WHEN r.asn1 < r.asn2 THEN r.asn2 ELSE r.asn1 END as asn2,
                COALESCE(
                    NULLIF(p.aka, ''),
                    NULLIF(p.name_long, ''),
                    NULLIF(p.name, ''),
                    NULLIF(ai.org_name, ''),
                    NULLIF(ai.name, ''),
                    NULLIF(o.org_name, ''),
                    NULLIF(o.as_name, ''),
                    c.name
                ) as asn2_name,
                MAX(CASE WHEN r.rel = 0 THEN r.peers_count ELSE 0 END) as connected_count,
                SUM(CASE
                    WHEN r.asn1 < r.asn2 AND r.rel = 1 THEN r.peers_count
                    WHEN r.asn1 > r.asn2 AND r.rel = -1 THEN r.peers_count
                    ELSE 0
                END) as as1_upstream_count,
                SUM(CASE
                    WHEN r.asn1 < r.asn2 AND r.rel = -1 THEN r.peers_count
                    WHEN r.asn1 > r.asn2 AND r.rel = 1 THEN r.peers_count
                    ELSE 0
                END) as as2_upstream_count
            FROM as2rel r
            LEFT JOIN asinfo_core c
                ON c.asn = CASE WHEN r.asn1 < r.asn2 THEN r.asn2 ELSE r.asn1 END
            LEFT JOIN asinfo_as2org ai
                ON ai.asn = CASE WHEN r.asn1 < r.asn2 THEN r.asn2 ELSE r.asn1 END
            LEFT JOIN asinfo_peeringdb p
                ON p.asn = CASE WHEN r.asn1 < r.asn2 THEN r.asn2 ELSE r.asn1 END
            LEFT JOIN as2org_all o
                ON o.asn = CASE WHEN r.asn1 < r.asn2 THEN r.asn2 ELSE r.asn1 END
            WHERE {}
            GROUP BY
                CASE WHEN r.asn1 < r.asn2 THEN r.asn1 ELSE r.asn2 END,
                CASE WHEN r.asn1 < r.asn2 THEN r.asn2 ELSE r.asn1 END
            HAVING connected_count > 0 OR as1_upstream_count > 0 OR as2_upstream_count > 0
            ORDER BY asn1, asn2
        "#,
            where_clause
        );

        let mut stmt = self.conn.prepare(&query)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(AggregatedRelationship {
                    asn1: row.get(0)?,
                    asn2: row.get(1)?,
                    asn2_name: row.get(2)?,
                    connected_count: row.get(3)?,
                    as1_upstream_count: row.get(4)?,
                    as2_upstream_count: row.get(5)?,
                })
            })
            .map_err(|e| anyhow!("Failed to search multi-ASN pairs: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
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
