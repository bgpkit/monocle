//! Core database infrastructure
//!
//! This module provides the foundational database components used throughout monocle:
//!
//! ## SQLite (for backward compatibility and exports)
//! - `DatabaseConn`: SQLite connection wrapper with configuration
//! - `SchemaManager`: SQLite schema initialization and management
//! - `SchemaStatus`: SQLite schema state enumeration
//!
//! ## DuckDB (primary internal database)
//! - `DuckDbConn`: DuckDB connection wrapper with INET support
//! - `DuckDbSchemaManager`: DuckDB schema initialization and management
//! - `DuckDbSchemaStatus`: DuckDB schema state enumeration

mod connection;
mod duckdb_conn;
mod duckdb_query;
mod duckdb_schema;
mod schema;

// SQLite exports (for backward compatibility and SQLite export functionality)
pub use connection::DatabaseConn;
pub use schema::{SchemaDefinitions, SchemaManager, SchemaStatus, SCHEMA_VERSION};

// DuckDB exports (primary database backend)
pub use duckdb_conn::DuckDbConn;
pub use duckdb_query::{
    build_prefix_containment_clause, order_by_prefix_length, Pfx2asQuery, PrefixQueryBuilder,
    RpkiValidationQuery,
};
pub use duckdb_schema::{
    DuckDbSchemaDefinitions, DuckDbSchemaManager, DuckDbSchemaStatus, DUCKDB_SCHEMA_VERSION,
};
