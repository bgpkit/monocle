#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

//! Monocle - A BGP information toolkit
//!
//! Monocle provides tools for searching, parsing, and processing BGP information
//! from public sources. It can be used as both a command-line application and
//! a library.
//!
//! # Architecture
//!
//! The library is organized into the following modules:
//!
//! - **[`database`]**: All database functionality
//!   - `core`: SQLite connection management and schema definitions
//!   - `session`: One-time storage (e.g., search results)
//!   - `monocle`: Main monocle database (AS2Org, AS2Rel) and file caches
//!
//! - **[`lens`]**: High-level business logic (reusable across CLI, API, GUI)
//!   - `as2org`: AS-to-Organization lookup lens
//!   - `as2rel`: AS-level relationships lens
//!   - `country`: Country code/name lookup lens
//!   - `ip`: IP information lookup lens
//!   - `parse`: MRT file parsing lens
//!   - `pfx2as`: Prefix-to-ASN mapping lens
//!   - `rpki`: RPKI validation and data lens
//!   - `search`: BGP message search lens
//!   - `time`: Time parsing and formatting lens
//!
//! - **[`config`]**: Configuration management and shared types
//!   - `MonocleConfig`: Main configuration struct
//!   - `DataSource`: Available data sources for refresh operations
//!   - Database info types for status reporting
//!
//! # Database Strategy
//!
//! Monocle uses SQLite for AS2Org and AS2Rel data storage. For data requiring
//! INET operations (prefix matching, containment queries), file-based JSON
//! caching is used since SQLite doesn't natively support these operations.
//!
//! # Features
//!
//! Monocle supports the following Cargo features:
//!
//! - **`cli`** (default): Enables CLI support including clap derives for argument
//!   structs, progress bars, and table formatting. Required for building the binary.
//!
//! - **`full`**: Enables all features (currently same as `cli`).
//!
//! ## Feature Usage
//!
//! ```toml
//! # Minimal library (no CLI dependencies)
//! monocle = { version = "0.9", default-features = false }
//!
//! # Library with CLI argument structs (for building CLI tools)
//! monocle = { version = "0.9", features = ["cli"] }
//!
//! # Full build (default, same as just "monocle")
//! monocle = { version = "0.9" }
//! ```
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//! use monocle::lens::as2org::{As2orgLens, As2orgSearchArgs, As2orgOutputFormat};
//!
//! // Open the monocle database
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//!
//! // Create a lens and search
//! let lens = As2orgLens::new(&db);
//!
//! // Bootstrap data if needed
//! if lens.needs_bootstrap() {
//!     lens.bootstrap()?;
//! }
//!
//! // Search
//! let args = As2orgSearchArgs::new("cloudflare");
//! let results = lens.search(&args)?;
//!
//! // Format output
//! let output = lens.format_results(&results, &As2orgOutputFormat::Json, false);
//! ```
//!
//! # Example: Using Lenses
//!
//! All functionality is accessed through lens structs. Each lens module exports:
//! - A lens struct (the main entry point)
//! - Args structs (input parameters)
//! - Output types (return values and format enums)
//!
//! ```rust,ignore
//! use monocle::lens::time::{TimeLens, TimeParseArgs, TimeOutputFormat};
//! use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs, RpkiListArgs};
//! use monocle::lens::ip::{IpLens, IpLookupArgs};
//!
//! // Time parsing - all operations go through TimeLens
//! let time_lens = TimeLens::new();
//! let args = TimeParseArgs::new(vec!["2023-10-11T00:00:00Z".to_string()]);
//! let results = time_lens.parse(&args)?;
//! let output = time_lens.format_results(&results, &TimeOutputFormat::Table);
//!
//! // RPKI validation - all operations go through RpkiLens
//! let rpki_lens = RpkiLens::new();
//! let args = RpkiValidationArgs::new(13335, "1.1.1.0/24");
//! let (validity, covering_roas) = rpki_lens.validate(&args)?;
//!
//! // List ROAs for an ASN
//! let args = RpkiListArgs::for_asn(13335);
//! let roas = rpki_lens.list_roas(&args)?;
//!
//! // IP lookup - all operations go through IpLens
//! let ip_lens = IpLens::new();
//! let args = IpLookupArgs::new("1.1.1.1".parse().unwrap());
//! let info = ip_lens.lookup(&args)?;
//! ```
//!
//! # Example: Using File Caches
//!
//! For RPKI and Pfx2as data that require prefix operations:
//!
//! ```rust,ignore
//! use monocle::database::{RpkiFileCache, Pfx2asFileCache, DEFAULT_RPKI_TTL};
//!
//! // RPKI cache
//! let rpki_cache = RpkiFileCache::new("~/.monocle")?;
//! if !rpki_cache.is_fresh("cloudflare", None, DEFAULT_RPKI_TTL) {
//!     // Load and cache new data
//!     rpki_cache.store("cloudflare", None, roas, aspas)?;
//! }
//!
//! // Pfx2as cache
//! let pfx2as_cache = Pfx2asFileCache::new("~/.monocle")?;
//! let data = pfx2as_cache.load("source")?;
//! ```
//!
//! # Progress Tracking
//!
//! For long-running operations like parsing and searching, monocle provides
//! progress tracking through callbacks. This is useful for building responsive
//! GUI applications or showing progress bars in CLI tools.
//!
//! ```rust,ignore
//! use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
//! use std::sync::Arc;
//!
//! let lens = ParseLens::new();
//! let filters = ParseFilters::default();
//!
//! // Define a progress callback
//! let callback = Arc::new(|progress: ParseProgress| {
//!     match progress {
//!         ParseProgress::Started { file_path } => {
//!             println!("Started parsing: {}", file_path);
//!         }
//!         ParseProgress::Update { messages_processed, .. } => {
//!             println!("Processed {} messages", messages_processed);
//!         }
//!         ParseProgress::Completed { total_messages, duration_secs } => {
//!             println!("Completed: {} messages in {:.2}s", total_messages, duration_secs);
//!         }
//!     }
//! });
//!
//! // Parse with progress tracking
//! let elems = lens.parse_with_progress(&filters, "path/to/file.mrt", Some(callback))?;
//! for elem in elems {
//!     // Process each BGP element
//! }
//! ```

pub mod config;
pub mod database;
pub mod lens;

#[cfg(feature = "cli")]
pub mod server;

// =============================================================================
// Configuration
// =============================================================================

pub use config::MonocleConfig;

// Shared database info types (used by config and database commands)
pub use config::{
    format_size, get_cache_info, get_cache_settings, get_data_source_info, get_sqlite_info,
    CacheInfo, CacheSettings, DataSource, DataSourceInfo, DataSourceStatus, SqliteDatabaseInfo,
};

// =============================================================================
// Database Module - Re-export commonly used types
// =============================================================================

// Primary database type (SQLite)
pub use database::MonocleDatabase;

// File-based caches for RPKI and Pfx2as
pub use database::{Pfx2asFileCache, RpkiFileCache};

// =============================================================================
// Common Types
// =============================================================================

// Unified output format for all commands
pub use lens::utils::OutputFormat;

// =============================================================================
// Server Module (WebSocket API) - requires "server" feature
// =============================================================================

#[cfg(feature = "cli")]
pub use server::{
    create_router, start_server, Dispatcher, OperationRegistry, Router, ServerConfig, ServerState,
    WsContext, WsError, WsMethod, WsRequest, WsResult, WsSink,
};
