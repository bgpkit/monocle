//! Monocle database storage
//!
//! This module provides the main persistent database used across monocle sessions.
//! The monocle database stores:
//! - AS2Org mappings (AS to Organization)
//! - AS2Rel data (AS-level relationships)
//! - Future: RPKI ROAs, prefix observations, etc.
//!
//! All data in the monocle database can be regenerated from external sources,
//! so schema migrations can reset and repopulate when needed.
//!
//! # Database Backends
//!
//! This module supports two database backends:
//! - **SQLite** (`MonocleDatabase`): Legacy backend, retained for backward compatibility
//! - **DuckDB** (`DuckDbMonocleDatabase`): Primary backend with native INET type support
//!
//! New code should prefer the DuckDB backend for better IP/prefix query performance.

mod as2org;
mod as2rel;
mod duckdb_as2org;
mod duckdb_as2rel;
mod duckdb_monocle;

// SQLite exports (for backward compatibility)
pub use as2org::{As2orgRecord, As2orgRepository};
pub use as2rel::{
    AggregatedRelationship, As2relEntry, As2relMeta, As2relRecord, As2relRepository,
    BGPKIT_AS2REL_URL,
};

// DuckDB exports (primary backend)
pub use duckdb_as2org::{DuckDbAs2orgRecord, DuckDbAs2orgRepository};
pub use duckdb_as2rel::{
    DuckDbAggregatedRelationship, DuckDbAs2relEntry, DuckDbAs2relMeta, DuckDbAs2relRecord,
    DuckDbAs2relRepository, DUCKDB_BGPKIT_AS2REL_URL,
};
pub use duckdb_monocle::{ensure_duckdb_data_dir, DuckDbMonocleDatabase};

use crate::database::core::{DatabaseConn, SchemaManager, SchemaStatus};
use anyhow::{anyhow, Result};
use tracing::info;

/// Main monocle database for persistent data (SQLite backend)
///
/// `MonocleDatabase` provides a unified interface to all monocle data tables.
/// It handles:
/// - Schema initialization and migrations
/// - Automatic schema drift detection and reset
/// - Access to data repositories
///
/// **Note**: For new code, prefer `DuckDbMonocleDatabase` which provides
/// native INET type support for better IP/prefix query performance.
pub struct MonocleDatabase {
    db: DatabaseConn,
}

impl MonocleDatabase {
    /// Open the monocle database at the specified path
    ///
    /// If the database doesn't exist, it will be created and initialized.
    /// If the schema is outdated or corrupted, it will be reset and
    /// data will need to be repopulated.
    pub fn open(path: &str) -> Result<Self> {
        let db = DatabaseConn::open_path(path)?;
        let schema = SchemaManager::new(&db.conn);

        match schema.check_status()? {
            SchemaStatus::Current => {
                info!("Monocle database schema is current");
            }
            SchemaStatus::NotInitialized => {
                info!("Initializing monocle database schema");
                schema.initialize()?;
            }
            SchemaStatus::NeedsMigration { from, to } => {
                info!("Monocle database needs migration from v{} to v{}", from, to);
                // For now, we reset and reinitialize
                // In the future, we could implement incremental migrations
                schema.reset()?;
                schema.initialize()?;
            }
            SchemaStatus::Incompatible {
                database_version,
                required_version,
            } => {
                info!(
                    "Monocle database schema incompatible (db: v{}, required: v{}), resetting",
                    database_version, required_version
                );
                schema.reset()?;
                schema.initialize()?;
            }
            SchemaStatus::Corrupted => {
                info!("Monocle database schema corrupted, resetting");
                schema.reset()?;
                schema.initialize()?;
            }
        }

        Ok(Self { db })
    }

    /// Open the monocle database from a data directory
    ///
    /// Creates the standard database file path: `{data_dir}/monocle-data.sqlite3`
    pub fn open_in_dir(data_dir: &str) -> Result<Self> {
        let path = format!("{}/monocle-data.sqlite3", data_dir);
        Self::open(&path)
    }

    /// Create an in-memory monocle database (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let db = DatabaseConn::open_in_memory()?;
        let schema = SchemaManager::new(&db.conn);
        schema.initialize()?;
        Ok(Self { db })
    }

    /// Get a reference to the AS2Org repository
    pub fn as2org(&self) -> As2orgRepository<'_> {
        As2orgRepository::new(&self.db.conn)
    }

    /// Get a reference to the AS2Rel repository
    pub fn as2rel(&self) -> As2relRepository<'_> {
        As2relRepository::new(&self.db.conn)
    }

    /// Get the underlying database connection (for advanced queries)
    ///
    /// Use this for cross-table queries that span multiple repositories.
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.db.conn
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
    /// Returns (as_count, org_count) on success.
    pub fn bootstrap_as2org(&self) -> Result<(usize, usize)> {
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
        let schema = SchemaManager::new(&self.db.conn);
        schema.get_meta(key)
    }

    /// Set metadata value in the database
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        let schema = SchemaManager::new(&self.db.conn);
        schema.set_meta(key, value)
    }
}

/// Ensure the data directory exists
pub fn ensure_data_dir(data_dir: &str) -> Result<()> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| anyhow!("Failed to create data directory '{}': {}", data_dir, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = MonocleDatabase::open_in_memory();
        assert!(db.is_ok());
    }

    #[test]
    fn test_repositories() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // Should have empty repositories
        assert!(db.as2org().is_empty());
        assert!(db.as2rel().is_empty());
    }

    #[test]
    fn test_needs_bootstrap() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        assert!(db.needs_as2org_bootstrap());
        assert!(db.needs_as2rel_update());
    }

    #[test]
    fn test_meta_operations() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // Set and get a meta value
        db.set_meta("test_key", "test_value").unwrap();
        let value = db.get_meta("test_key").unwrap();
        assert_eq!(value, Some("test_value".to_string()));
    }
}
