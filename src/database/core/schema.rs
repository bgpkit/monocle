//! Database schema management
//!
//! This module provides schema definitions and management for the shared database.
//! All tables are defined here to ensure consistency and enable cross-table queries.

use anyhow::{anyhow, Result};
use rusqlite::Connection;

/// Current schema version
/// Increment this when making breaking schema changes
pub const SCHEMA_VERSION: u32 = 2;

/// Schema definitions for all tables in the shared database
pub struct SchemaDefinitions;

impl SchemaDefinitions {
    /// SQL for creating the meta table (tracks schema version and global metadata)
    pub const META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS monocle_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
    "#;

    /// SQL for creating AS2Org tables
    pub const AS2ORG_AS_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS as2org_as (
            asn INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            org_id TEXT NOT NULL,
            source TEXT NOT NULL
        );
    "#;

    pub const AS2ORG_ORG_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS as2org_org (
            org_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            country TEXT NOT NULL,
            source TEXT NOT NULL
        );
    "#;

    /// SQL for creating AS2Org indexes
    pub const AS2ORG_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_as2org_as_org_id ON as2org_as(org_id)",
        "CREATE INDEX IF NOT EXISTS idx_as2org_as_name ON as2org_as(name)",
        "CREATE INDEX IF NOT EXISTS idx_as2org_org_name ON as2org_org(name)",
        "CREATE INDEX IF NOT EXISTS idx_as2org_org_country ON as2org_org(country)",
    ];

    /// SQL for creating AS2Org views
    pub const AS2ORG_VIEWS: &'static [&'static str] = &[
        r#"
        CREATE VIEW IF NOT EXISTS as2org_both AS
        SELECT a.asn, a.name AS 'as_name', b.name AS 'org_name', b.org_id, b.country
        FROM as2org_as AS a JOIN as2org_org AS b ON a.org_id = b.org_id;
        "#,
        r#"
        CREATE VIEW IF NOT EXISTS as2org_count AS
        SELECT org_id, org_name, COUNT(*) AS count
        FROM as2org_both GROUP BY org_name
        ORDER BY count DESC;
        "#,
        r#"
        CREATE VIEW IF NOT EXISTS as2org_all AS
        SELECT a.*, b.count
        FROM as2org_both AS a JOIN as2org_count AS b ON a.org_id = b.org_id;
        "#,
    ];

    /// SQL for creating AS2Rel tables
    pub const AS2REL_META_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS as2rel_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            file_url TEXT NOT NULL,
            last_updated INTEGER NOT NULL,
            max_peers_count INTEGER NOT NULL DEFAULT 0
        );
    "#;

    pub const AS2REL_TABLE: &'static str = r#"
        CREATE TABLE IF NOT EXISTS as2rel (
            asn1 INTEGER NOT NULL,
            asn2 INTEGER NOT NULL,
            paths_count INTEGER NOT NULL,
            peers_count INTEGER NOT NULL,
            rel INTEGER NOT NULL,
            PRIMARY KEY (asn1, asn2, rel)
        );
    "#;

    /// SQL for creating AS2Rel indexes
    pub const AS2REL_INDEXES: &'static [&'static str] = &[
        "CREATE INDEX IF NOT EXISTS idx_as2rel_asn1 ON as2rel(asn1)",
        "CREATE INDEX IF NOT EXISTS idx_as2rel_asn2 ON as2rel(asn2)",
    ];

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
            aspa_count INTEGER NOT NULL DEFAULT 0
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

/// Schema manager for the shared database
///
/// Handles schema initialization, version checking, and migrations.
pub struct SchemaManager<'a> {
    conn: &'a Connection,
}

impl<'a> SchemaManager<'a> {
    /// Create a new schema manager for the given connection
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Initialize the database schema
    ///
    /// Creates all tables, indexes, and views if they don't exist.
    /// Sets the schema version in the meta table.
    pub fn initialize(&self) -> Result<()> {
        // Create meta table first
        self.conn
            .execute(SchemaDefinitions::META_TABLE, [])
            .map_err(|e| anyhow!("Failed to create meta table: {}", e))?;

        // Set schema version
        self.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;

        // Create AS2Org tables
        self.conn
            .execute(SchemaDefinitions::AS2ORG_AS_TABLE, [])
            .map_err(|e| anyhow!("Failed to create as2org_as table: {}", e))?;

        self.conn
            .execute(SchemaDefinitions::AS2ORG_ORG_TABLE, [])
            .map_err(|e| anyhow!("Failed to create as2org_org table: {}", e))?;

        // Create AS2Org indexes
        for index_sql in SchemaDefinitions::AS2ORG_INDEXES {
            self.conn
                .execute(index_sql, [])
                .map_err(|e| anyhow!("Failed to create AS2Org index: {}", e))?;
        }

        // Create AS2Org views
        for view_sql in SchemaDefinitions::AS2ORG_VIEWS {
            self.conn
                .execute(view_sql, [])
                .map_err(|e| anyhow!("Failed to create AS2Org view: {}", e))?;
        }

        // Create AS2Rel tables
        self.conn
            .execute(SchemaDefinitions::AS2REL_META_TABLE, [])
            .map_err(|e| anyhow!("Failed to create as2rel_meta table: {}", e))?;

        self.conn
            .execute(SchemaDefinitions::AS2REL_TABLE, [])
            .map_err(|e| anyhow!("Failed to create as2rel table: {}", e))?;

        // Create AS2Rel indexes
        for index_sql in SchemaDefinitions::AS2REL_INDEXES {
            self.conn
                .execute(index_sql, [])
                .map_err(|e| anyhow!("Failed to create AS2Rel index: {}", e))?;
        }

        // Create RPKI tables
        self.conn
            .execute(SchemaDefinitions::RPKI_ROA_TABLE, [])
            .map_err(|e| anyhow!("Failed to create rpki_roa table: {}", e))?;

        self.conn
            .execute(SchemaDefinitions::RPKI_ASPA_TABLE, [])
            .map_err(|e| anyhow!("Failed to create rpki_aspa table: {}", e))?;

        self.conn
            .execute(SchemaDefinitions::RPKI_META_TABLE, [])
            .map_err(|e| anyhow!("Failed to create rpki_meta table: {}", e))?;

        // Create RPKI indexes
        for index_sql in SchemaDefinitions::RPKI_INDEXES {
            self.conn
                .execute(index_sql, [])
                .map_err(|e| anyhow!("Failed to create RPKI index: {}", e))?;
        }

        Ok(())
    }

    /// Check the current schema status
    pub fn check_status(&self) -> Result<SchemaStatus> {
        // Check if meta table exists
        let meta_exists: i32 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='monocle_meta'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if meta_exists == 0 {
            return Ok(SchemaStatus::NotInitialized);
        }

        // Get current schema version
        let current_version = self.get_schema_version()?;

        if current_version == SCHEMA_VERSION {
            // Verify schema integrity
            if self.verify_integrity()? {
                Ok(SchemaStatus::Current)
            } else {
                Ok(SchemaStatus::Corrupted)
            }
        } else if current_version < SCHEMA_VERSION {
            Ok(SchemaStatus::NeedsMigration {
                from: current_version,
                to: SCHEMA_VERSION,
            })
        } else {
            // Database is from a newer version
            Ok(SchemaStatus::Incompatible {
                database_version: current_version,
                required_version: SCHEMA_VERSION,
            })
        }
    }

    /// Get the current schema version from the database
    fn get_schema_version(&self) -> Result<u32> {
        let version: String = self
            .conn
            .query_row(
                "SELECT value FROM monocle_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "0".to_string());

        version
            .parse()
            .map_err(|e| anyhow!("Invalid schema version: {}", e))
    }

    /// Verify schema integrity by checking required tables exist
    fn verify_integrity(&self) -> Result<bool> {
        let required_tables = [
            "monocle_meta",
            "as2org_as",
            "as2org_org",
            "as2rel",
            "as2rel_meta",
            "rpki_roa",
            "rpki_aspa",
            "rpki_meta",
        ];

        for table in required_tables {
            let exists: i32 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            if exists == 0 {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Set a metadata value
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO monocle_meta (key, value, updated_at) VALUES (?1, ?2, strftime('%s', 'now'))",
                [key, value],
            )
            .map_err(|e| anyhow!("Failed to set meta value: {}", e))?;
        Ok(())
    }

    /// Get a metadata value
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let result: Result<String, _> = self.conn.query_row(
            "SELECT value FROM monocle_meta WHERE key = ?1",
            [key],
            |row| row.get(0),
        );

        match result {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("Failed to get meta value: {}", e)),
        }
    }

    /// Reset the database by dropping all tables
    pub fn reset(&self) -> Result<()> {
        // Drop views first (they depend on tables)
        self.conn.execute("DROP VIEW IF EXISTS as2org_all", [])?;
        self.conn.execute("DROP VIEW IF EXISTS as2org_count", [])?;
        self.conn.execute("DROP VIEW IF EXISTS as2org_both", [])?;

        // Drop tables
        self.conn.execute("DROP TABLE IF EXISTS as2rel", [])?;
        self.conn.execute("DROP TABLE IF EXISTS as2rel_meta", [])?;
        self.conn.execute("DROP TABLE IF EXISTS as2org_as", [])?;
        self.conn.execute("DROP TABLE IF EXISTS as2org_org", [])?;
        self.conn.execute("DROP TABLE IF EXISTS rpki_roa", [])?;
        self.conn.execute("DROP TABLE IF EXISTS rpki_aspa", [])?;
        self.conn.execute("DROP TABLE IF EXISTS rpki_meta", [])?;
        self.conn.execute("DROP TABLE IF EXISTS monocle_meta", [])?;

        Ok(())
    }
}

/// Status of the database schema
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaStatus {
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
    use rusqlite::Connection;

    fn create_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Configure like MonocleDatabase would
        conn.execute("PRAGMA foreign_keys=ON", []).unwrap();
        conn
    }

    #[test]
    fn test_schema_not_initialized() {
        let conn = create_test_db();
        let manager = SchemaManager::new(&conn);

        assert_eq!(
            manager.check_status().unwrap(),
            SchemaStatus::NotInitialized
        );
    }

    #[test]
    fn test_schema_initialize() {
        let conn = create_test_db();
        let manager = SchemaManager::new(&conn);

        manager.initialize().unwrap();

        assert_eq!(manager.check_status().unwrap(), SchemaStatus::Current);
    }

    #[test]
    fn test_schema_version() {
        let conn = create_test_db();
        let manager = SchemaManager::new(&conn);

        manager.initialize().unwrap();

        let version = manager.get_schema_version().unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_meta_operations() {
        let conn = create_test_db();
        let manager = SchemaManager::new(&conn);

        manager.initialize().unwrap();

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
        let manager = SchemaManager::new(&conn);

        manager.initialize().unwrap();
        assert_eq!(manager.check_status().unwrap(), SchemaStatus::Current);

        manager.reset().unwrap();
        assert_eq!(
            manager.check_status().unwrap(),
            SchemaStatus::NotInitialized
        );
    }
}
