//! RPKI repository for the shared database
//!
//! This module provides SQLite-based storage for RPKI ROAs and ASPAs,
//! with support for caching current data and validating prefix-ASN pairs.
//!
//! # IP Address Storage
//!
//! IP prefixes are stored as two 16-byte columns (start and end addresses).
//! IPv4 addresses are converted to IPv6-mapped format (::ffff:x.x.x.x) for
//! uniform storage and comparison.
//!
//! # Validation Logic
//!
//! ROA validation follows RFC 6811:
//! - **Valid**: A covering ROA exists with matching ASN and valid prefix length
//! - **Invalid**: A covering ROA exists but ASN doesn't match or length exceeds max_length
//! - **NotFound**: No covering ROA exists for the prefix

use anyhow::{anyhow, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use tracing::info;

/// Default TTL for RPKI cache (24 hours)
pub const DEFAULT_RPKI_CACHE_TTL: Duration = Duration::hours(24);

/// RPKI validation state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpkiValidationState {
    /// A valid ROA covers this prefix-ASN pair
    Valid,
    /// A ROA exists but the ASN or prefix length is invalid
    Invalid,
    /// No ROA covers this prefix
    NotFound,
}

impl std::fmt::Display for RpkiValidationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpkiValidationState::Valid => write!(f, "valid"),
            RpkiValidationState::Invalid => write!(f, "invalid"),
            RpkiValidationState::NotFound => write!(f, "not-found"),
        }
    }
}

/// Detailed validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "display", derive(tabled::Tabled))]
pub struct RpkiValidationResult {
    pub prefix: String,
    pub asn: u32,
    pub state: String,
    pub reason: String,
}

/// ROA record for database storage
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "display", derive(tabled::Tabled))]
pub struct RpkiRoaRecord {
    pub prefix: String,
    pub max_length: u8,
    pub origin_asn: u32,
    pub ta: String,
}

/// ASPA record for database storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiAspaRecord {
    pub customer_asn: u32,
    pub provider_asns: Vec<u32>,
}

/// ASPA provider with enriched name information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiAspaProviderEnriched {
    pub asn: u32,
    pub name: Option<String>,
}

/// Enriched ASPA record with customer and provider names from asinfo
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiAspaEnrichedRecord {
    pub customer_asn: u32,
    pub customer_name: Option<String>,
    pub customer_country: Option<String>,
    pub providers: Vec<RpkiAspaProviderEnriched>,
}

/// RPKI cache metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiCacheMetadata {
    pub updated_at: DateTime<Utc>,
    pub roa_count: u64,
    pub aspa_count: u64,
    /// Source of ROA data (e.g., "Cloudflare" or "RTR (rtr.rpki.cloudflare.com:8282)")
    pub roa_source: String,
    /// Source of ASPA data (currently always "Cloudflare")
    pub aspa_source: String,
}

impl RpkiCacheMetadata {
    /// Format the data source information for display.
    ///
    /// Returns a single source name if ROA and ASPA sources are the same,
    /// otherwise returns "ROAs from X, ASPAs from Y".
    pub fn format_source(&self) -> String {
        if self.roa_source == self.aspa_source {
            self.roa_source.clone()
        } else {
            format!(
                "ROAs from {}, ASPAs from {}",
                self.roa_source, self.aspa_source
            )
        }
    }
}

/// SQL schema definitions for RPKI tables
pub struct RpkiSchemaDefinitions;

impl RpkiSchemaDefinitions {
    /// SQL for creating the RPKI ROA table
    pub const RPKI_ROA_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS rpki_roa (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            prefix_start BLOB NOT NULL,
            prefix_end BLOB NOT NULL,
            prefix_length INTEGER NOT NULL,
            max_length INTEGER NOT NULL,
            origin_asn INTEGER NOT NULL,
            ta TEXT NOT NULL,
            prefix_str TEXT NOT NULL
        );
    "#;

    /// SQL for creating the RPKI ASPA table
    pub const RPKI_ASPA_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS rpki_aspa (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            customer_asn INTEGER NOT NULL,
            provider_asn INTEGER NOT NULL
        );
    "#;

    /// SQL for creating the RPKI meta table
    pub const RPKI_META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS rpki_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            updated_at INTEGER NOT NULL,
            roa_count INTEGER NOT NULL DEFAULT 0,
            aspa_count INTEGER NOT NULL DEFAULT 0,
            roa_source TEXT NOT NULL DEFAULT 'Cloudflare',
            aspa_source TEXT NOT NULL DEFAULT 'Cloudflare'
        );
    "#;

    /// SQL for creating RPKI indexes
    pub const RPKI_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_rpki_roa_prefix_range ON rpki_roa(prefix_start, prefix_end)",
        "CREATE INDEX IF NOT EXISTS idx_rpki_roa_origin_asn ON rpki_roa(origin_asn)",
        "CREATE INDEX IF NOT EXISTS idx_rpki_aspa_customer ON rpki_aspa(customer_asn)",
        "CREATE INDEX IF NOT EXISTS idx_rpki_aspa_provider ON rpki_aspa(provider_asn)",
    ];
}

/// Repository for RPKI data operations
pub struct RpkiRepository<'a> {
    conn: &'a Connection,
}

impl<'a> RpkiRepository<'a> {
    /// Create a new RPKI repository
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Initialize the RPKI schema (create tables if not exist)
    pub fn initialize_schema(&self) -> Result<()> {
        self.conn
            .execute(RpkiSchemaDefinitions::RPKI_ROA_TABLE, [])
            .map_err(|e| anyhow!("Failed to create rpki_roa table: {}", e))?;

        self.conn
            .execute(RpkiSchemaDefinitions::RPKI_ASPA_TABLE, [])
            .map_err(|e| anyhow!("Failed to create rpki_aspa table: {}", e))?;

        self.conn
            .execute(RpkiSchemaDefinitions::RPKI_META_TABLE, [])
            .map_err(|e| anyhow!("Failed to create rpki_meta table: {}", e))?;

        // Migration: add source columns if they don't exist (for existing databases)
        self.migrate_add_source_columns();

        for index_sql in RpkiSchemaDefinitions::RPKI_INDEXES {
            self.conn
                .execute(index_sql, [])
                .map_err(|e| anyhow!("Failed to create RPKI index: {}", e))?;
        }

        Ok(())
    }

    /// Migrate: add roa_source and aspa_source columns if they don't exist
    fn migrate_add_source_columns(&self) {
        // Check if columns exist by querying table info
        let has_roa_source: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('rpki_meta') WHERE name='roa_source'",
                [],
                |row| row.get::<_, i32>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_roa_source {
            // Add the new columns with default values
            let _ = self.conn.execute(
                "ALTER TABLE rpki_meta ADD COLUMN roa_source TEXT NOT NULL DEFAULT 'Cloudflare'",
                [],
            );
            let _ = self.conn.execute(
                "ALTER TABLE rpki_meta ADD COLUMN aspa_source TEXT NOT NULL DEFAULT 'Cloudflare'",
                [],
            );
        }
    }

    /// Check if RPKI tables exist
    pub fn tables_exist(&self) -> bool {
        let exists: i32 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='rpki_roa'",
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
        let count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM rpki_roa", [], |row| row.get(0))
            .unwrap_or(0);
        count == 0
    }

    /// Check if the cache needs refresh (expired or empty)
    pub fn needs_refresh(&self, ttl: Duration) -> bool {
        if !self.tables_exist() || self.is_empty() {
            return true;
        }

        match self.get_metadata() {
            Ok(Some(meta)) => {
                let now = Utc::now();
                now.signed_duration_since(meta.updated_at) > ttl
            }
            _ => true,
        }
    }

    /// Get cache metadata
    pub fn get_metadata(&self) -> Result<Option<RpkiCacheMetadata>> {
        if !self.tables_exist() {
            return Ok(None);
        }

        // Ensure migration has run to add source columns for older databases
        self.migrate_add_source_columns();

        let result = self.conn.query_row(
            "SELECT updated_at, roa_count, aspa_count, roa_source, aspa_source FROM rpki_meta WHERE id = 1",
            [],
            |row| {
                let ts: i64 = row.get(0)?;
                let roa_count: u64 = row.get(1)?;
                let aspa_count: u64 = row.get(2)?;
                let roa_source: String = row.get(3).unwrap_or_else(|_| "Cloudflare".to_string());
                let aspa_source: String = row.get(4).unwrap_or_else(|_| "Cloudflare".to_string());
                Ok(RpkiCacheMetadata {
                    updated_at: DateTime::from_timestamp(ts, 0).unwrap_or_default(),
                    roa_count,
                    aspa_count,
                    roa_source,
                    aspa_source,
                })
            },
        );

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get RPKI metadata: {}", e)),
        }
    }

    /// Clear all RPKI data
    pub fn clear(&self) -> Result<()> {
        if !self.tables_exist() {
            return Ok(());
        }

        self.conn
            .execute("DELETE FROM rpki_roa", [])
            .map_err(|e| anyhow!("Failed to clear rpki_roa: {}", e))?;

        self.conn
            .execute("DELETE FROM rpki_aspa", [])
            .map_err(|e| anyhow!("Failed to clear rpki_aspa: {}", e))?;

        self.conn
            .execute("DELETE FROM rpki_meta", [])
            .map_err(|e| anyhow!("Failed to clear rpki_meta: {}", e))?;

        Ok(())
    }

    /// Store ROAs and ASPAs in the database
    ///
    /// Uses optimized batch insert with:
    /// - Disabled synchronous writes for performance
    /// - Memory-based journal mode
    /// - Single transaction for all inserts
    ///
    /// # Arguments
    /// * `roas` - ROA records to store
    /// * `aspas` - ASPA records to store
    /// * `roa_source` - Source of ROA data (e.g., "Cloudflare" or "RTR (host:port)")
    /// * `aspa_source` - Source of ASPA data (e.g., "Cloudflare")
    pub fn store(
        &self,
        roas: &[RpkiRoaRecord],
        aspas: &[RpkiAspaRecord],
        roa_source: &str,
        aspa_source: &str,
    ) -> Result<()> {
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

        // Insert ROAs in batches
        let mut roa_stmt = self.conn.prepare(
            "INSERT INTO rpki_roa (prefix_start, prefix_end, prefix_length, max_length, origin_asn, ta, prefix_str)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        let mut roa_inserted = 0usize;
        for roa in roas {
            if let Ok((start, end, prefix_len)) = parse_prefix_to_range(&roa.prefix) {
                roa_stmt.execute(params![
                    start.as_slice(),
                    end.as_slice(),
                    prefix_len,
                    roa.max_length,
                    roa.origin_asn,
                    roa.ta,
                    roa.prefix,
                ])?;
                roa_inserted += 1;
            }
        }

        // Insert ASPAs (one row per customer-provider pair)
        let mut aspa_stmt = self
            .conn
            .prepare("INSERT INTO rpki_aspa (customer_asn, provider_asn) VALUES (?1, ?2)")?;

        let mut aspa_pairs_inserted = 0usize;
        for aspa in aspas {
            for provider in &aspa.provider_asns {
                aspa_stmt.execute(params![aspa.customer_asn, provider])?;
                aspa_pairs_inserted += 1;
            }
        }

        // Update metadata
        let now = Utc::now().timestamp();
        self.conn.execute(
            "INSERT OR REPLACE INTO rpki_meta (id, updated_at, roa_count, aspa_count, roa_source, aspa_source) VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![now, roa_inserted, aspas.len(), roa_source, aspa_source],
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
            "Stored {} ROAs and {} ASPA customer-provider pairs ({} customers) in RPKI database",
            roa_inserted,
            aspa_pairs_inserted,
            aspas.len()
        );

        Ok(())
    }

    /// Get all ROAs
    pub fn get_all_roas(&self) -> Result<Vec<RpkiRoaRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let mut stmt = self
            .conn
            .prepare("SELECT prefix_str, max_length, origin_asn, ta FROM rpki_roa")?;

        let rows = stmt.query_map([], |row| {
            Ok(RpkiRoaRecord {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Get ROAs filtered by origin ASN
    pub fn get_roas_by_asn(&self, asn: u32) -> Result<Vec<RpkiRoaRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let mut stmt = self.conn.prepare(
            "SELECT prefix_str, max_length, origin_asn, ta FROM rpki_roa WHERE origin_asn = ?1",
        )?;

        let rows = stmt.query_map([asn], |row| {
            Ok(RpkiRoaRecord {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Get ROAs that cover a given prefix
    pub fn get_covering_roas(&self, prefix: &str) -> Result<Vec<RpkiRoaRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let (addr_bytes, _, _) = parse_prefix_to_range(prefix)?;

        // Find ROAs where the prefix start <= query address <= prefix end
        let mut stmt = self.conn.prepare(
            "SELECT prefix_str, max_length, origin_asn, ta FROM rpki_roa
             WHERE prefix_start <= ?1 AND prefix_end >= ?1",
        )?;

        let rows = stmt.query_map([addr_bytes.as_slice()], |row| {
            Ok(RpkiRoaRecord {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Get all ASPAs
    pub fn get_all_aspas(&self) -> Result<Vec<RpkiAspaRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let mut stmt = self.conn.prepare(
            "SELECT customer_asn, GROUP_CONCAT(provider_asn) as providers
             FROM rpki_aspa GROUP BY customer_asn",
        )?;

        let rows = stmt.query_map([], |row| {
            let customer_asn: u32 = row.get(0)?;
            let providers_str: String = row.get(1)?;
            let provider_asns: Vec<u32> = providers_str
                .split(',')
                .filter_map(|s| s.parse().ok())
                .collect();
            Ok(RpkiAspaRecord {
                customer_asn,
                provider_asns,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Get ASPAs filtered by customer ASN
    pub fn get_aspas_by_customer(&self, customer_asn: u32) -> Result<Vec<RpkiAspaRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let mut stmt = self.conn.prepare(
            "SELECT customer_asn, GROUP_CONCAT(provider_asn) as providers
             FROM rpki_aspa WHERE customer_asn = ?1 GROUP BY customer_asn",
        )?;

        let rows = stmt.query_map([customer_asn], |row| {
            let customer_asn: u32 = row.get(0)?;
            let providers_str: String = row.get(1)?;
            let provider_asns: Vec<u32> = providers_str
                .split(',')
                .filter_map(|s| s.parse().ok())
                .collect();
            Ok(RpkiAspaRecord {
                customer_asn,
                provider_asns,
            })
        })?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Get ASPAs filtered by provider ASN
    pub fn get_aspas_by_provider(&self, provider_asn: u32) -> Result<Vec<RpkiAspaRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT customer_asn FROM rpki_aspa WHERE provider_asn = ?1")?;

        let customer_asns: Vec<u32> = stmt
            .query_map([provider_asn], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut results = Vec::new();
        for customer_asn in customer_asns {
            let aspas = self.get_aspas_by_customer(customer_asn)?;
            results.extend(aspas);
        }

        Ok(results)
    }

    /// Get all ASPAs with enriched customer and provider names (via SQL JOINs)
    pub fn get_all_aspas_enriched(&self) -> Result<Vec<RpkiAspaEnrichedRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        // Query that joins with asinfo tables for preferred customer info
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                a.customer_asn,
                COALESCE(
                    NULLIF(pc.aka, ''),
                    NULLIF(pc.name_long, ''),
                    NULLIF(pc.name, ''),
                    NULLIF(ac.org_name, ''),
                    NULLIF(ac.name, ''),
                    c.name
                ) as customer_name,
                c.country as customer_country,
                GROUP_CONCAT(a.provider_asn) as provider_asns
            FROM rpki_aspa a
            LEFT JOIN asinfo_core c ON a.customer_asn = c.asn
            LEFT JOIN asinfo_as2org ac ON a.customer_asn = ac.asn
            LEFT JOIN asinfo_peeringdb pc ON a.customer_asn = pc.asn
            GROUP BY a.customer_asn
            ORDER BY a.customer_asn
            "#,
        )?;

        let customer_rows: Vec<(u32, Option<String>, Option<String>, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Now get provider names with a second query (preferred name)
        let mut provider_stmt = self.conn.prepare(
            r#"
            SELECT
                a.customer_asn,
                a.provider_asn,
                COALESCE(
                    NULLIF(pp.aka, ''),
                    NULLIF(pp.name_long, ''),
                    NULLIF(pp.name, ''),
                    NULLIF(ap.org_name, ''),
                    NULLIF(ap.name, ''),
                    c.name
                ) as provider_name
            FROM rpki_aspa a
            LEFT JOIN asinfo_core c ON a.provider_asn = c.asn
            LEFT JOIN asinfo_as2org ap ON a.provider_asn = ap.asn
            LEFT JOIN asinfo_peeringdb pp ON a.provider_asn = pp.asn
            ORDER BY a.customer_asn, a.provider_asn
            "#,
        )?;

        let provider_rows: Vec<(u32, u32, Option<String>)> = provider_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Build a map of customer_asn -> providers
        let mut provider_map: std::collections::HashMap<u32, Vec<RpkiAspaProviderEnriched>> =
            std::collections::HashMap::new();
        for (customer_asn, provider_asn, provider_name) in provider_rows {
            provider_map
                .entry(customer_asn)
                .or_default()
                .push(RpkiAspaProviderEnriched {
                    asn: provider_asn,
                    name: provider_name,
                });
        }

        // Build the final results
        let results: Vec<RpkiAspaEnrichedRecord> = customer_rows
            .into_iter()
            .map(|(customer_asn, customer_name, customer_country, _)| {
                let providers = provider_map.remove(&customer_asn).unwrap_or_default();
                RpkiAspaEnrichedRecord {
                    customer_asn,
                    customer_name,
                    customer_country,
                    providers,
                }
            })
            .collect();

        Ok(results)
    }

    /// Get ASPAs filtered by customer ASN with enriched names (via SQL JOINs)
    pub fn get_aspas_by_customer_enriched(
        &self,
        customer_asn: u32,
    ) -> Result<Vec<RpkiAspaEnrichedRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        // Get customer info (preferred name)
        let mut customer_stmt = self.conn.prepare(
            r#"
            SELECT
                a.customer_asn,
                COALESCE(
                    NULLIF(pc.aka, ''),
                    NULLIF(pc.name_long, ''),
                    NULLIF(pc.name, ''),
                    NULLIF(ac.org_name, ''),
                    NULLIF(ac.name, ''),
                    c.name
                ) as customer_name,
                c.country as customer_country
            FROM rpki_aspa a
            LEFT JOIN asinfo_core c ON a.customer_asn = c.asn
            LEFT JOIN asinfo_as2org ac ON a.customer_asn = ac.asn
            LEFT JOIN asinfo_peeringdb pc ON a.customer_asn = pc.asn
            WHERE a.customer_asn = ?1
            LIMIT 1
            "#,
        )?;

        let customer_info: Option<(u32, Option<String>, Option<String>)> = customer_stmt
            .query_row([customer_asn], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .ok();

        let Some((customer_asn, customer_name, customer_country)) = customer_info else {
            return Ok(Vec::new());
        };

        // Get providers with names (preferred name)
        let mut provider_stmt = self.conn.prepare(
            r#"
            SELECT
                a.provider_asn,
                COALESCE(
                    NULLIF(pp.aka, ''),
                    NULLIF(pp.name_long, ''),
                    NULLIF(pp.name, ''),
                    NULLIF(ap.org_name, ''),
                    NULLIF(ap.name, ''),
                    c.name
                ) as provider_name
            FROM rpki_aspa a
            LEFT JOIN asinfo_core c ON a.provider_asn = c.asn
            LEFT JOIN asinfo_as2org ap ON a.provider_asn = ap.asn
            LEFT JOIN asinfo_peeringdb pp ON a.provider_asn = pp.asn
            WHERE a.customer_asn = ?1
            ORDER BY a.provider_asn
            "#,
        )?;

        let providers: Vec<RpkiAspaProviderEnriched> = provider_stmt
            .query_map([customer_asn], |row| {
                Ok(RpkiAspaProviderEnriched {
                    asn: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(vec![RpkiAspaEnrichedRecord {
            customer_asn,
            customer_name,
            customer_country,
            providers,
        }])
    }

    /// Get ASPAs filtered by provider ASN with enriched names (via SQL JOINs)
    pub fn get_aspas_by_provider_enriched(
        &self,
        provider_asn: u32,
    ) -> Result<Vec<RpkiAspaEnrichedRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        // Get all customer ASNs that have this provider
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT customer_asn FROM rpki_aspa WHERE provider_asn = ?1")?;

        let customer_asns: Vec<u32> = stmt
            .query_map([provider_asn], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut results = Vec::new();
        for customer_asn in customer_asns {
            let aspas = self.get_aspas_by_customer_enriched(customer_asn)?;
            results.extend(aspas);
        }

        Ok(results)
    }

    // =========================================================================
    // RPKI Validation
    // =========================================================================

    /// Validate a prefix-ASN pair against the cached ROAs
    ///
    /// Returns the validation state and a list of covering ROAs.
    ///
    /// Validation logic (RFC 6811):
    /// - **Valid**: A covering ROA exists with matching ASN and the announced
    ///   prefix length is <= max_length
    /// - **Invalid**: A covering ROA exists but either:
    ///   - The ASN doesn't match (unauthorized AS)
    ///   - The prefix length exceeds max_length (length violation)
    /// - **NotFound**: No covering ROA exists for the prefix
    pub fn validate(
        &self,
        prefix: &str,
        asn: u32,
    ) -> Result<(RpkiValidationState, Vec<RpkiRoaRecord>)> {
        let covering_roas = self.get_covering_roas_for_validation(prefix)?;

        if covering_roas.is_empty() {
            return Ok((RpkiValidationState::NotFound, Vec::new()));
        }

        // Parse the query prefix to get its length
        let query_prefix_len = parse_prefix_length(prefix)?;

        // Check if any ROA makes this valid
        for roa in &covering_roas {
            if roa.origin_asn == asn {
                // Check if prefix length is within max_length
                if query_prefix_len <= roa.max_length {
                    return Ok((RpkiValidationState::Valid, covering_roas));
                }
            }
        }

        // If we found matching ASN but length was too long, it's invalid (length violation)
        // If no matching ASN, it's invalid (unauthorized AS)
        Ok((RpkiValidationState::Invalid, covering_roas))
    }

    /// Validate and return detailed result
    pub fn validate_detailed(&self, prefix: &str, asn: u32) -> Result<RpkiValidationResult> {
        let (state, covering_roas) = self.validate(prefix, asn)?;

        let reason = match state {
            RpkiValidationState::Valid => {
                "ROA exists with matching ASN and valid prefix length".to_string()
            }
            RpkiValidationState::Invalid => {
                let query_prefix_len = parse_prefix_length(prefix).unwrap_or(0);
                let has_matching_asn = covering_roas.iter().any(|r| r.origin_asn == asn);

                if has_matching_asn {
                    format!(
                        "Prefix length {} exceeds max_length in covering ROAs",
                        query_prefix_len
                    )
                } else {
                    let authorized_asns: Vec<String> = covering_roas
                        .iter()
                        .map(|r| r.origin_asn.to_string())
                        .collect();
                    format!(
                        "ASN {} not authorized; authorized ASNs: {}",
                        asn,
                        authorized_asns.join(", ")
                    )
                }
            }
            RpkiValidationState::NotFound => "No covering ROA found".to_string(),
        };

        Ok(RpkiValidationResult {
            prefix: prefix.to_string(),
            asn,
            state: state.to_string(),
            reason,
        })
    }

    /// Get covering ROAs for validation (internal helper)
    ///
    /// This finds all ROAs where the ROA's prefix covers the query prefix.
    /// A ROA covers a prefix if:
    /// - The ROA prefix contains the query prefix's network address
    /// - The ROA prefix length is <= the query prefix length
    fn get_covering_roas_for_validation(&self, prefix: &str) -> Result<Vec<RpkiRoaRecord>> {
        if !self.tables_exist() {
            return Ok(Vec::new());
        }

        let (start_bytes, end_bytes, query_prefix_len) = parse_prefix_to_range(prefix)?;

        // Find ROAs where:
        // 1. The ROA's prefix range contains the query prefix range
        // 2. The ROA's prefix length is <= query prefix length (ROA is less specific or equal)
        let mut stmt = self.conn.prepare(
            "SELECT prefix_str, max_length, origin_asn, ta, prefix_length FROM rpki_roa
             WHERE prefix_start <= ?1 AND prefix_end >= ?2 AND prefix_length <= ?3",
        )?;

        let rows = stmt.query_map(
            params![
                start_bytes.as_slice(),
                end_bytes.as_slice(),
                query_prefix_len
            ],
            |row| {
                Ok(RpkiRoaRecord {
                    prefix: row.get(0)?,
                    max_length: row.get(1)?,
                    origin_asn: row.get(2)?,
                    ta: row.get(3)?,
                })
            },
        )?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }

        Ok(results)
    }

    /// Get ROA count
    pub fn roa_count(&self) -> Result<u64> {
        if !self.tables_exist() {
            return Ok(0);
        }
        let count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM rpki_roa", [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to get ROA count: {}", e))?;
        Ok(count)
    }

    /// Get ASPA count (unique customer ASNs)
    pub fn aspa_count(&self) -> Result<u64> {
        if !self.tables_exist() {
            return Ok(0);
        }
        let count: u64 = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT customer_asn) FROM rpki_aspa",
                [],
                |row| row.get(0),
            )
            .map_err(|e| anyhow!("Failed to get ASPA count: {}", e))?;
        Ok(count)
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
    let net: ipnet::IpNet = prefix
        .parse()
        .map_err(|e| anyhow!("Invalid prefix '{}': {}", prefix, e))?;

    let start = ip_to_bytes(net.network());
    let end = ip_to_bytes(net.broadcast());
    let prefix_len = net.prefix_len();

    Ok((start, end, prefix_len))
}

/// Parse prefix length from a prefix string
fn parse_prefix_length(prefix: &str) -> Result<u8> {
    let net: ipnet::IpNet = prefix
        .parse()
        .map_err(|e| anyhow!("Invalid prefix '{}': {}", prefix, e))?;

    Ok(net.prefix_len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("PRAGMA foreign_keys=ON", []).unwrap();
        conn
    }

    #[test]
    fn test_schema_initialization() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        assert!(!repo.tables_exist());
        repo.initialize_schema().unwrap();
        assert!(repo.tables_exist());
    }

    #[test]
    fn test_store_and_retrieve_roas() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![
            RpkiRoaRecord {
                prefix: "1.0.0.0/24".to_string(),
                max_length: 24,
                origin_asn: 13335,
                ta: "apnic".to_string(),
            },
            RpkiRoaRecord {
                prefix: "2001:db8::/32".to_string(),
                max_length: 48,
                origin_asn: 64496,
                ta: "ripe".to_string(),
            },
        ];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        let retrieved = repo.get_all_roas().unwrap();
        assert_eq!(retrieved.len(), 2);

        let by_asn = repo.get_roas_by_asn(13335).unwrap();
        assert_eq!(by_asn.len(), 1);
        assert_eq!(by_asn[0].prefix, "1.0.0.0/24");
    }

    #[test]
    fn test_store_and_retrieve_aspas() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let aspas = vec![
            RpkiAspaRecord {
                customer_asn: 64496,
                provider_asns: vec![64497, 64498],
            },
            RpkiAspaRecord {
                customer_asn: 64499,
                provider_asns: vec![64497],
            },
        ];

        repo.store(&[], &aspas, "Cloudflare", "Cloudflare").unwrap();

        let retrieved = repo.get_all_aspas().unwrap();
        assert_eq!(retrieved.len(), 2);

        let by_customer = repo.get_aspas_by_customer(64496).unwrap();
        assert_eq!(by_customer.len(), 1);
        assert_eq!(by_customer[0].provider_asns.len(), 2);

        let by_provider = repo.get_aspas_by_provider(64497).unwrap();
        assert_eq!(by_provider.len(), 2);
    }

    #[test]
    fn test_metadata() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        assert!(repo.get_metadata().unwrap().is_none());

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        let meta = repo.get_metadata().unwrap().unwrap();
        assert_eq!(meta.roa_count, 1);
        assert_eq!(meta.aspa_count, 0);
    }

    #[test]
    fn test_validation_valid() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        let (state, covering) = repo.validate("1.0.0.0/24", 13335).unwrap();
        assert_eq!(state, RpkiValidationState::Valid);
        assert_eq!(covering.len(), 1);
    }

    #[test]
    fn test_validation_invalid_asn() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        // Wrong ASN
        let (state, _) = repo.validate("1.0.0.0/24", 99999).unwrap();
        assert_eq!(state, RpkiValidationState::Invalid);
    }

    #[test]
    fn test_validation_invalid_length() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        // More specific than max_length allows
        let (state, _) = repo.validate("1.0.0.0/25", 13335).unwrap();
        assert_eq!(state, RpkiValidationState::Invalid);
    }

    #[test]
    fn test_validation_not_found() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        // Prefix not covered by any ROA
        let (state, covering) = repo.validate("2.0.0.0/24", 13335).unwrap();
        assert_eq!(state, RpkiValidationState::NotFound);
        assert!(covering.is_empty());
    }

    #[test]
    fn test_validation_with_max_length() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        // ROA allows /24 to /26
        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 26,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        // /25 is valid (within max_length)
        let (state, _) = repo.validate("1.0.0.0/25", 13335).unwrap();
        assert_eq!(state, RpkiValidationState::Valid);

        // /26 is valid (within max_length)
        let (state, _) = repo.validate("1.0.0.0/26", 13335).unwrap();
        assert_eq!(state, RpkiValidationState::Valid);

        // /27 is invalid (exceeds max_length)
        let (state, _) = repo.validate("1.0.0.0/27", 13335).unwrap();
        assert_eq!(state, RpkiValidationState::Invalid);
    }

    #[test]
    fn test_needs_refresh() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        // Empty cache needs refresh
        assert!(repo.needs_refresh(DEFAULT_RPKI_CACHE_TTL));

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        // Just stored, should not need refresh
        assert!(!repo.needs_refresh(DEFAULT_RPKI_CACHE_TTL));

        // With zero TTL, should need refresh
        assert!(repo.needs_refresh(Duration::zero()));
    }

    #[test]
    fn test_ipv6_prefix() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![RpkiRoaRecord {
            prefix: "2001:db8::/32".to_string(),
            max_length: 48,
            origin_asn: 64496,
            ta: "ripe".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        // Valid: exact match
        let (state, _) = repo.validate("2001:db8::/32", 64496).unwrap();
        assert_eq!(state, RpkiValidationState::Valid);

        // Valid: more specific within max_length
        let (state, _) = repo.validate("2001:db8:1::/48", 64496).unwrap();
        assert_eq!(state, RpkiValidationState::Valid);

        // Invalid: too specific
        let (state, _) = repo.validate("2001:db8:1:1::/64", 64496).unwrap();
        assert_eq!(state, RpkiValidationState::Invalid);

        // Not found: different prefix
        let (state, _) = repo.validate("2001:db9::/32", 64496).unwrap();
        assert_eq!(state, RpkiValidationState::NotFound);
    }

    #[test]
    fn test_clear() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();
        assert!(!repo.is_empty());

        repo.clear().unwrap();
        assert!(repo.is_empty());
    }

    #[test]
    fn test_validate_detailed() {
        let conn = create_test_db();
        let repo = RpkiRepository::new(&conn);

        let roas = vec![RpkiRoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: "apnic".to_string(),
        }];

        repo.store(&roas, &[], "Cloudflare", "Cloudflare").unwrap();

        // Valid
        let result = repo.validate_detailed("1.0.0.0/24", 13335).unwrap();
        assert_eq!(result.state, "valid");

        // Invalid - wrong ASN
        let result = repo.validate_detailed("1.0.0.0/24", 99999).unwrap();
        assert_eq!(result.state, "invalid");
        assert!(result.reason.contains("not authorized"));

        // Not found
        let result = repo.validate_detailed("2.0.0.0/24", 13335).unwrap();
        assert_eq!(result.state, "not-found");
    }
}
