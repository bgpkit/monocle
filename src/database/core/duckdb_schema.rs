//! DuckDB schema management
//!
//! This module provides schema definitions and management for the DuckDB database.
//! All tables are defined here to ensure consistency and enable cross-table queries.

use anyhow::{anyhow, Result};

use super::duckdb_conn::DuckDbConn;

/// Current schema version
/// Increment this when making breaking schema changes
pub const DUCKDB_SCHEMA_VERSION: u32 = 1;

/// Schema definitions for all tables in the DuckDB database
pub struct DuckDbSchemaDefinitions;

impl DuckDbSchemaDefinitions {
    /// SQL for creating the meta table (tracks schema version and global metadata)
    pub const META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS monocle_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TIMESTAMP DEFAULT current_timestamp
        )
    "#;

    /// SQL for creating AS2Org table (denormalized for DuckDB columnar efficiency)
    pub const AS2ORG_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS as2org (
            asn INTEGER PRIMARY KEY,
            as_name TEXT NOT NULL,
            org_id TEXT NOT NULL,
            org_name TEXT NOT NULL,
            country TEXT NOT NULL,
            source TEXT NOT NULL
        )
    "#;

    /// SQL for creating AS2Org indexes
    pub const AS2ORG_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_as2org_org_name ON as2org(org_name)",
        "CREATE INDEX IF NOT EXISTS idx_as2org_country ON as2org(country)",
        "CREATE INDEX IF NOT EXISTS idx_as2org_org_id ON as2org(org_id)",
    ];

    /// SQL for creating AS2Rel metadata table
    pub const AS2REL_META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS as2rel_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            file_url TEXT NOT NULL,
            last_updated TIMESTAMP NOT NULL,
            max_peers_count INTEGER NOT NULL DEFAULT 0
        )
    "#;

    /// SQL for creating AS2Rel table
    pub const AS2REL_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS as2rel (
            asn1 INTEGER NOT NULL,
            asn2 INTEGER NOT NULL,
            paths_count INTEGER NOT NULL,
            peers_count INTEGER NOT NULL,
            rel INTEGER NOT NULL,
            PRIMARY KEY (asn1, asn2, rel)
        )
    "#;

    /// SQL for creating AS2Rel indexes
    pub const AS2REL_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_as2rel_asn1 ON as2rel(asn1)",
        "CREATE INDEX IF NOT EXISTS idx_as2rel_asn2 ON as2rel(asn2)",
    ];

    /// SQL for creating RPKI cache metadata table
    pub const RPKI_CACHE_META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS rpki_cache_meta (
            id INTEGER PRIMARY KEY,
            data_type TEXT NOT NULL,
            data_source TEXT NOT NULL,
            data_date DATE,
            loaded_at TIMESTAMP NOT NULL DEFAULT current_timestamp,
            record_count INTEGER NOT NULL,
            UNIQUE (data_type, data_source, data_date)
        )
    "#;

    /// SQL for creating RPKI ROAs table
    pub const RPKI_ROAS_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS rpki_roas (
            prefix INET NOT NULL,
            max_length INTEGER NOT NULL,
            origin_asn INTEGER NOT NULL,
            ta TEXT,
            cache_id INTEGER NOT NULL REFERENCES rpki_cache_meta(id)
        )
    "#;

    /// SQL for creating RPKI ROAs indexes
    /// Note: DuckDB doesn't support indexes on INET types, so we only index non-INET columns
    pub const RPKI_ROAS_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_rpki_roas_origin ON rpki_roas(origin_asn)",
        "CREATE INDEX IF NOT EXISTS idx_rpki_roas_cache ON rpki_roas(cache_id)",
    ];

    /// SQL for creating RPKI ASPAs table
    pub const RPKI_ASPAS_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS rpki_aspas (
            customer_asn INTEGER NOT NULL,
            provider_asns INTEGER[] NOT NULL,
            cache_id INTEGER NOT NULL REFERENCES rpki_cache_meta(id)
        )
    "#;

    /// SQL for creating RPKI ASPAs indexes
    pub const RPKI_ASPAS_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_rpki_aspas_customer ON rpki_aspas(customer_asn)",
        "CREATE INDEX IF NOT EXISTS idx_rpki_aspas_cache ON rpki_aspas(cache_id)",
    ];

    /// SQL for creating Pfx2as cache metadata table
    pub const PFX2AS_CACHE_META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS pfx2as_cache_meta (
            id INTEGER PRIMARY KEY,
            data_source TEXT NOT NULL,
            loaded_at TIMESTAMP NOT NULL DEFAULT current_timestamp,
            record_count INTEGER NOT NULL
        )
    "#;

    /// SQL for creating Pfx2as table
    pub const PFX2AS_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS pfx2as (
            prefix INET NOT NULL,
            origin_asns INTEGER[] NOT NULL,
            cache_id INTEGER NOT NULL REFERENCES pfx2as_cache_meta(id)
        )
    "#;

    /// SQL for creating Pfx2as indexes
    /// Note: DuckDB doesn't support indexes on INET types, so we only index non-INET columns
    pub const PFX2AS_INDEXES: &'static [&'static str] =
        &["CREATE INDEX IF NOT EXISTS idx_pfx2as_cache ON pfx2as(cache_id)"];

    /// SQL for creating BGP elements table (internal DuckDB storage)
    pub const ELEMS_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS elems (
            timestamp TIMESTAMP,
            elem_type TEXT,
            collector TEXT,
            peer_ip INET,
            peer_asn INTEGER,
            prefix INET,
            next_hop INET,
            as_path TEXT,
            origin_asn INTEGER,
            origin TEXT,
            local_pref INTEGER,
            med INTEGER,
            communities TEXT,
            atomic BOOLEAN,
            aggr_asn INTEGER,
            aggr_ip INET
        )
    "#;

    /// SQL for creating BGP elements indexes
    /// Note: DuckDB doesn't support indexes on INET types (prefix, peer_ip, next_hop, aggr_ip)
    pub const ELEMS_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_elems_timestamp ON elems(timestamp)",
        "CREATE INDEX IF NOT EXISTS idx_elems_peer_asn ON elems(peer_asn)",
        "CREATE INDEX IF NOT EXISTS idx_elems_collector ON elems(collector)",
        "CREATE INDEX IF NOT EXISTS idx_elems_elem_type ON elems(elem_type)",
        "CREATE INDEX IF NOT EXISTS idx_elems_origin_asn ON elems(origin_asn)",
    ];
}

/// Schema manager for the DuckDB database
///
/// Handles schema initialization, version checking, and migrations.
pub struct DuckDbSchemaManager<'a> {
    conn: &'a DuckDbConn,
}

impl<'a> DuckDbSchemaManager<'a> {
    /// Create a new schema manager for the given connection
    pub fn new(conn: &'a DuckDbConn) -> Self {
        Self { conn }
    }

    /// Initialize the database schema
    ///
    /// Creates all tables, indexes, and views if they don't exist.
    /// Sets the schema version in the meta table.
    pub fn initialize(&self) -> Result<()> {
        // Create meta table first
        self.conn
            .execute(DuckDbSchemaDefinitions::META_TABLE)
            .map_err(|e| anyhow!("Failed to create meta table: {}", e))?;

        // Set schema version
        self.set_meta("schema_version", &DUCKDB_SCHEMA_VERSION.to_string())?;

        // Create AS2Org table
        self.conn
            .execute(DuckDbSchemaDefinitions::AS2ORG_TABLE)
            .map_err(|e| anyhow!("Failed to create as2org table: {}", e))?;

        // Create AS2Org indexes
        for index_sql in DuckDbSchemaDefinitions::AS2ORG_INDEXES {
            self.conn
                .execute(index_sql)
                .map_err(|e| anyhow!("Failed to create AS2Org index: {}", e))?;
        }

        // Create AS2Rel tables
        self.conn
            .execute(DuckDbSchemaDefinitions::AS2REL_META_TABLE)
            .map_err(|e| anyhow!("Failed to create as2rel_meta table: {}", e))?;

        self.conn
            .execute(DuckDbSchemaDefinitions::AS2REL_TABLE)
            .map_err(|e| anyhow!("Failed to create as2rel table: {}", e))?;

        // Create AS2Rel indexes
        for index_sql in DuckDbSchemaDefinitions::AS2REL_INDEXES {
            self.conn
                .execute(index_sql)
                .map_err(|e| anyhow!("Failed to create AS2Rel index: {}", e))?;
        }

        // Create RPKI cache tables
        self.conn
            .execute(DuckDbSchemaDefinitions::RPKI_CACHE_META_TABLE)
            .map_err(|e| anyhow!("Failed to create rpki_cache_meta table: {}", e))?;

        self.conn
            .execute(DuckDbSchemaDefinitions::RPKI_ROAS_TABLE)
            .map_err(|e| anyhow!("Failed to create rpki_roas table: {}", e))?;

        for index_sql in DuckDbSchemaDefinitions::RPKI_ROAS_INDEXES {
            self.conn
                .execute(index_sql)
                .map_err(|e| anyhow!("Failed to create RPKI ROAs index: {}", e))?;
        }

        self.conn
            .execute(DuckDbSchemaDefinitions::RPKI_ASPAS_TABLE)
            .map_err(|e| anyhow!("Failed to create rpki_aspas table: {}", e))?;

        for index_sql in DuckDbSchemaDefinitions::RPKI_ASPAS_INDEXES {
            self.conn
                .execute(index_sql)
                .map_err(|e| anyhow!("Failed to create RPKI ASPAs index: {}", e))?;
        }

        // Create Pfx2as cache tables
        self.conn
            .execute(DuckDbSchemaDefinitions::PFX2AS_CACHE_META_TABLE)
            .map_err(|e| anyhow!("Failed to create pfx2as_cache_meta table: {}", e))?;

        self.conn
            .execute(DuckDbSchemaDefinitions::PFX2AS_TABLE)
            .map_err(|e| anyhow!("Failed to create pfx2as table: {}", e))?;

        for index_sql in DuckDbSchemaDefinitions::PFX2AS_INDEXES {
            self.conn
                .execute(index_sql)
                .map_err(|e| anyhow!("Failed to create Pfx2as index: {}", e))?;
        }

        Ok(())
    }

    /// Initialize only the core tables (meta, as2org, as2rel) without RPKI/Pfx2as cache tables
    /// This is useful for backward compatibility during migration
    pub fn initialize_core(&self) -> Result<()> {
        // Create meta table first
        self.conn
            .execute(DuckDbSchemaDefinitions::META_TABLE)
            .map_err(|e| anyhow!("Failed to create meta table: {}", e))?;

        // Set schema version
        self.set_meta("schema_version", &DUCKDB_SCHEMA_VERSION.to_string())?;

        // Create AS2Org table
        self.conn
            .execute(DuckDbSchemaDefinitions::AS2ORG_TABLE)
            .map_err(|e| anyhow!("Failed to create as2org table: {}", e))?;

        // Create AS2Org indexes
        for index_sql in DuckDbSchemaDefinitions::AS2ORG_INDEXES {
            self.conn
                .execute(index_sql)
                .map_err(|e| anyhow!("Failed to create AS2Org index: {}", e))?;
        }

        // Create AS2Rel tables
        self.conn
            .execute(DuckDbSchemaDefinitions::AS2REL_META_TABLE)
            .map_err(|e| anyhow!("Failed to create as2rel_meta table: {}", e))?;

        self.conn
            .execute(DuckDbSchemaDefinitions::AS2REL_TABLE)
            .map_err(|e| anyhow!("Failed to create as2rel table: {}", e))?;

        // Create AS2Rel indexes
        for index_sql in DuckDbSchemaDefinitions::AS2REL_INDEXES {
            self.conn
                .execute(index_sql)
                .map_err(|e| anyhow!("Failed to create AS2Rel index: {}", e))?;
        }

        Ok(())
    }

    /// Check the current schema status
    pub fn check_status(&self) -> Result<DuckDbSchemaStatus> {
        // Check if meta table exists
        let meta_exists = self.conn.table_exists("monocle_meta")?;

        if !meta_exists {
            return Ok(DuckDbSchemaStatus::NotInitialized);
        }

        // Get current schema version
        let current_version = self.get_schema_version()?;

        if current_version == DUCKDB_SCHEMA_VERSION {
            // Verify schema integrity
            if self.verify_integrity()? {
                Ok(DuckDbSchemaStatus::Current)
            } else {
                Ok(DuckDbSchemaStatus::Corrupted)
            }
        } else if current_version < DUCKDB_SCHEMA_VERSION {
            Ok(DuckDbSchemaStatus::NeedsMigration {
                from: current_version,
                to: DUCKDB_SCHEMA_VERSION,
            })
        } else {
            // Database is from a newer version
            Ok(DuckDbSchemaStatus::Incompatible {
                database_version: current_version,
                required_version: DUCKDB_SCHEMA_VERSION,
            })
        }
    }

    /// Get the current schema version from the database
    fn get_schema_version(&self) -> Result<u32> {
        let result = self.get_meta("schema_version")?;
        match result {
            Some(version) => version
                .parse()
                .map_err(|e| anyhow!("Invalid schema version: {}", e)),
            None => Ok(0),
        }
    }

    /// Verify schema integrity by checking required tables exist
    fn verify_integrity(&self) -> Result<bool> {
        // Only check core tables for integrity (not cache tables which are optional)
        let required_tables = ["monocle_meta", "as2org", "as2rel", "as2rel_meta"];

        for table in required_tables {
            if !self.conn.table_exists(table)? {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Set a metadata value
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(&format!(
                "INSERT OR REPLACE INTO monocle_meta (key, value, updated_at) VALUES ('{}', '{}', current_timestamp)",
                key, value
            ))
            .map_err(|e| anyhow!("Failed to set meta value: {}", e))?;
        Ok(())
    }

    /// Get a metadata value
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .conn
            .prepare("SELECT value FROM monocle_meta WHERE key = ?")
            .map_err(|e| anyhow!("Failed to prepare meta query: {}", e))?;

        let result: std::result::Result<String, _> =
            stmt.query_row(duckdb::params![key], |row| row.get(0));

        match result {
            Ok(value) => Ok(Some(value)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get meta value: {}", e)),
        }
    }

    /// Reset the database by dropping all tables
    pub fn reset(&self) -> Result<()> {
        // Drop tables in reverse dependency order
        let tables = [
            "elems",
            "pfx2as",
            "pfx2as_cache_meta",
            "rpki_aspas",
            "rpki_roas",
            "rpki_cache_meta",
            "as2rel",
            "as2rel_meta",
            "as2org",
            "monocle_meta",
        ];

        for table in tables {
            self.conn
                .execute(&format!("DROP TABLE IF EXISTS {}", table))
                .ok(); // Ignore errors for tables that don't exist
        }

        Ok(())
    }
}

/// Status of the DuckDB schema
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuckDbSchemaStatus {
    /// Database is not initialized (fresh database)
    NotInitialized,

    /// Schema is current and valid
    Current,

    /// Schema needs migration from an older version
    NeedsMigration { from: u32, to: u32 },

    /// Database is from a newer version (incompatible)
    Incompatible {
        database_version: u32,
        required_version: u32,
    },

    /// Schema is corrupted (missing tables)
    Corrupted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_db() -> DuckDbConn {
        DuckDbConn::open_in_memory().unwrap()
    }

    #[test]
    fn test_schema_not_initialized() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        assert_eq!(
            manager.check_status().unwrap(),
            DuckDbSchemaStatus::NotInitialized
        );
    }

    #[test]
    fn test_schema_initialize_core() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        manager.initialize_core().unwrap();

        assert_eq!(manager.check_status().unwrap(), DuckDbSchemaStatus::Current);

        // Verify core tables exist
        assert!(conn.table_exists("monocle_meta").unwrap());
        assert!(conn.table_exists("as2org").unwrap());
        assert!(conn.table_exists("as2rel").unwrap());
        assert!(conn.table_exists("as2rel_meta").unwrap());
    }

    #[test]
    fn test_schema_initialize_full() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        manager.initialize().unwrap();

        assert_eq!(manager.check_status().unwrap(), DuckDbSchemaStatus::Current);

        // Verify all tables exist
        assert!(conn.table_exists("monocle_meta").unwrap());
        assert!(conn.table_exists("as2org").unwrap());
        assert!(conn.table_exists("as2rel").unwrap());
        assert!(conn.table_exists("as2rel_meta").unwrap());
        assert!(conn.table_exists("rpki_cache_meta").unwrap());
        assert!(conn.table_exists("rpki_roas").unwrap());
        assert!(conn.table_exists("rpki_aspas").unwrap());
        assert!(conn.table_exists("pfx2as_cache_meta").unwrap());
        assert!(conn.table_exists("pfx2as").unwrap());
    }

    #[test]
    fn test_schema_version() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        manager.initialize_core().unwrap();

        let version = manager.get_schema_version().unwrap();
        assert_eq!(version, DUCKDB_SCHEMA_VERSION);
    }

    #[test]
    fn test_meta_operations() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        manager.initialize_core().unwrap();

        // Set and get a meta value
        manager.set_meta("test_key", "test_value").unwrap();
        let value = manager.get_meta("test_key").unwrap();
        assert_eq!(value, Some("test_value".to_string()));

        // Non-existent key
        let missing = manager.get_meta("nonexistent").unwrap();
        assert_eq!(missing, None);
    }

    #[test]
    fn test_schema_reset() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        manager.initialize_core().unwrap();
        assert_eq!(manager.check_status().unwrap(), DuckDbSchemaStatus::Current);

        manager.reset().unwrap();
        assert_eq!(
            manager.check_status().unwrap(),
            DuckDbSchemaStatus::NotInitialized
        );
    }

    #[test]
    fn test_inet_types_in_schema() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        manager.initialize().unwrap();

        // Test inserting INET data into rpki_roas
        conn.execute(
            "INSERT INTO rpki_cache_meta (id, data_type, data_source, record_count) VALUES (1, 'roas', 'test', 0)",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO rpki_roas (prefix, max_length, origin_asn, cache_id) VALUES ('10.0.0.0/8'::INET, 24, 65000, 1)",
        )
        .unwrap();

        // Test INET containment query
        let mut stmt = conn
            .conn
            .prepare("SELECT COUNT(*) FROM rpki_roas WHERE prefix <<= '10.0.0.0/8'::INET")
            .unwrap();
        let count: i32 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_array_types_in_schema() {
        let conn = create_test_db();
        let manager = DuckDbSchemaManager::new(&conn);

        manager.initialize().unwrap();

        // Test inserting array data into rpki_aspas
        conn.execute(
            "INSERT INTO rpki_cache_meta (id, data_type, data_source, record_count) VALUES (1, 'aspas', 'test', 0)",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO rpki_aspas (customer_asn, provider_asns, cache_id) VALUES (65000, [65001, 65002, 65003], 1)",
        )
        .unwrap();

        // Test array containment query
        let mut stmt = conn
            .conn
            .prepare("SELECT COUNT(*) FROM rpki_aspas WHERE list_contains(provider_asns, 65002)")
            .unwrap();
        let count: i32 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
    }
}
