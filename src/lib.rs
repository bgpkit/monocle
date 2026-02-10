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
//! Monocle uses a simplified feature system with three options:
//!
//! | Feature | Description | Implies |
//! |---------|-------------|---------|
//! | `lib` | Complete library (database + all lenses + display) | - |
//! | `server` | WebSocket server for programmatic API access | `lib` |
//! | `cli` | Full CLI binary with all functionality | `lib`, `server` |
//!
//! ## Choosing Features
//!
//! ```toml
//! # Library-only - all lenses and database operations
//! monocle = { version = "1.0", default-features = false, features = ["lib"] }
//!
//! # Library + WebSocket server
//! monocle = { version = "1.0", default-features = false, features = ["server"] }
//!
//! # Default (full CLI binary)
//! monocle = "1.0"
//! ```
//!
//! # Architecture
//!
//! The library is organized into the following modules:
//!
//! - **[`database`]**: Database functionality (requires `lib` feature)
//!   - `core`: SQLite connection management and schema definitions
//!   - `session`: One-time storage (e.g., search results)
//!   - `monocle`: Main monocle database (ASInfo, AS2Rel, RPKI, Pfx2as)
//!
//! - **[`lens`]**: High-level business logic (requires `lib` feature)
//!   - `time`: Time parsing and formatting
//!   - `country`: Country code/name lookup
//!   - `ip`: IP information lookup
//!   - `parse`: MRT file parsing
//!   - `search`: BGP message search
//!   - `rpki`: RPKI validation and data
//!   - `pfx2as`: Prefix-to-ASN mapping
//!   - `as2rel`: AS-level relationships
//!   - `inspect`: Unified AS/prefix lookup
//!
//! - **[`server`]**: WebSocket API server (requires `server` feature)
//!
//! - **[`config`]**: Configuration management (always available)
//!
//! # Quick Start Examples
//!
//! ## Database Operations (feature = "lib")
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//!
//! // Open or create database
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//!
//! // Check if AS2Rel data needs update
//! use std::time::Duration;
//! let ttl = Duration::from_secs(24 * 60 * 60); // 24 hours
//! if db.needs_as2rel_refresh(ttl) {
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
//! ## Time Parsing (feature = "lib")
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
//! ## RPKI Validation (feature = "lib")
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
//! ## MRT Parsing with Progress (feature = "lib")
//!
//! ```rust,ignore
//! use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
//! use std::sync::Arc;
//!
//! let lens = ParseLens::new();
//! let filters = ParseFilters {
//!     origin_asn: vec!["13335".to_string()],
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
//! ## Unified Inspection (feature = "lib")
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
#[cfg(feature = "lib")]
pub mod database;

// Lens module - requires lib feature
#[cfg(feature = "lib")]
pub mod lens;

// Server module - requires server feature
#[cfg(feature = "server")]
pub mod server;

// =============================================================================
// Configuration (always available)
// =============================================================================

pub use config::MonocleConfig;

// Shared database info types (used by config and database commands)
#[cfg(feature = "lib")]
pub use config::get_data_source_info;
#[cfg(feature = "lib")]
pub use config::get_sqlite_info;
pub use config::{
    format_size, get_cache_settings, CacheSettings, DataSource, DataSourceInfo, DataSourceStatus,
    SqliteDatabaseInfo,
};

// =============================================================================
// Database Module - Re-export all public types
// =============================================================================

#[cfg(feature = "lib")]
pub use database::*;

// =============================================================================
// Lens Module - Feature-gated exports
// =============================================================================

// Output format utilities (lib feature)
#[cfg(feature = "lib")]
pub use lens::utils::OutputFormat;

// =============================================================================
// Server Module (WebSocket API) - requires "server" feature
// =============================================================================

#[cfg(feature = "server")]
pub use server::{
    create_router, start_server, Dispatcher, OperationRegistry, Router, ServerConfig, ServerState,
    WsContext, WsError, WsMethod, WsRequest, WsResult, WsSink,
};
