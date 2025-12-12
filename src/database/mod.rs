//! Database module
//!
//! This module provides all database functionality for monocle, organized into:
//!
//! - **core**: Core database infrastructure (connections, schema management)
//! - **session**: Session-based storage for one-time operations (e.g., search results)
//! - **monocle**: Main monocle database for persistent data (AS2Org, AS2Rel)
//!
//! # Architecture
//!
//! ```text
//! database/
//! ├── core/           # Foundation
//! │   ├── connection  # SQLite DatabaseConn wrapper (for exports)
//! │   ├── duckdb_conn # DuckDB DuckDbConn wrapper (primary backend)
//! │   ├── schema      # SQLite schema definitions and management
//! │   └── duckdb_schema # DuckDB schema definitions and management
//! │
//! ├── session/        # One-time storage
//! │   └── msg_store   # BGP message search results
//! │
//! └── monocle/        # Persistent storage
//!     ├── as2org      # AS-to-Organization mappings
//!     └── as2rel      # AS-level relationships
//! ```
//!
//! # Database Backend Strategy
//!
//! Monocle uses a dual-database approach:
//! - **DuckDB** is used as the internal database for monocle's data storage,
//!   leveraging native INET type support for IP/prefix operations and columnar
//!   storage for better compression.
//! - **SQLite** is retained for export functionality (search results) to maintain
//!   compatibility with tools that expect SQLite files.
//!
//! # Usage
//!
//! ## Monocle Database
//!
//! The monocle database is the primary interface for persistent data:
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//!
//! // Open the monocle database
//! let db = MonocleDatabase::open_in_dir("~/.monocle")?;
//!
//! // Bootstrap data if needed
//! if db.needs_as2org_bootstrap() {
//!     db.bootstrap_as2org()?;
//! }
//!
//! // Query data
//! let results = db.as2org().search_by_name("cloudflare")?;
//! ```
//!
//! ## Session Database
//!
//! For one-time operations like storing search results:
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

// Re-export commonly used types

// SQLite types (for backward compatibility and export functionality)
pub use core::{DatabaseConn, SchemaManager, SchemaStatus, SCHEMA_VERSION};

// DuckDB types (primary database backend)
pub use core::{
    DuckDbConn, DuckDbSchemaDefinitions, DuckDbSchemaManager, DuckDbSchemaStatus,
    DUCKDB_SCHEMA_VERSION,
};

// Monocle database types
pub use monocle::{
    ensure_data_dir, AggregatedRelationship, As2orgRecord, As2orgRepository, As2relEntry,
    As2relMeta, As2relRecord, As2relRepository, MonocleDatabase, BGPKIT_AS2REL_URL,
};

// Session types
pub use session::MsgStore;
