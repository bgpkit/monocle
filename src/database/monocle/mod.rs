//! Monocle database storage
//!
//! This module provides the main persistent database used across monocle sessions.
//! The monocle database stores:
//! - ASInfo (unified AS information) - SQLite
//! - AS2Rel data (AS-level relationships) - SQLite
//! - RPKI ROAs and ASPAs - SQLite (with blob-based prefix storage)
//! - Pfx2as mappings - SQLite (with blob-based prefix storage)

mod as2rel;
mod asinfo;
mod pfx2as;
mod rpki;

// SQLite-based repositories
pub use as2rel::{
    AggregatedRelationship, As2relEntry, As2relMeta, As2relRecord, As2relRepository,
    AsConnectivitySummary, ConnectivityEntry, ConnectivityGroup, BGPKIT_AS2REL_URL,
};
pub use asinfo::{
    AsinfoAs2orgRecord, AsinfoCoreRecord, AsinfoFullRecord, AsinfoHegemonyRecord, AsinfoMetadata,
    AsinfoPeeringdbRecord, AsinfoPopulationRecord, AsinfoRepository, AsinfoSchemaDefinitions,
    AsinfoStoreCounts, JsonlRecord, ASINFO_DATA_URL, DEFAULT_ASINFO_TTL,
};
pub use rpki::{
    RpkiAspaEnrichedRecord, RpkiAspaProviderEnriched, RpkiAspaRecord, RpkiCacheMetadata,
    RpkiRepository, RpkiRoaRecord, RpkiValidationResult, RpkiValidationState,
    DEFAULT_RPKI_CACHE_TTL,
};

// Pfx2as repository (SQLite-based)
pub use pfx2as::{
    Pfx2asCacheDbMetadata, Pfx2asDbRecord, Pfx2asQueryResult, Pfx2asRepository,
    Pfx2asSchemaDefinitions, ValidationStats, DEFAULT_PFX2AS_CACHE_TTL,
};

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

    /// Get a reference to the AS2Rel repository
    pub fn as2rel(&self) -> As2relRepository<'_> {
        As2relRepository::new(&self.db.conn)
    }

    /// Get a reference to the RPKI repository
    pub fn rpki(&self) -> RpkiRepository<'_> {
        RpkiRepository::new(&self.db.conn)
    }

    /// Get a reference to the Pfx2as repository
    pub fn pfx2as(&self) -> Pfx2asRepository<'_> {
        Pfx2asRepository::new(&self.db.conn)
    }

    /// Get a reference to the ASInfo repository
    pub fn asinfo(&self) -> AsinfoRepository<'_> {
        AsinfoRepository::new(&self.db.conn)
    }

    /// Get the underlying database connection (for advanced queries)
    ///
    /// Use this for cross-table queries that span multiple repositories.
    pub fn connection(&self) -> &rusqlite::Connection {
        &self.db.conn
    }

    /// Check if the ASInfo data needs to be bootstrapped
    pub fn needs_asinfo_bootstrap(&self) -> bool {
        self.asinfo().is_empty()
    }

    /// Check if the ASInfo data needs refresh
    pub fn needs_asinfo_refresh(&self) -> bool {
        self.asinfo().needs_refresh(DEFAULT_ASINFO_TTL)
    }

    /// Bootstrap ASInfo data from the default URL
    ///
    /// Returns the counts of records loaded per table.
    pub fn bootstrap_asinfo(&self) -> Result<AsinfoStoreCounts> {
        self.asinfo().load_from_url(ASINFO_DATA_URL)
    }

    /// Check if the AS2Rel data needs to be updated
    pub fn needs_as2rel_update(&self) -> bool {
        self.as2rel().should_update()
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

    /// Check if the RPKI cache needs refresh
    pub fn needs_rpki_refresh(&self) -> bool {
        self.rpki().needs_refresh(DEFAULT_RPKI_CACHE_TTL)
    }

    /// Check if the RPKI cache needs refresh with custom TTL
    pub fn needs_rpki_refresh_with_ttl(&self, ttl: chrono::Duration) -> bool {
        self.rpki().needs_refresh(ttl)
    }

    /// Check if the Pfx2as cache needs refresh
    pub fn needs_pfx2as_refresh(&self) -> bool {
        self.pfx2as().needs_refresh(DEFAULT_PFX2AS_CACHE_TTL)
    }

    /// Check if the Pfx2as cache needs refresh with custom TTL
    pub fn needs_pfx2as_refresh_with_ttl(&self, ttl: chrono::Duration) -> bool {
        self.pfx2as().needs_refresh(ttl)
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
        assert!(db.as2rel().is_empty());
    }

    #[test]
    fn test_needs_bootstrap() {
        let db = MonocleDatabase::open_in_memory().unwrap();

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

    #[test]
    fn test_all_repositories_accessible() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // All repositories should be accessible and empty initially
        assert!(db.asinfo().is_empty());
        assert!(db.as2rel().is_empty());
        assert!(db.rpki().is_empty());
        assert!(db.pfx2as().is_empty());
    }

    #[test]
    fn test_schema_initialized_on_open() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // The monocle_meta table should exist and have a version
        let version: String = db
            .connection()
            .query_row(
                "SELECT value FROM monocle_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert!(!version.is_empty());
    }

    #[test]
    fn test_rpki_store_and_retrieve_mock_data() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // Store mock RPKI data
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

        let aspas = vec![RpkiAspaRecord {
            customer_asn: 64496,
            provider_asns: vec![64497, 64498],
        }];

        db.rpki().store(&roas, &aspas).unwrap();

        // Verify data is stored
        assert!(!db.rpki().is_empty());
        let retrieved_roas = db.rpki().get_roas_by_asn(13335).unwrap();
        assert_eq!(retrieved_roas.len(), 1);
        assert_eq!(retrieved_roas[0].prefix, "1.0.0.0/24");

        let retrieved_aspas = db.rpki().get_aspas_by_customer(64496).unwrap();
        assert_eq!(retrieved_aspas.len(), 1);
        assert_eq!(retrieved_aspas[0].provider_asns.len(), 2);
    }

    #[test]
    fn test_pfx2as_store_and_retrieve_mock_data() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // Store mock pfx2as data
        let records = vec![
            Pfx2asDbRecord {
                prefix: "1.0.0.0/24".to_string(),
                origin_asn: 13335,
                validation: "valid".to_string(),
            },
            Pfx2asDbRecord {
                prefix: "8.8.8.0/24".to_string(),
                origin_asn: 15169,
                validation: "valid".to_string(),
            },
            Pfx2asDbRecord {
                prefix: "192.0.2.0/24".to_string(),
                origin_asn: 64496,
                validation: "unknown".to_string(),
            },
        ];

        db.pfx2as().store(&records, "test://mock").unwrap();

        // Verify data is stored
        assert!(!db.pfx2as().is_empty());

        // Check validation stats
        let stats = db.pfx2as().validation_stats().unwrap();
        assert_eq!(stats.valid, 2);
        assert_eq!(stats.unknown, 1);
        assert_eq!(stats.invalid, 0);
    }

    #[test]
    fn test_needs_refresh_flags() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // Empty databases should need refresh
        assert!(db.needs_asinfo_bootstrap());
        assert!(db.needs_asinfo_refresh());
        assert!(db.needs_as2rel_update());
        assert!(db.needs_rpki_refresh());
        assert!(db.needs_pfx2as_refresh());
    }

    #[test]
    fn test_connection_accessible() {
        let db = MonocleDatabase::open_in_memory().unwrap();

        // Should be able to execute queries on the connection
        let result: i32 = db
            .connection()
            .query_row("SELECT 1 + 1", [], |row| row.get(0))
            .unwrap();

        assert_eq!(result, 2);
    }
}
