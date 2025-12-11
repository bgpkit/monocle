//! Database connection management
//!
//! This module provides the core database connection wrapper used throughout monocle.

use anyhow::{anyhow, Result};
use rusqlite::Connection;

/// Core database connection wrapper
///
/// `DatabaseConn` provides a thin wrapper around SQLite connections,
/// handling both file-based and in-memory databases with consistent
/// configuration and error handling.
pub struct DatabaseConn {
    pub conn: Connection,
}

impl DatabaseConn {
    /// Open a database at the specified path
    ///
    /// If the path is `None`, an in-memory database is created.
    /// The database is configured with optimal settings for monocle's use case.
    pub fn open(path: Option<&str>) -> Result<Self> {
        let conn = match path {
            Some(p) => Connection::open(p)
                .map_err(|e| anyhow!("Failed to open database at '{}': {}", p, e))?,
            None => Connection::open_in_memory()
                .map_err(|e| anyhow!("Failed to create in-memory database: {}", e))?,
        };

        let db = DatabaseConn { conn };
        db.configure()?;
        Ok(db)
    }

    /// Create a new database connection (backward-compatible signature)
    ///
    /// This method accepts `&Option<String>` for compatibility with existing code.
    /// Prefer using `open()` with `Option<&str>` for new code.
    pub fn new(path: &Option<String>) -> Result<Self> {
        Self::open(path.as_deref())
    }

    /// Open a database at the specified path (convenience method)
    pub fn open_path(path: &str) -> Result<Self> {
        Self::open(Some(path))
    }

    /// Create an in-memory database
    pub fn open_in_memory() -> Result<Self> {
        Self::open(None)
    }

    /// Configure the database with optimal settings
    fn configure(&self) -> Result<()> {
        // Enable WAL mode for better concurrent read/write performance
        let _: String = self
            .conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to set journal mode: {}", e))?;

        // Use NORMAL synchronous mode (good balance of safety and performance)
        self.conn
            .execute("PRAGMA synchronous=NORMAL", [])
            .map_err(|e| anyhow!("Failed to set synchronous mode: {}", e))?;

        // Increase cache size for better performance (100MB)
        self.conn
            .execute("PRAGMA cache_size=100000", [])
            .map_err(|e| anyhow!("Failed to set cache size: {}", e))?;

        // Store temp tables in memory
        self.conn
            .execute("PRAGMA temp_store=MEMORY", [])
            .map_err(|e| anyhow!("Failed to set temp store: {}", e))?;

        // Enable foreign keys
        self.conn
            .execute("PRAGMA foreign_keys=ON", [])
            .map_err(|e| anyhow!("Failed to enable foreign keys: {}", e))?;

        Ok(())
    }

    /// Execute a SQL statement
    pub fn execute(&self, sql: &str) -> Result<usize> {
        self.conn
            .execute(sql, [])
            .map_err(|e| anyhow!("Failed to execute SQL: {}", e))
    }

    /// Execute a SQL statement with parameters
    pub fn execute_with_params<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<usize> {
        self.conn
            .execute(sql, params)
            .map_err(|e| anyhow!("Failed to execute SQL with params: {}", e))
    }

    /// Begin an unchecked transaction
    ///
    /// This is useful for batch operations where we want to commit
    /// multiple statements atomically.
    pub fn transaction(&self) -> Result<rusqlite::Transaction<'_>> {
        self.conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("Failed to begin transaction: {}", e))
    }

    /// Check if a table exists in the database
    pub fn table_exists(&self, table_name: &str) -> Result<bool> {
        let count: i32 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table_name],
                |row| row.get(0),
            )
            .map_err(|e| anyhow!("Failed to check table existence: {}", e))?;
        Ok(count > 0)
    }

    /// Get the row count for a table
    pub fn table_count(&self, table_name: &str) -> Result<u64> {
        let query = format!("SELECT COUNT(*) FROM {}", table_name);
        let count: u64 = self
            .conn
            .query_row(&query, [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to get table count: {}", e))?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = DatabaseConn::open_in_memory();
        assert!(db.is_ok());
    }

    #[test]
    fn test_execute() {
        let db = DatabaseConn::open_in_memory().unwrap();
        let result = db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)");
        assert!(result.is_ok());
    }

    #[test]
    fn test_table_exists() {
        let db = DatabaseConn::open_in_memory().unwrap();
        db.execute("CREATE TABLE test_table (id INTEGER PRIMARY KEY)")
            .unwrap();

        assert!(db.table_exists("test_table").unwrap());
        assert!(!db.table_exists("nonexistent_table").unwrap());
    }

    #[test]
    fn test_table_count() {
        let db = DatabaseConn::open_in_memory().unwrap();
        db.execute("CREATE TABLE test_table (id INTEGER PRIMARY KEY)")
            .unwrap();
        db.execute("INSERT INTO test_table (id) VALUES (1), (2), (3)")
            .unwrap();

        assert_eq!(db.table_count("test_table").unwrap(), 3);
    }
}
