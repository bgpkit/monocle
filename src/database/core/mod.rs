//! Core database infrastructure
//!
//! This module provides the foundational database components used throughout monocle:
//! - `DatabaseConn`: Core SQLite connection wrapper with configuration
//! - `SchemaManager`: Schema initialization and management
//! - `SchemaStatus`: Schema state enumeration

mod connection;
mod schema;

pub use connection::DatabaseConn;
pub use schema::{SchemaDefinitions, SchemaManager, SchemaStatus, SCHEMA_VERSION};
