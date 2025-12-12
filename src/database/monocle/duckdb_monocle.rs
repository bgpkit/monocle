//! DuckDB Monocle database storage
//!
//! This module provides the main persistent DuckDB database used across monocle sessions.
//! The monocle database stores:
//! - AS2Org mappings (AS to Organization) - denormalized for columnar efficiency
//! - AS2Rel data (AS-level relationships)
//! - RPKI cache (ROAs, ASPAs) - Phase 3
//! - Pfx2as cache - Phase 3
//!
//! All data in the monocle database can be regenerated from external sources,
//! so schema migrations can reset and repopulate when needed.

use anyhow::{anyhow, Result};
use tracing::info;

use crate::database::core::{DuckDbConn, DuckDbSchemaManager, DuckDbSchemaStatus};

use super::duckdb_as2org::DuckDbAs2orgRepository;
use super::duckdb_as2rel::DuckDbAs2relRepository;

/// Main monocle DuckDB database for persistent data
///
/// `DuckDbMonocleDatabase` provides a unified interface to all monocle data tables
/// using DuckDB as the backend. It handles:
/// - Schema initialization and migrations
/// - Automatic schema drift detection and reset
/// - Access to data repositories
pub struct DuckDbMonocleDatabase {
    conn: DuckDbConn,
}

impl DuckDbMonocleDatabase {
    /// Open the monocle database at the specified path
    ///
    /// If the database doesn't exist, it will be created and initialized.
    /// If the schema is outdated or corrupted, it will be reset and
    /// data will need to be repopulated.
    pub fn open(path: &str) -> Result<Self> {
        let conn = DuckDbConn::open_path(path)?;
        let schema = DuckDbSchemaManager::new(&conn);

        match schema.check_status()? {
            DuckDbSchemaStatus::Current => {
                info!("DuckDB monocle database schema is current");
            }
            DuckDbSchemaStatus::NotInitialized => {
                info!("Initializing DuckDB monocle database schema");
                schema.initialize_core()?;
            }
            DuckDbSchemaStatus::NeedsMigration { from, to } => {
                info!(
                    "DuckDB monocle database needs migration from v{} to v{}",
                    from, to
                );
                // For now, we reset and reinitialize
                // In the future, we could implement incremental migrations
                schema.reset()?;
                schema.initialize_core()?;
            }
            DuckDbSchemaStatus::Incompatible {
                database_version,
                required_version,
            } => {
                info!(
                    "DuckDB monocle database schema incompatible (db: v{}, required: v{}), resetting",
                    database_version, required_version
                );
                schema.reset()?;
                schema.initialize_core()?;
            }
            DuckDbSchemaStatus::Corrupted => {
                info!("DuckDB monocle database schema corrupted, resetting");
                schema.reset()?;
                schema.initialize_core()?;
            }
        }

        Ok(Self { conn })
    }

    /// Open the monocle database from a data directory
    ///
    /// Creates the standard database file path: `{data_dir}/monocle-data.duckdb`
    pub fn open_in_dir(data_dir: &str) -> Result<Self> {
        let path = format!("{}/monocle-data.duckdb", data_dir);
        Self::open(&path)
    }

    /// Create an in-memory monocle database (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let conn = DuckDbConn::open_in_memory()?;
        let schema = DuckDbSchemaManager::new(&conn);
        schema.initialize_core()?;
        Ok(Self { conn })
    }

    /// Get a reference to the AS2Org repository
    pub fn as2org(&self) -> DuckDbAs2orgRepository<'_> {
        DuckDbAs2orgRepository::new(&self.conn)
    }

    /// Get a reference to the AS2Rel repository
    pub fn as2rel(&self) -> DuckDbAs2relRepository<'_> {
        DuckDbAs2relRepository::new(&self.conn)
    }

    /// Get the underlying DuckDB connection (for advanced queries)
    ///
    /// Use this for cross-table queries that span multiple repositories.
    pub fn connection(&self) -> &DuckDbConn {
        &self.conn
    }

    /// Check if the AS2Org data needs to be bootstrapped
    pub fn needs_as2org_bootstrap(&self) -> bool {
        self.as2org().is_empty()
    }

    /// Check if the AS2Rel data needs to be updated
    pub fn needs_as2rel_update(&self) -> bool {
        self.as2rel().should_update()
    }

    /// Bootstrap AS2Org data from bgpkit-commons
    ///
    /// Returns the count of entries loaded.
    pub fn bootstrap_as2org(&self) -> Result<usize> {
        self.as2org().load_from_commons()
    }

    /// Update AS2Rel data from the default URL
    ///
    /// Returns the number of entries loaded.
    pub fn update_as2rel(&self) -> Result<usize> {
        self.as2rel().load_from_url()
    }

    /// Update AS2Rel data from a custom path
    ///
    /// Returns the number of entries loaded.
    pub fn update_as2rel_from(&self, path: &str) -> Result<usize> {
        self.as2rel().load_from_path(path)
    }

    /// Get metadata value from the database
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let schema = DuckDbSchemaManager::new(&self.conn);
        schema.get_meta(key)
    }

    /// Set metadata value in the database
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let schema = DuckDbSchemaManager::new(&self.conn);
        schema.set_meta(key, value)
    }

    /// Set memory limit for the database
    pub fn set_memory_limit(&self, limit: &str) -> Result<()> {
        self.conn.set_memory_limit(limit)
    }
}

/// Ensure the data directory exists
pub fn ensure_duckdb_data_dir(data_dir: &str) -> Result<()> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| anyhow!("Failed to create data directory '{}': {}", data_dir, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = DuckDbMonocleDatabase::open_in_memory();
        assert!(db.is_ok());
    }

    #[test]
    fn test_repositories() {
        let db = DuckDbMonocleDatabase::open_in_memory().unwrap();

        // Should have empty repositories
        assert!(db.as2org().is_empty());
        assert!(db.as2rel().is_empty());
    }

    #[test]
    fn test_needs_bootstrap() {
        let db = DuckDbMonocleDatabase::open_in_memory().unwrap();

        assert!(db.needs_as2org_bootstrap());
        assert!(db.needs_as2rel_update());
    }

    #[test]
    fn test_meta_operations() {
        let db = DuckDbMonocleDatabase::open_in_memory().unwrap();

        // Set and get a meta value
        db.set_meta("test_key", "test_value").unwrap();
        let value = db.get_meta("test_key").unwrap();
        assert_eq!(value, Some("test_value".to_string()));
    }

    #[test]
    fn test_set_memory_limit() {
        let db = DuckDbMonocleDatabase::open_in_memory().unwrap();
        let result = db.set_memory_limit("1GB");
        assert!(result.is_ok());
    }

    #[test]
    fn test_cross_table_query() {
        let db = DuckDbMonocleDatabase::open_in_memory().unwrap();

        // Insert test data
        db.connection()
            .execute(
                "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source) VALUES
                 (65000, 'Test AS', 'TEST-ORG', 'Test Organization', 'US', 'test')",
            )
            .unwrap();

        db.connection()
            .execute(
                "INSERT INTO as2rel (asn1, asn2, paths_count, peers_count, rel) VALUES
                 (65000, 65001, 100, 10, 0)",
            )
            .unwrap();

        // Test cross-table query
        let mut stmt = db
            .connection()
            .conn
            .prepare(
                "SELECT r.asn1, r.asn2, a.org_name
                 FROM as2rel r
                 LEFT JOIN as2org a ON r.asn1 = a.asn
                 WHERE r.asn1 = 65000",
            )
            .unwrap();

        let mut rows = stmt.query([]).unwrap();
        let mut count = 0;
        while let Some(row) = rows.next().unwrap() {
            let asn1: u32 = row.get(0).unwrap();
            let org_name: Option<String> = row.get(2).unwrap();
            assert_eq!(asn1, 65000);
            assert_eq!(org_name, Some("Test Organization".to_string()));
            count += 1;
        }
        assert_eq!(count, 1);
    }
}
