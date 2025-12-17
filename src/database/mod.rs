//! Database module
//!
//! This module provides all database functionality for monocle, organized into:
//!
//! - **core**: Core database infrastructure (SQLite connections, schema management)
//! - **session**: Session-based storage for one-time operations (e.g., search results)
//! - **monocle**: Main monocle database for persistent data (AS2Org, AS2Rel, caches)
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
//! └── monocle/        # Persistent storage
//!     ├── as2org      # AS-to-Organization mappings (SQLite)
//!     ├── as2rel      # AS-level relationships (SQLite)
//!     └── file_cache  # RPKI and Pfx2as caches (JSON files)
//! ```
//!
//! # Database Backend Strategy
//!
//! Monocle uses SQLite as its database backend for AS2Org and AS2Rel data.
//! For data requiring INET operations (prefix matching, containment queries),
//! file-based JSON caching is used since SQLite doesn't natively support these.
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
//! The monocle database is the primary interface for AS2Org and AS2Rel data:
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//!
//! // Open the monocle database
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//!
//! // Bootstrap data if needed
//! if db.needs_asinfo_bootstrap() {
//!     db.bootstrap_asinfo()?;
//! }
//!
//! // Query data
//! let results = db.asinfo().search_by_name("cloudflare")?;
//! ```
//!
//! ## File-based Caching (RPKI and Pfx2as)
//!
//! For RPKI and Pfx2as data that require prefix operations:
//!
//! ```rust,ignore
//! use monocle::database::{RpkiFileCache, DEFAULT_RPKI_TTL};
//!
//! // RPKI cache
//! let rpki_cache = RpkiFileCache::new("~/.monocle")?;
//! if !rpki_cache.is_fresh("cloudflare", None, DEFAULT_RPKI_TTL) {
//!     // Load and cache new data
//!     rpki_cache.store("cloudflare", None, roas, aspas)?;
//! }
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
// File-based Cache Types (for RPKI)
// =============================================================================

// RPKI file cache
pub use monocle::{
    // Cache utilities
    cache_size,
    clear_all_caches,
    ensure_cache_dirs,
    // RPKI cache
    AspaRecord,
    RoaRecord,
    RpkiCacheData,
    RpkiCacheMeta,
    RpkiFileCache,
    // TTL defaults
    DEFAULT_RPKI_HISTORICAL_TTL,
    DEFAULT_RPKI_TTL,
};

// =============================================================================
// Helper function
// =============================================================================

/// Ensure the data directory exists
pub fn ensure_data_dir(data_dir: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create data directory '{}': {}", data_dir, e))
}
