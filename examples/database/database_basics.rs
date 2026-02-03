//! Database Basics Example
//!
//! This example demonstrates using MonocleDatabase for SQLite operations
//! without requiring bgpkit-* dependencies.
//!
//! # Feature Requirements
//!
//! This example only requires the `database` feature, which has minimal
//! dependencies (rusqlite, serde, chrono).
//!
//! # Running
//!
//! ```bash
//! cargo run --example database_basics --features database
//! ```

use monocle::database::{DatabaseConn, MonocleDatabase, SchemaManager, SchemaStatus};

fn main() -> anyhow::Result<()> {
    println!("=== Monocle Database Basics Example ===\n");

    // Example 1: Create an in-memory database
    println!("1. Creating in-memory database:");
    let db = MonocleDatabase::open_in_memory()?;
    println!("   Database created successfully");

    // Example 2: Check database repositories
    println!("\n2. Checking database repositories:");
    println!("   AS2Rel is empty: {}", db.as2rel().is_empty());
    println!("   ASInfo is empty: {}", db.asinfo().is_empty());
    println!("   RPKI is empty: {}", db.rpki().is_empty());
    println!("   Pfx2as is empty: {}", db.pfx2as().is_empty());

    // Example 3: Check if updates are needed (with TTL)
    println!("\n3. Checking update status (with configurable TTL):");
    use std::time::Duration;
    let ttl = Duration::from_secs(24 * 60 * 60); // 24 hours
    println!("   Needs ASInfo bootstrap: {}", db.needs_asinfo_bootstrap());
    println!(
        "   Needs ASInfo refresh (24h TTL): {}",
        db.needs_asinfo_refresh(ttl)
    );
    println!(
        "   Needs AS2Rel refresh (24h TTL): {}",
        db.needs_as2rel_refresh(ttl)
    );
    println!(
        "   Needs RPKI refresh (24h TTL): {}",
        db.needs_rpki_refresh(ttl)
    );
    println!(
        "   Needs Pfx2as refresh (24h TTL): {}",
        db.needs_pfx2as_refresh(ttl)
    );

    // Example 4: Working with metadata
    println!("\n4. Working with metadata:");
    db.set_meta("example_key", "example_value")?;
    let value = db.get_meta("example_key")?;
    println!("   Set 'example_key' to 'example_value'");
    println!("   Retrieved: {:?}", value);

    // Example 5: Using DatabaseConn directly for custom queries
    println!("\n5. Using DatabaseConn for custom operations:");
    let conn = DatabaseConn::open_in_memory()?;

    // Create a custom table
    conn.execute("CREATE TABLE custom_data (id INTEGER PRIMARY KEY, name TEXT, value REAL)")?;
    println!("   Created custom table 'custom_data'");

    // Check if table exists
    let exists = conn.table_exists("custom_data")?;
    println!("   Table 'custom_data' exists: {}", exists);

    // Insert some data
    conn.execute_with_params(
        "INSERT INTO custom_data (name, value) VALUES (?1, ?2)",
        ("test_entry", 42.5),
    )?;
    println!("   Inserted test entry");

    // Check row count
    let count = conn.table_count("custom_data")?;
    println!("   Row count: {}", count);

    // Example 6: Schema management
    println!("\n6. Schema management:");
    let schema_conn = DatabaseConn::open_in_memory()?;
    let schema_mgr = SchemaManager::new(&schema_conn.conn);

    let status = schema_mgr.check_status()?;
    println!("   Initial schema status: {:?}", status);

    match status {
        SchemaStatus::NotInitialized => {
            println!("   Initializing schema...");
            schema_mgr.initialize()?;
            println!("   Schema initialized successfully");
        }
        SchemaStatus::Current => {
            println!("   Schema is already current");
        }
        _ => {
            println!("   Schema needs attention: {:?}", status);
        }
    }

    // Example 7: Working with transactions
    println!("\n7. Using transactions:");
    let tx_conn = DatabaseConn::open_in_memory()?;
    tx_conn.execute("CREATE TABLE tx_test (id INTEGER PRIMARY KEY, data TEXT)")?;

    {
        let tx = tx_conn.transaction()?;

        tx.execute("INSERT INTO tx_test (data) VALUES ('item1')", [])?;
        tx.execute("INSERT INTO tx_test (data) VALUES ('item2')", [])?;
        tx.execute("INSERT INTO tx_test (data) VALUES ('item3')", [])?;

        tx.commit()?;
        println!("   Transaction committed successfully");
    }

    let final_count = tx_conn.table_count("tx_test")?;
    println!("   Final row count: {}", final_count);

    // Example 8: Accessing raw connection for advanced queries
    println!("\n8. Advanced queries with raw connection:");
    let raw_conn = db.connection();

    let mut stmt = raw_conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    println!("   Tables in database:");
    for table in &tables {
        println!("     - {}", table);
    }

    // Example 9: Database file path (persistent database)
    println!("\n9. Persistent database example (not actually created):");
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join("monocle-example.sqlite3");
    println!("   Would create database at: {}", db_path.display());
    println!("   Use MonocleDatabase::open(&path) for persistent storage");

    // Example 10: Using open_in_dir
    println!("\n10. Using open_in_dir pattern:");
    println!("   MonocleDatabase::open_in_dir(\"~/.monocle\") creates:");
    println!("   ~/.monocle/monocle-data.sqlite3");

    println!("\n=== Example Complete ===");
    Ok(())
}
