//! DuckDB connection management
//!
//! This module provides the core DuckDB connection wrapper used throughout monocle.
//! DuckDB is used as the internal database for monocle, providing native INET type
//! support for IP/prefix operations and columnar storage for better compression.

use anyhow::{anyhow, Result};
use duckdb::{params, Connection};

/// Core DuckDB connection wrapper
///
/// `DuckDbConn` provides a thin wrapper around DuckDB connections,
/// handling both file-based and in-memory databases with consistent
/// configuration and error handling.
pub struct DuckDbConn {
    pub conn: Connection,
}

impl DuckDbConn {
    /// Open a database at the specified path
    ///
    /// If the path is `None`, an in-memory database is created.
    /// The database is configured with optimal settings for monocle's use case.
    pub fn open(path: Option<&str>) -> Result<Self> {
        let conn = match path {
            Some(p) => Connection::open(p)
                .map_err(|e| anyhow!("Failed to open DuckDB database at '{}': {}", p, e))?,
            None => Connection::open_in_memory()
                .map_err(|e| anyhow!("Failed to create in-memory DuckDB database: {}", e))?,
        };

        let db = DuckDbConn { conn };
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
        // Load the inet extension for IP address handling
        self.conn
            .execute("INSTALL inet", [])
            .or_else(|_| Ok::<_, duckdb::Error>(0))?; // Ignore if already installed

        self.conn
            .execute("LOAD inet", [])
            .map_err(|e| anyhow!("Failed to load inet extension: {}", e))?;

        // Set memory limit (default 2GB, can be overridden via config)
        // Using a reasonable default that works for most use cases
        self.conn
            .execute("SET memory_limit='2GB'", [])
            .map_err(|e| anyhow!("Failed to set memory limit: {}", e))?;

        // Enable progress bar for long-running queries (disabled by default)
        self.conn
            .execute("SET enable_progress_bar=false", [])
            .map_err(|e| anyhow!("Failed to set progress bar: {}", e))?;

        Ok(())
    }

    /// Set memory limit for the database
    pub fn set_memory_limit(&self, limit: &str) -> Result<()> {
        self.conn
            .execute(&format!("SET memory_limit='{}'", limit), [])
            .map_err(|e| anyhow!("Failed to set memory limit: {}", e))?;
        Ok(())
    }

    /// Execute a SQL statement
    pub fn execute(&self, sql: &str) -> Result<usize> {
        self.conn
            .execute(sql, [])
            .map_err(|e| anyhow!("Failed to execute SQL: {}", e))
    }

    /// Execute a SQL statement with parameters
    pub fn execute_with_params<P: duckdb::Params>(&self, sql: &str, params: P) -> Result<usize> {
        self.conn
            .execute(sql, params)
            .map_err(|e| anyhow!("Failed to execute SQL with params: {}", e))
    }

    /// Begin a transaction
    ///
    /// DuckDB transactions work differently from SQLite. This creates
    /// a savepoint-based transaction for batch operations.
    pub fn transaction(&self) -> Result<()> {
        self.conn
            .execute("BEGIN TRANSACTION", [])
            .map_err(|e| anyhow!("Failed to begin transaction: {}", e))?;
        Ok(())
    }

    /// Commit a transaction
    pub fn commit(&self) -> Result<()> {
        self.conn
            .execute("COMMIT", [])
            .map_err(|e| anyhow!("Failed to commit transaction: {}", e))?;
        Ok(())
    }

    /// Rollback a transaction
    pub fn rollback(&self) -> Result<()> {
        self.conn
            .execute("ROLLBACK", [])
            .map_err(|e| anyhow!("Failed to rollback transaction: {}", e))?;
        Ok(())
    }

    /// Check if a table exists in the database
    pub fn table_exists(&self, table_name: &str) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ? AND table_schema = 'main'",
            )
            .map_err(|e| anyhow!("Failed to prepare table existence check: {}", e))?;

        let count: i32 = stmt
            .query_row(params![table_name], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to check table existence: {}", e))?;

        Ok(count > 0)
    }

    /// Get the row count for a table
    pub fn table_count(&self, table_name: &str) -> Result<u64> {
        let query = format!("SELECT COUNT(*) FROM {}", table_name);
        let mut stmt = self
            .conn
            .prepare(&query)
            .map_err(|e| anyhow!("Failed to prepare count query: {}", e))?;

        let count: u64 = stmt
            .query_row([], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to get table count: {}", e))?;

        Ok(count)
    }

    /// Prepare a statement for execution
    pub fn prepare(&self, sql: &str) -> Result<duckdb::Statement<'_>> {
        self.conn
            .prepare(sql)
            .map_err(|e| anyhow!("Failed to prepare statement: {}", e))
    }

    /// Execute a batch of SQL statements (for schema initialization)
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        // DuckDB doesn't have execute_batch, so we split by semicolon
        for statement in sql.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                self.conn.execute(trimmed, []).map_err(|e| {
                    anyhow!("Failed to execute batch statement '{}': {}", trimmed, e)
                })?;
            }
        }
        Ok(())
    }

    /// Get a single value from a query
    pub fn query_row<T, F>(&self, sql: &str, f: F) -> Result<T>
    where
        F: FnOnce(&duckdb::Row<'_>) -> std::result::Result<T, duckdb::Error>,
    {
        let mut stmt = self.conn.prepare(sql)?;
        stmt.query_row([], f)
            .map_err(|e| anyhow!("Failed to query row: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let db = DuckDbConn::open_in_memory();
        assert!(db.is_ok());
    }

    #[test]
    fn test_execute() {
        let db = DuckDbConn::open_in_memory().unwrap();
        let result = db.execute("CREATE TABLE test (id INTEGER PRIMARY KEY)");
        assert!(result.is_ok());
    }

    #[test]
    fn test_table_exists() {
        let db = DuckDbConn::open_in_memory().unwrap();
        db.execute("CREATE TABLE test_table (id INTEGER PRIMARY KEY)")
            .unwrap();

        assert!(db.table_exists("test_table").unwrap());
        assert!(!db.table_exists("nonexistent_table").unwrap());
    }

    #[test]
    fn test_table_count() {
        let db = DuckDbConn::open_in_memory().unwrap();
        db.execute("CREATE TABLE test_table (id INTEGER PRIMARY KEY)")
            .unwrap();
        db.execute("INSERT INTO test_table (id) VALUES (1), (2), (3)")
            .unwrap();

        assert_eq!(db.table_count("test_table").unwrap(), 3);
    }

    #[test]
    fn test_inet_extension() {
        let db = DuckDbConn::open_in_memory().unwrap();

        // Test that inet extension is loaded and working
        db.execute("CREATE TABLE test_inet (ip INET, prefix INET)")
            .unwrap();

        db.execute("INSERT INTO test_inet VALUES ('192.168.1.1'::INET, '10.0.0.0/8'::INET)")
            .unwrap();

        let count = db.table_count("test_inet").unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_inet_containment() {
        let db = DuckDbConn::open_in_memory().unwrap();

        db.execute("CREATE TABLE prefixes (prefix INET)").unwrap();
        db.execute(
            "INSERT INTO prefixes VALUES
             ('10.0.0.0/8'::INET),
             ('10.1.0.0/16'::INET),
             ('10.1.1.0/24'::INET),
             ('192.168.0.0/16'::INET)",
        )
        .unwrap();

        // Test sub-prefix query (<<= operator)
        let mut stmt = db
            .conn
            .prepare("SELECT COUNT(*) FROM prefixes WHERE prefix <<= '10.0.0.0/8'::INET")
            .unwrap();
        let count: i32 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 3); // /8, /16, and /24 are all sub-prefixes of /8

        // Test super-prefix query (>>= operator)
        let mut stmt = db
            .conn
            .prepare("SELECT COUNT(*) FROM prefixes WHERE prefix >>= '10.1.1.0/24'::INET")
            .unwrap();
        let count: i32 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(count, 3); // /8, /16, and /24 all contain /24
    }

    #[test]
    fn test_transaction() {
        let db = DuckDbConn::open_in_memory().unwrap();
        db.execute("CREATE TABLE test_tx (id INTEGER)").unwrap();

        // Test successful transaction
        db.transaction().unwrap();
        db.execute("INSERT INTO test_tx VALUES (1)").unwrap();
        db.commit().unwrap();

        assert_eq!(db.table_count("test_tx").unwrap(), 1);

        // Test rollback
        db.transaction().unwrap();
        db.execute("INSERT INTO test_tx VALUES (2)").unwrap();
        db.rollback().unwrap();

        assert_eq!(db.table_count("test_tx").unwrap(), 1);
    }

    #[test]
    fn test_execute_batch() {
        let db = DuckDbConn::open_in_memory().unwrap();

        let sql = r#"
            CREATE TABLE batch_test1 (id INTEGER);
            CREATE TABLE batch_test2 (name TEXT);
            INSERT INTO batch_test1 VALUES (1);
        "#;

        db.execute_batch(sql).unwrap();

        assert!(db.table_exists("batch_test1").unwrap());
        assert!(db.table_exists("batch_test2").unwrap());
        assert_eq!(db.table_count("batch_test1").unwrap(), 1);
    }

    #[test]
    fn test_set_memory_limit() {
        let db = DuckDbConn::open_in_memory().unwrap();
        let result = db.set_memory_limit("1GB");
        assert!(result.is_ok());
    }
}
