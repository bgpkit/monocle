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
//! │   ├── connection  # DatabaseConn wrapper
//! │   └── schema      # Schema definitions and management
//! │
//! ├── session/        # One-time storage
//! │   └── msg_store   # BGP message search results
//! │
//! └── monocle/        # Persistent storage
//!     ├── as2org      # AS-to-Organization mappings
//!     └── as2rel      # AS-level relationships
//! ```
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
pub use core::{DatabaseConn, SchemaManager, SchemaStatus, SCHEMA_VERSION};
pub use monocle::{
    ensure_data_dir, AggregatedRelationship, As2orgRecord, As2orgRepository, As2relEntry,
    As2relMeta, As2relRecord, As2relRepository, MonocleDatabase, BGPKIT_AS2REL_URL,
};
pub use session::MsgStore;
