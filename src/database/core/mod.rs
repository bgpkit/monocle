//! Core database infrastructure
//!
//! This module provides the foundational database components used throughout monocle:
//!
//! ## SQLite
//! - `DatabaseConn`: SQLite connection wrapper with configuration
//! - `SchemaManager`: SQLite schema initialization and management
//! - `SchemaStatus`: SQLite schema state enumeration

mod connection;
mod schema;

// SQLite exports
pub use connection::DatabaseConn;
pub use schema::{SchemaDefinitions, SchemaManager, SchemaStatus, SCHEMA_VERSION};
