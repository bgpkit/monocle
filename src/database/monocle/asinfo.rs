//! ASInfo repository for AS information lookup
//!
//! This module provides data access operations for AS information from multiple sources:
//! - Core AS info (name, country) - always populated
//! - AS2Org data (organization mapping from CAIDA)
//! - PeeringDB data (network information)
//! - Hegemony scores (from IHR)
//! - Population estimates (from APNIC)
//!
//! Data is loaded from a JSONL file at http://spaces.bgpkit.org/broker/asninfo.jsonl

use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::BufRead;
use std::time::Duration;
use tracing::info;

/// Default URL for the ASInfo JSONL data
pub const ASINFO_DATA_URL: &str = "http://spaces.bgpkit.org/broker/asninfo.jsonl";

/// Default TTL for ASInfo data (7 days)
pub const DEFAULT_ASINFO_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Repository for ASInfo data operations
pub struct AsinfoRepository<'a> {
    conn: &'a Connection,
}

// =============================================================================
// Record Types
// =============================================================================

/// Core AS information (always present)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoCoreRecord {
    pub asn: u32,
    pub name: String,
    pub country: String,
}

/// AS2Org data from CAIDA
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoAs2orgRecord {
    pub asn: u32,
    pub name: String,
    pub org_id: String,
    pub org_name: String,
    pub country: String,
}

/// PeeringDB network information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoPeeringdbRecord {
    pub asn: u32,
    pub name: String,
    pub name_long: Option<String>,
    pub aka: Option<String>,
    pub website: Option<String>,
    pub irr_as_set: Option<String>,
}

/// IHR AS Hegemony scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoHegemonyRecord {
    pub asn: u32,
    pub ipv4: f64,
    pub ipv6: f64,
}

/// APNIC Population estimates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoPopulationRecord {
    pub asn: u32,
    /// Percentage of country's users (0.0 - 100.0)
    pub percent_country: f64,
    /// Percentage of global users (0.0 - 100.0)
    pub percent_global: f64,
    pub sample_count: u32,
    pub user_count: u32,
}

/// Complete AS information (joined from all tables)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoFullRecord {
    pub core: AsinfoCoreRecord,
    pub as2org: Option<AsinfoAs2orgRecord>,
    pub peeringdb: Option<AsinfoPeeringdbRecord>,
    pub hegemony: Option<AsinfoHegemonyRecord>,
    pub population: Option<AsinfoPopulationRecord>,
}

/// Metadata about stored ASInfo data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoMetadata {
    pub source_url: String,
    pub last_updated: i64,
    pub core_count: u32,
    pub as2org_count: u32,
    pub peeringdb_count: u32,
    pub hegemony_count: u32,
    pub population_count: u32,
}

/// Counts of records stored per table
#[derive(Debug, Clone, Default)]
pub struct AsinfoStoreCounts {
    pub core: usize,
    pub as2org: usize,
    pub peeringdb: usize,
    pub hegemony: usize,
    pub population: usize,
}

// =============================================================================
// JSONL Input Types (for parsing the source data)
// =============================================================================

/// Raw record from the JSONL source file
#[derive(Debug, Clone, Deserialize)]
pub struct JsonlRecord {
    pub asn: u32,
    pub name: String,
    pub country: String,
    #[serde(default)]
    pub as2org: Option<JsonlAs2org>,
    #[serde(default)]
    pub peeringdb: Option<JsonlPeeringdb>,
    #[serde(default)]
    pub hegemony: Option<JsonlHegemony>,
    #[serde(default)]
    pub population: Option<JsonlPopulation>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonlAs2org {
    pub country: String,
    pub name: String,
    pub org_id: String,
    pub org_name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonlPeeringdb {
    #[serde(default)]
    pub aka: Option<String>,
    pub asn: u32,
    #[serde(default)]
    pub irr_as_set: Option<String>,
    pub name: String,
    #[serde(default)]
    pub name_long: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonlHegemony {
    pub asn: u32,
    pub ipv4: f64,
    pub ipv6: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonlPopulation {
    pub percent_country: f64,
    pub percent_global: f64,
    pub sample_count: u32,
    pub user_count: u32,
}

// =============================================================================
// Schema Definitions
// =============================================================================

/// SQL schema definitions for ASInfo tables
pub struct AsinfoSchemaDefinitions;

impl AsinfoSchemaDefinitions {
    /// Core AS table (always populated)
    pub const ASINFO_CORE_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS asinfo_core (
            asn INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            country TEXT NOT NULL
        );
    "#;

    /// AS2Org data (from CAIDA)
    pub const ASINFO_AS2ORG_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS asinfo_as2org (
            asn INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            org_id TEXT NOT NULL,
            org_name TEXT NOT NULL,
            country TEXT NOT NULL
        );
    "#;

    /// PeeringDB data
    pub const ASINFO_PEERINGDB_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS asinfo_peeringdb (
            asn INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            name_long TEXT,
            aka TEXT,
            website TEXT,
            irr_as_set TEXT
        );
    "#;

    /// IHR Hegemony scores
    pub const ASINFO_HEGEMONY_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS asinfo_hegemony (
            asn INTEGER PRIMARY KEY,
            ipv4 REAL NOT NULL,
            ipv6 REAL NOT NULL
        );
    "#;

    /// APNIC Population estimates
    pub const ASINFO_POPULATION_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS asinfo_population (
            asn INTEGER PRIMARY KEY,
            percent_country REAL NOT NULL,
            percent_global REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            user_count INTEGER NOT NULL
        );
    "#;

    /// Metadata table
    pub const ASINFO_META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS asinfo_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            source_url TEXT NOT NULL,
            last_updated INTEGER NOT NULL,
            core_count INTEGER NOT NULL,
            as2org_count INTEGER NOT NULL,
            peeringdb_count INTEGER NOT NULL,
            hegemony_count INTEGER NOT NULL,
            population_count INTEGER NOT NULL
        );
    "#;

    /// Indexes for common queries
    pub const ASINFO_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_asinfo_core_name ON asinfo_core(name)",
        "CREATE INDEX IF NOT EXISTS idx_asinfo_core_country ON asinfo_core(country)",
        "CREATE INDEX IF NOT EXISTS idx_asinfo_as2org_org_id ON asinfo_as2org(org_id)",
        "CREATE INDEX IF NOT EXISTS idx_asinfo_as2org_org_name ON asinfo_as2org(org_name)",
        "CREATE INDEX IF NOT EXISTS idx_asinfo_peeringdb_name ON asinfo_peeringdb(name)",
    ];

    /// Get all table creation SQL statements
    pub fn all_tables() -> Vec<&'static str> {
        vec![
            Self::ASINFO_CORE_TABLE,
            Self::ASINFO_AS2ORG_TABLE,
            Self::ASINFO_PEERINGDB_TABLE,
            Self::ASINFO_HEGEMONY_TABLE,
            Self::ASINFO_POPULATION_TABLE,
            Self::ASINFO_META_TABLE,
        ]
    }
}

// =============================================================================
// Repository Implementation
// =============================================================================

impl<'a> AsinfoRepository<'a> {
    /// Create a new ASInfo repository
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    // =========================================================================
    // Data Loading
    // =========================================================================

    /// Store records from parsed JSONL, returns counts per table
    pub fn store_from_jsonl(
        &self,
        records: &[JsonlRecord],
        source_url: &str,
    ) -> Result<AsinfoStoreCounts> {
        // Clear existing data first
        self.clear()?;

        let mut counts = AsinfoStoreCounts::default();

        // Optimize for batch insert performance
        self.conn
            .execute("PRAGMA synchronous = OFF", [])
            .map_err(|e| anyhow!("Failed to set synchronous mode: {}", e))?;
        self.conn
            .query_row("PRAGMA journal_mode = MEMORY", [], |_| Ok(()))
            .map_err(|e| anyhow!("Failed to set journal mode: {}", e))?;
        self.conn
            .execute("PRAGMA cache_size = -64000", [])
            .map_err(|e| anyhow!("Failed to set cache size: {}", e))?;

        // Use a transaction for all inserts
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("Failed to begin transaction: {}", e))?;

        {
            // Prepare statements
            let mut stmt_core = tx.prepare(
                "INSERT OR REPLACE INTO asinfo_core (asn, name, country) VALUES (?1, ?2, ?3)",
            )?;
            let mut stmt_as2org = tx.prepare(
                "INSERT OR REPLACE INTO asinfo_as2org (asn, name, org_id, org_name, country) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            let mut stmt_peeringdb = tx.prepare(
                "INSERT OR REPLACE INTO asinfo_peeringdb (asn, name, name_long, aka, website, irr_as_set) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut stmt_hegemony = tx.prepare(
                "INSERT OR REPLACE INTO asinfo_hegemony (asn, ipv4, ipv6) VALUES (?1, ?2, ?3)",
            )?;
            let mut stmt_population = tx.prepare(
                "INSERT OR REPLACE INTO asinfo_population (asn, percent_country, percent_global, sample_count, user_count) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;

            for record in records {
                // Always insert core record
                stmt_core.execute(params![record.asn, &record.name, &record.country])?;
                counts.core += 1;

                // Insert optional records
                if let Some(as2org) = &record.as2org {
                    stmt_as2org.execute(params![
                        record.asn,
                        &as2org.name,
                        &as2org.org_id,
                        &as2org.org_name,
                        &as2org.country
                    ])?;
                    counts.as2org += 1;
                }

                if let Some(pdb) = &record.peeringdb {
                    stmt_peeringdb.execute(params![
                        record.asn,
                        &pdb.name,
                        &pdb.name_long,
                        &pdb.aka,
                        &pdb.website,
                        &pdb.irr_as_set
                    ])?;
                    counts.peeringdb += 1;
                }

                if let Some(heg) = &record.hegemony {
                    stmt_hegemony.execute(params![record.asn, heg.ipv4, heg.ipv6])?;
                    counts.hegemony += 1;
                }

                if let Some(pop) = &record.population {
                    stmt_population.execute(params![
                        record.asn,
                        pop.percent_country,
                        pop.percent_global,
                        pop.sample_count,
                        pop.user_count
                    ])?;
                    counts.population += 1;
                }
            }

            // Store metadata
            let now = chrono::Utc::now().timestamp();
            tx.execute(
                "INSERT OR REPLACE INTO asinfo_meta (id, source_url, last_updated, core_count, as2org_count, peeringdb_count, hegemony_count, population_count) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    source_url,
                    now,
                    counts.core as u32,
                    counts.as2org as u32,
                    counts.peeringdb as u32,
                    counts.hegemony as u32,
                    counts.population as u32
                ],
            )?;
        }

        tx.commit()
            .map_err(|e| anyhow!("Failed to commit transaction: {}", e))?;

        // Restore default settings
        self.conn
            .execute("PRAGMA synchronous = FULL", [])
            .map_err(|e| anyhow!("Failed to restore synchronous mode: {}", e))?;
        self.conn
            .query_row("PRAGMA journal_mode = DELETE", [], |_| Ok(()))
            .map_err(|e| anyhow!("Failed to restore journal mode: {}", e))?;

        info!(
            "ASInfo data loaded: {} core, {} as2org, {} peeringdb, {} hegemony, {} population",
            counts.core, counts.as2org, counts.peeringdb, counts.hegemony, counts.population
        );

        Ok(counts)
    }

    /// Fetch URL and store (convenience wrapper)
    pub fn load_from_url(&self, url: &str) -> Result<AsinfoStoreCounts> {
        info!("Loading ASInfo data from {}", url);

        let reader =
            oneio::get_reader(url).map_err(|e| anyhow!("Failed to fetch ASInfo data: {}", e))?;

        let buf_reader = std::io::BufReader::new(reader);
        let mut records = Vec::new();

        for (line_num, line) in buf_reader.lines().enumerate() {
            let line = line.map_err(|e| anyhow!("Failed to read line {}: {}", line_num + 1, e))?;
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<JsonlRecord>(&line) {
                Ok(record) => records.push(record),
                Err(e) => {
                    // Log warning but continue processing
                    tracing::warn!("Failed to parse line {}: {}", line_num + 1, e);
                }
            }
        }

        info!("Parsed {} records from JSONL", records.len());
        self.store_from_jsonl(&records, url)
    }

    /// Clear all asinfo tables
    pub fn clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM asinfo_core", [])
            .map_err(|e| anyhow!("Failed to clear asinfo_core: {}", e))?;
        self.conn
            .execute("DELETE FROM asinfo_as2org", [])
            .map_err(|e| anyhow!("Failed to clear asinfo_as2org: {}", e))?;
        self.conn
            .execute("DELETE FROM asinfo_peeringdb", [])
            .map_err(|e| anyhow!("Failed to clear asinfo_peeringdb: {}", e))?;
        self.conn
            .execute("DELETE FROM asinfo_hegemony", [])
            .map_err(|e| anyhow!("Failed to clear asinfo_hegemony: {}", e))?;
        self.conn
            .execute("DELETE FROM asinfo_population", [])
            .map_err(|e| anyhow!("Failed to clear asinfo_population: {}", e))?;
        self.conn
            .execute("DELETE FROM asinfo_meta", [])
            .map_err(|e| anyhow!("Failed to clear asinfo_meta: {}", e))?;
        Ok(())
    }

    // =========================================================================
    // Metadata
    // =========================================================================

    /// Check if core table is empty
    pub fn is_empty(&self) -> bool {
        let count: u32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM asinfo_core", [], |row| row.get(0))
            .unwrap_or(0);
        count == 0
    }

    /// Check if data needs refresh based on TTL
    pub fn needs_refresh(&self, ttl: Duration) -> bool {
        if self.is_empty() {
            return true;
        }

        match self.get_metadata() {
            Ok(Some(meta)) => {
                let now = chrono::Utc::now().timestamp();
                let age = now - meta.last_updated;
                age >= ttl.as_secs() as i64
            }
            _ => true,
        }
    }

    /// Get metadata (timestamp, counts, source URL)
    pub fn get_metadata(&self) -> Result<Option<AsinfoMetadata>> {
        let result = self.conn.query_row(
            "SELECT source_url, last_updated, core_count, as2org_count, peeringdb_count, hegemony_count, population_count FROM asinfo_meta WHERE id = 1",
            [],
            |row| {
                Ok(AsinfoMetadata {
                    source_url: row.get(0)?,
                    last_updated: row.get(1)?,
                    core_count: row.get(2)?,
                    as2org_count: row.get(3)?,
                    peeringdb_count: row.get(4)?,
                    hegemony_count: row.get(5)?,
                    population_count: row.get(6)?,
                })
            },
        );

        match result {
            Ok(meta) => Ok(Some(meta)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get metadata: {}", e)),
        }
    }

    // =========================================================================
    // Core Queries
    // =========================================================================

    /// Get full record for single ASN (LEFT JOINs all tables)
    pub fn get_full(&self, asn: u32) -> Result<Option<AsinfoFullRecord>> {
        let results = self.get_full_batch(&[asn])?;
        Ok(results.into_iter().next())
    }

    /// Get full records for multiple ASNs (batch)
    pub fn get_full_batch(&self, asns: &[u32]) -> Result<Vec<AsinfoFullRecord>> {
        if asns.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = asns.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            r#"
            SELECT
                c.asn, c.name, c.country,
                a.name, a.org_id, a.org_name, a.country,
                p.name, p.name_long, p.aka, p.website, p.irr_as_set,
                h.ipv4, h.ipv6,
                pop.percent_country, pop.percent_global, pop.sample_count, pop.user_count
            FROM asinfo_core c
            LEFT JOIN asinfo_as2org a ON c.asn = a.asn
            LEFT JOIN asinfo_peeringdb p ON c.asn = p.asn
            LEFT JOIN asinfo_hegemony h ON c.asn = h.asn
            LEFT JOIN asinfo_population pop ON c.asn = pop.asn
            WHERE c.asn IN ({})
            ORDER BY c.asn
            "#,
            placeholders.join(",")
        );

        let mut stmt = self.conn.prepare(&query)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            asns.iter().map(|a| a as &dyn rusqlite::ToSql).collect();

        let rows = stmt.query_map(params.as_slice(), |row| {
            let asn: u32 = row.get(0)?;

            let core = AsinfoCoreRecord {
                asn,
                name: row.get(1)?,
                country: row.get(2)?,
            };

            let as2org: Option<AsinfoAs2orgRecord> =
                row.get::<_, Option<String>>(3)?
                    .map(|name| AsinfoAs2orgRecord {
                        asn,
                        name,
                        org_id: row.get(4).unwrap_or_default(),
                        org_name: row.get(5).unwrap_or_default(),
                        country: row.get(6).unwrap_or_default(),
                    });

            let peeringdb: Option<AsinfoPeeringdbRecord> =
                row.get::<_, Option<String>>(7)?
                    .map(|name| AsinfoPeeringdbRecord {
                        asn,
                        name,
                        name_long: row.get(8).ok().flatten(),
                        aka: row.get(9).ok().flatten(),
                        website: row.get(10).ok().flatten(),
                        irr_as_set: row.get(11).ok().flatten(),
                    });

            let hegemony: Option<AsinfoHegemonyRecord> =
                row.get::<_, Option<f64>>(12)?
                    .map(|ipv4| AsinfoHegemonyRecord {
                        asn,
                        ipv4,
                        ipv6: row.get(13).unwrap_or(0.0),
                    });

            let population: Option<AsinfoPopulationRecord> =
                row.get::<_, Option<f64>>(14)?
                    .map(|percent_country| AsinfoPopulationRecord {
                        asn,
                        percent_country,
                        percent_global: row.get(15).unwrap_or(0.0),
                        sample_count: row.get(16).unwrap_or(0),
                        user_count: row.get(17).unwrap_or(0),
                    });

            Ok(AsinfoFullRecord {
                core,
                as2org,
                peeringdb,
                hegemony,
                population,
            })
        })?;

        let results: Vec<AsinfoFullRecord> = rows.filter_map(|r| r.ok()).collect();
        Ok(results)
    }

    /// Search by AS name OR org name (merged, deduplicated)
    pub fn search_by_text(&self, query: &str, limit: usize) -> Result<Vec<AsinfoCoreRecord>> {
        let pattern = format!("%{}%", query.to_lowercase());

        let sql = r#"
            SELECT DISTINCT c.asn, c.name, c.country
            FROM asinfo_core c
            LEFT JOIN asinfo_as2org a ON c.asn = a.asn
            LEFT JOIN asinfo_peeringdb p ON c.asn = p.asn
            WHERE LOWER(c.name) LIKE ?1
               OR LOWER(a.name) LIKE ?1
               OR LOWER(a.org_name) LIKE ?1
               OR LOWER(p.name) LIKE ?1
               OR LOWER(p.name_long) LIKE ?1
            ORDER BY c.asn
            LIMIT ?2
        "#;

        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![pattern, limit as u32], |row| {
            Ok(AsinfoCoreRecord {
                asn: row.get(0)?,
                name: row.get(1)?,
                country: row.get(2)?,
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Search by country code
    pub fn search_by_country(&self, country: &str, limit: usize) -> Result<Vec<AsinfoCoreRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT asn, name, country FROM asinfo_core WHERE UPPER(country) = UPPER(?1) ORDER BY asn LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![country, limit as u32], |row| {
            Ok(AsinfoCoreRecord {
                asn: row.get(0)?,
                name: row.get(1)?,
                country: row.get(2)?,
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // =========================================================================
    // Batch Lookups (for enrichment)
    // =========================================================================

    /// Batch lookup of AS names
    pub fn lookup_names_batch(&self, asns: &[u32]) -> HashMap<u32, String> {
        let mut result = HashMap::new();

        if asns.is_empty() {
            return result;
        }

        let placeholders: Vec<String> = asns.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT asn, name FROM asinfo_core WHERE asn IN ({})",
            placeholders.join(",")
        );

        if let Ok(mut stmt) = self.conn.prepare(&query) {
            let params: Vec<&dyn rusqlite::ToSql> =
                asns.iter().map(|a| a as &dyn rusqlite::ToSql).collect();

            if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    result.insert(row.0, row.1);
                }
            }
        }

        result
    }

    /// Batch lookup of org names (from as2org table)
    pub fn lookup_orgs_batch(&self, asns: &[u32]) -> HashMap<u32, String> {
        let mut result = HashMap::new();

        if asns.is_empty() {
            return result;
        }

        let placeholders: Vec<String> = asns.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT asn, org_name FROM asinfo_as2org WHERE asn IN ({})",
            placeholders.join(",")
        );

        if let Ok(mut stmt) = self.conn.prepare(&query) {
            let params: Vec<&dyn rusqlite::ToSql> =
                asns.iter().map(|a| a as &dyn rusqlite::ToSql).collect();

            if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    result.insert(row.0, row.1);
                }
            }
        }

        result
    }

    // =========================================================================
    // Individual Table Queries
    // =========================================================================

    /// Get just core record
    pub fn get_core(&self, asn: u32) -> Result<Option<AsinfoCoreRecord>> {
        let result = self.conn.query_row(
            "SELECT asn, name, country FROM asinfo_core WHERE asn = ?1",
            params![asn],
            |row| {
                Ok(AsinfoCoreRecord {
                    asn: row.get(0)?,
                    name: row.get(1)?,
                    country: row.get(2)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get core record: {}", e)),
        }
    }

    /// Get just AS2Org record
    pub fn get_as2org(&self, asn: u32) -> Result<Option<AsinfoAs2orgRecord>> {
        let result = self.conn.query_row(
            "SELECT asn, name, org_id, org_name, country FROM asinfo_as2org WHERE asn = ?1",
            params![asn],
            |row| {
                Ok(AsinfoAs2orgRecord {
                    asn: row.get(0)?,
                    name: row.get(1)?,
                    org_id: row.get(2)?,
                    org_name: row.get(3)?,
                    country: row.get(4)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get as2org record: {}", e)),
        }
    }

    /// Get just PeeringDB record
    pub fn get_peeringdb(&self, asn: u32) -> Result<Option<AsinfoPeeringdbRecord>> {
        let result = self.conn.query_row(
            "SELECT asn, name, name_long, aka, website, irr_as_set FROM asinfo_peeringdb WHERE asn = ?1",
            params![asn],
            |row| {
                Ok(AsinfoPeeringdbRecord {
                    asn: row.get(0)?,
                    name: row.get(1)?,
                    name_long: row.get(2)?,
                    aka: row.get(3)?,
                    website: row.get(4)?,
                    irr_as_set: row.get(5)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get peeringdb record: {}", e)),
        }
    }

    /// Get just hegemony record
    pub fn get_hegemony(&self, asn: u32) -> Result<Option<AsinfoHegemonyRecord>> {
        let result = self.conn.query_row(
            "SELECT asn, ipv4, ipv6 FROM asinfo_hegemony WHERE asn = ?1",
            params![asn],
            |row| {
                Ok(AsinfoHegemonyRecord {
                    asn: row.get(0)?,
                    ipv4: row.get(1)?,
                    ipv6: row.get(2)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get hegemony record: {}", e)),
        }
    }

    /// Get just population record
    pub fn get_population(&self, asn: u32) -> Result<Option<AsinfoPopulationRecord>> {
        let result = self.conn.query_row(
            "SELECT asn, percent_country, percent_global, sample_count, user_count FROM asinfo_population WHERE asn = ?1",
            params![asn],
            |row| {
                Ok(AsinfoPopulationRecord {
                    asn: row.get(0)?,
                    percent_country: row.get(1)?,
                    percent_global: row.get(2)?,
                    sample_count: row.get(3)?,
                    user_count: row.get(4)?,
                })
            },
        );

        match result {
            Ok(record) => Ok(Some(record)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get population record: {}", e)),
        }
    }

    /// Search by org_id
    pub fn search_by_org_id(&self, org_id: &str, limit: usize) -> Result<Vec<AsinfoAs2orgRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT asn, name, org_id, org_name, country FROM asinfo_as2org WHERE org_id = ?1 ORDER BY asn LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![org_id, limit as u32], |row| {
            Ok(AsinfoAs2orgRecord {
                asn: row.get(0)?,
                name: row.get(1)?,
                org_id: row.get(2)?,
                org_name: row.get(3)?,
                country: row.get(4)?,
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Get count of core records
    pub fn core_count(&self) -> u32 {
        self.conn
            .query_row("SELECT COUNT(*) FROM asinfo_core", [], |row| row.get(0))
            .unwrap_or(0)
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

        // Initialize ASInfo tables
        for table_sql in AsinfoSchemaDefinitions::all_tables() {
            db.conn.execute(table_sql, []).unwrap();
        }
        for index_sql in AsinfoSchemaDefinitions::ASINFO_INDEXES {
            db.conn.execute(index_sql, []).unwrap();
        }

        db
    }

    #[test]
    fn test_is_empty() {
        let db = setup_test_db();
        let repo = AsinfoRepository::new(&db.conn);
        assert!(repo.is_empty());
    }

    #[test]
    fn test_store_and_get() {
        let db = setup_test_db();
        let repo = AsinfoRepository::new(&db.conn);

        let records = vec![JsonlRecord {
            asn: 13335,
            name: "CLOUDFLARENET".to_string(),
            country: "US".to_string(),
            as2org: Some(JsonlAs2org {
                country: "US".to_string(),
                name: "Cloudflare, Inc.".to_string(),
                org_id: "CLOUD14".to_string(),
                org_name: "Cloudflare, Inc.".to_string(),
            }),
            peeringdb: Some(JsonlPeeringdb {
                aka: Some("Cloudflare".to_string()),
                asn: 13335,
                irr_as_set: Some("AS-CLOUDFLARE".to_string()),
                name: "Cloudflare, Inc.".to_string(),
                name_long: Some("Cloudflare, Inc.".to_string()),
                website: Some("https://www.cloudflare.com".to_string()),
            }),
            hegemony: Some(JsonlHegemony {
                asn: 13335,
                ipv4: 0.002,
                ipv6: 0.003,
            }),
            population: Some(JsonlPopulation {
                percent_country: 1.5,
                percent_global: 0.5,
                sample_count: 1000,
                user_count: 500000,
            }),
        }];

        let counts = repo.store_from_jsonl(&records, "test://source").unwrap();
        assert_eq!(counts.core, 1);
        assert_eq!(counts.as2org, 1);
        assert_eq!(counts.peeringdb, 1);
        assert_eq!(counts.hegemony, 1);
        assert_eq!(counts.population, 1);

        // Test get_full
        let full = repo.get_full(13335).unwrap().unwrap();
        assert_eq!(full.core.asn, 13335);
        assert_eq!(full.core.name, "CLOUDFLARENET");
        assert!(full.as2org.is_some());
        assert!(full.peeringdb.is_some());
        assert!(full.hegemony.is_some());
        assert!(full.population.is_some());

        // Test metadata
        let meta = repo.get_metadata().unwrap().unwrap();
        assert_eq!(meta.source_url, "test://source");
        assert_eq!(meta.core_count, 1);
    }

    #[test]
    fn test_search_by_text() {
        let db = setup_test_db();
        let repo = AsinfoRepository::new(&db.conn);

        let records = vec![
            JsonlRecord {
                asn: 13335,
                name: "CLOUDFLARENET".to_string(),
                country: "US".to_string(),
                as2org: None,
                peeringdb: None,
                hegemony: None,
                population: None,
            },
            JsonlRecord {
                asn: 15169,
                name: "GOOGLE".to_string(),
                country: "US".to_string(),
                as2org: None,
                peeringdb: None,
                hegemony: None,
                population: None,
            },
        ];

        repo.store_from_jsonl(&records, "test://source").unwrap();

        let results = repo.search_by_text("cloudflare", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].asn, 13335);

        let results = repo.search_by_text("google", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].asn, 15169);
    }

    #[test]
    fn test_lookup_names_batch() {
        let db = setup_test_db();
        let repo = AsinfoRepository::new(&db.conn);

        let records = vec![
            JsonlRecord {
                asn: 13335,
                name: "CLOUDFLARENET".to_string(),
                country: "US".to_string(),
                as2org: None,
                peeringdb: None,
                hegemony: None,
                population: None,
            },
            JsonlRecord {
                asn: 15169,
                name: "GOOGLE".to_string(),
                country: "US".to_string(),
                as2org: None,
                peeringdb: None,
                hegemony: None,
                population: None,
            },
        ];

        repo.store_from_jsonl(&records, "test://source").unwrap();

        let names = repo.lookup_names_batch(&[13335, 15169, 99999]);
        assert_eq!(names.len(), 2);
        assert_eq!(names.get(&13335), Some(&"CLOUDFLARENET".to_string()));
        assert_eq!(names.get(&15169), Some(&"GOOGLE".to_string()));
        assert!(names.get(&99999).is_none());
    }

    #[test]
    fn test_clear() {
        let db = setup_test_db();
        let repo = AsinfoRepository::new(&db.conn);

        let records = vec![JsonlRecord {
            asn: 13335,
            name: "CLOUDFLARENET".to_string(),
            country: "US".to_string(),
            as2org: None,
            peeringdb: None,
            hegemony: None,
            population: None,
        }];

        repo.store_from_jsonl(&records, "test://source").unwrap();
        assert!(!repo.is_empty());

        repo.clear().unwrap();
        assert!(repo.is_empty());
    }
}
