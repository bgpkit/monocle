//! Database module
//!
//! This module provides all database functionality for monocle, organized into:
//!
//! - **core**: Core database infrastructure (SQLite connections, schema management)
//! - **session**: Session-based storage for one-time operations (e.g., search results)
//! - **monocle**: Main monocle database for persistent data
//!
//! # Architecture
//!
//! ```text
//! database/
//! ├── core/           # Foundation
//! │   ├── connection  # SQLite DatabaseConn wrapper
//! │   └── schema      # SQLite schema definitions and management
//! │
//! ├── session/        # One-time storage (requires lens-bgpkit feature)
//! │   └── msg_store   # BGP message search results (SQLite)
//! │
//! └── monocle/        # Persistent storage (all SQLite)
//!     ├── asinfo      # Unified AS information (from bgpkit-commons)
//!     ├── as2rel      # AS-level relationships
//!     ├── rpki        # RPKI ROA/ASPA data (blob-based prefix storage)
//!     └── pfx2as      # Prefix-to-ASN mappings (blob-based prefix storage)
//! ```
//!
//! # Database Backend Strategy
//!
//! Monocle uses SQLite as its sole database backend. All data is stored in SQLite:
//! - ASInfo (unified AS information)
//! - AS2Rel (AS-level relationships)
//! - RPKI ROAs and ASPAs (with blob-based prefix storage for range queries)
//! - Pfx2as mappings (with blob-based prefix storage for range queries)
//!
//! IP prefixes are stored as 16-byte start/end address pairs (BLOBs), with IPv4
//! addresses converted to IPv6-mapped format for uniform storage.
//!
//! # Feature Requirements
//!
//! - Core database types are always available
//! - `MsgStore` requires the `lens-bgpkit` feature (depends on bgpkit_parser)
//!
//! # Usage
//!
//! ## Monocle Database (SQLite)
//!
//! The monocle database is the primary interface for all persistent data:
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//!
//! // Open the monocle database
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//!
//! // Bootstrap ASInfo data if needed
//! if db.needs_asinfo_bootstrap() {
//!     db.bootstrap_asinfo()?;
//! }
//!
//! // Query AS data
//! let results = db.asinfo().search_by_name("cloudflare")?;
//!
//! // Query RPKI data
//! let roas = db.rpki().get_roas_by_asn(13335)?;
//!
//! // Query prefix-to-ASN mappings
//! let results = db.pfx2as().lookup_longest("1.1.1.1")?;
//! ```
//!
//! ## Session Database (SQLite - for exports)
//!
//! For one-time operations like storing search results (requires `lens-bgpkit` feature):
//!
//! ```rust,ignore
//! use monocle::database::MsgStore;
//!
//! // Create a session store
//! let store = MsgStore::new(Some("/tmp/search-results.db"), false)?;
//!
//! // Insert BGP elements
//! store.insert_elems(&elements)?;
//! ```

pub mod core;
pub mod monocle;
pub mod session;

// =============================================================================
// SQLite Types (Primary Database Backend)
// =============================================================================

// SQLite connection and schema management
pub use core::{DatabaseConn, SchemaDefinitions, SchemaManager, SchemaStatus, SCHEMA_VERSION};

// Monocle database (main entry point for AS2Rel)
pub use monocle::MonocleDatabase;

// AS2Rel repository
pub use monocle::{
    AggregatedRelationship, As2relEntry, As2relMeta, As2relRecord, As2relRepository,
    AsConnectivitySummary, ConnectivityEntry, ConnectivityGroup, BGPKIT_AS2REL_URL,
};

// ASInfo repository (unified AS information from multiple sources)
pub use monocle::{
    AsinfoAs2orgRecord, AsinfoCoreRecord, AsinfoFullRecord, AsinfoHegemonyRecord, AsinfoMetadata,
    AsinfoPeeringdbRecord, AsinfoPopulationRecord, AsinfoRepository, AsinfoSchemaDefinitions,
    AsinfoStoreCounts, JsonlRecord, ASINFO_DATA_URL, DEFAULT_ASINFO_TTL,
};

// RPKI repository (SQLite-based cache)
pub use monocle::{
    RpkiAspaRecord, RpkiCacheMetadata, RpkiRepository, RpkiRoaRecord, RpkiValidationResult,
    RpkiValidationState, DEFAULT_RPKI_CACHE_TTL,
};

// Pfx2as repository (SQLite-based cache)
pub use monocle::{
    Pfx2asCacheDbMetadata, Pfx2asDbRecord, Pfx2asQueryResult, Pfx2asRepository,
    Pfx2asSchemaDefinitions, ValidationStats, DEFAULT_PFX2AS_CACHE_TTL,
};

// Session types (SQLite-based for search result exports)
// Requires lens-bgpkit feature because MsgStore depends on bgpkit_parser::BgpElem
#[cfg(feature = "lens-bgpkit")]
pub use session::MsgStore;

// =============================================================================
// Helper function
// =============================================================================

/// Ensure the data directory exists
pub fn ensure_data_dir(data_dir: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create data directory '{}': {}", data_dir, e))
}
