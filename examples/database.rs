//! Database Example
//!
//! Demonstrates basic database operations with MonocleDatabase.
//!
//! # Running
//!
//! ```bash
//! cargo run --example database --features lib
//! ```

use monocle::database::MonocleDatabase;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    // Open an in-memory database
    let db = MonocleDatabase::open_in_memory()?;

    // Check repository status
    println!("Database Status:");
    println!("  AS2Rel: {} records", db.as2rel().count()?);
    println!("  ASInfo: {} records", db.asinfo().core_count());

    // Check if refresh is needed
    let ttl = Duration::from_secs(24 * 60 * 60);
    if db.needs_as2rel_refresh(ttl) {
        println!("\nAS2Rel data needs refresh (older than 24h)");
    }

    // Query relationships (if data exists)
    let rels = db.as2rel().search_asn(13335)?;
    if !rels.is_empty() {
        println!("\nFound {} relationships for AS13335", rels.len());
    }

    Ok(())
}
