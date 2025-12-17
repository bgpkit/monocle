#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

//! Monocle - A BGP information toolkit
//!
//! Monocle provides tools for searching, parsing, and processing BGP information
//! from public sources. It can be used as both a command-line application and
//! a library.
//!
//! # Feature Flags
//!
//! Monocle uses a layered feature system to minimize dependencies based on your needs:
//!
//! | Feature | Description | Key Dependencies |
//! |---------|-------------|------------------|
//! | `database` | SQLite database operations only | `rusqlite` |
//! | `lens-core` | Standalone lenses (TimeLens, OutputFormat) | `chrono-humanize`, `dateparser` |
//! | `lens-bgpkit` | BGP-related lenses (Parse, Search, RPKI, Country) | `bgpkit-*`, `rayon` |
//! | `lens-full` | All lenses including InspectLens | All above |
//! | `display` | Table formatting with `tabled` | `tabled` |
//! | `cli` | Full CLI binary with server support | All above + `clap`, `axum` |
//!
//! ## Choosing Features
//!
//! ```toml
//! # Minimal - just database operations
//! monocle = { version = "0.9", default-features = false, features = ["database"] }
//!
//! # Standalone utilities (time parsing, output formatting)
//! monocle = { version = "0.9", default-features = false, features = ["lens-core"] }
//!
//! # BGP operations without CLI overhead
//! monocle = { version = "0.9", default-features = false, features = ["lens-bgpkit"] }
//!
//! # Full lens functionality with display support
//! monocle = { version = "0.9", default-features = false, features = ["lens-full", "display"] }
//!
//! # Default (CLI binary)
//! monocle = "0.9"
//! ```
//!
//! # Architecture
//!
//! The library is organized into the following modules:
//!
//! - **[`database`]**: All database functionality (always available)
//!   - `core`: SQLite connection management and schema definitions
//!   - `session`: One-time storage (e.g., search results)
//!   - `monocle`: Main monocle database (ASInfo, AS2Rel) and file caches
//!
//! - **[`lens`]**: High-level business logic (feature-gated)
//!   - `time`: Time parsing and formatting (requires `lens-core`)
//!   - `country`: Country code/name lookup (requires `lens-bgpkit`)
//!   - `ip`: IP information lookup (requires `lens-bgpkit`)
//!   - `parse`: MRT file parsing (requires `lens-bgpkit`)
//!   - `search`: BGP message search (requires `lens-bgpkit`)
//!   - `rpki`: RPKI validation and data (requires `lens-bgpkit`)
//!   - `pfx2as`: Prefix-to-ASN mapping (requires `lens-bgpkit`)
//!   - `as2rel`: AS-level relationships (requires `lens-bgpkit`)
//!   - `inspect`: Unified AS/prefix lookup (requires `lens-full`)
//!
//! - **[`config`]**: Configuration management
//!
//! # Quick Start Examples
//!
//! ## Database Operations (feature = "database")
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//!
//! // Open or create database
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//!
//! // Check if AS2Rel data needs update
//! if db.needs_as2rel_update() {
//!     let count = db.update_as2rel()?;
//!     println!("Loaded {} relationships", count);
//! }
//!
//! // Query relationships
//! let rels = db.as2rel().search_asn(13335)?;
//! for rel in rels {
//!     println!("AS{} <-> AS{}", rel.asn1, rel.asn2);
//! }
//! ```
//!
//! ## Time Parsing (feature = "lens-core")
//!
//! ```rust,ignore
//! use monocle::lens::time::{TimeLens, TimeParseArgs};
//!
//! let lens = TimeLens::new();
//! let args = TimeParseArgs::new(vec![
//!     "1697043600".to_string(),          // Unix timestamp
//!     "2023-10-11T00:00:00Z".to_string(), // RFC3339
//! ]);
//!
//! let results = lens.parse(&args)?;
//! for t in &results {
//!     println!("{} -> {}", t.unix, t.rfc3339);
//! }
//! ```
//!
//! ## RPKI Validation (feature = "lens-bgpkit")
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//! use monocle::lens::rpki::RpkiLens;
//!
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//! let lens = RpkiLens::new(&db);
//!
//! // Ensure cache is populated
//! if lens.needs_refresh()? {
//!     lens.refresh()?;
//! }
//!
//! // Validate a prefix-ASN pair
//! let result = lens.validate("1.1.1.0/24", 13335)?;
//! println!("{}: {}", result.state, result.reason);
//! ```
//!
//! ## MRT Parsing with Progress (feature = "lens-bgpkit")
//!
//! ```rust,ignore
//! use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
//! use std::sync::Arc;
//!
//! let lens = ParseLens::new();
//! let filters = ParseFilters {
//!     origin_asn: Some(13335),
//!     ..Default::default()
//! };
//!
//! let callback = Arc::new(|progress: ParseProgress| {
//!     if let ParseProgress::Completed { total_messages, .. } = progress {
//!         println!("Parsed {} messages", total_messages);
//!     }
//! });
//!
//! let elems = lens.parse_with_progress(&filters, "path/to/file.mrt", Some(callback))?;
//! ```
//!
//! ## Unified Inspection (feature = "lens-full")
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//! use monocle::lens::inspect::{InspectLens, InspectQueryOptions};
//!
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//! let lens = InspectLens::new(&db);
//!
//! // Auto-refresh data if needed
//! lens.ensure_data_available()?;
//!
//! // Query by ASN, prefix, or name (auto-detected)
//! let options = InspectQueryOptions::default();
//! let results = lens.query(&["13335".to_string()], &options)?;
//!
//! // Get JSON output
//! let json = lens.format_json(&results, true);
//! println!("{}", json);
//! ```

pub mod config;
pub mod database;

// Lens module - feature gated
#[cfg(any(feature = "lens-core", feature = "lens-bgpkit", feature = "lens-full"))]
pub mod lens;

// Server module - requires CLI feature
#[cfg(feature = "cli")]
pub mod server;

// =============================================================================
// Configuration (always available)
// =============================================================================

pub use config::MonocleConfig;

// Shared database info types (used by config and database commands)
pub use config::{
    format_size, get_cache_info, get_cache_settings, get_data_source_info, get_sqlite_info,
    CacheInfo, CacheSettings, DataSource, DataSourceInfo, DataSourceStatus, SqliteDatabaseInfo,
};

// =============================================================================
// Database Module - Re-export commonly used types (always available)
// =============================================================================

// Primary database type (SQLite)
pub use database::MonocleDatabase;

// Core database types
pub use database::{DatabaseConn, SchemaDefinitions, SchemaManager, SchemaStatus, SCHEMA_VERSION};

// AS2Rel repository
pub use database::{
    AggregatedRelationship, As2relEntry, As2relMeta, As2relRecord, As2relRepository,
    AsConnectivitySummary, ConnectivityEntry, ConnectivityGroup, BGPKIT_AS2REL_URL,
};

// ASInfo repository
pub use database::{
    AsinfoAs2orgRecord, AsinfoCoreRecord, AsinfoFullRecord, AsinfoHegemonyRecord, AsinfoMetadata,
    AsinfoPeeringdbRecord, AsinfoPopulationRecord, AsinfoRepository, AsinfoSchemaDefinitions,
    AsinfoStoreCounts, JsonlRecord, ASINFO_DATA_URL, DEFAULT_ASINFO_TTL,
};

// RPKI repository
pub use database::{
    RpkiAspaRecord, RpkiCacheMetadata, RpkiRepository, RpkiRoaRecord, RpkiValidationResult,
    RpkiValidationState, DEFAULT_RPKI_CACHE_TTL,
};

// Pfx2as repository
pub use database::{
    Pfx2asCacheDbMetadata, Pfx2asDbRecord, Pfx2asQueryResult, Pfx2asRepository,
    Pfx2asSchemaDefinitions, ValidationStats, DEFAULT_PFX2AS_CACHE_TTL,
};

// Session types
#[cfg(feature = "lens-bgpkit")]
pub use database::MsgStore;

// File-based cache for RPKI
pub use database::RpkiFileCache;

// =============================================================================
// Lens Module - Feature-gated exports
// =============================================================================

// Output format utilities (lens-core)
#[cfg(any(feature = "lens-core", feature = "lens-bgpkit", feature = "lens-full"))]
pub use lens::utils::OutputFormat;

// =============================================================================
// Server Module (WebSocket API) - requires "cli" feature
// =============================================================================

#[cfg(feature = "cli")]
pub use server::{
    create_router, start_server, Dispatcher, OperationRegistry, Router, ServerConfig, ServerState,
    WsContext, WsError, WsMethod, WsRequest, WsResult, WsSink,
};
