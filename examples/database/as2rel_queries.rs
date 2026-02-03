//! AS2Rel Queries Example
//!
//! This example demonstrates querying AS-level relationship data using
//! the AS2Rel repository directly from the database layer.
//!
//! # Feature Requirements
//!
//! This example only requires the `database` feature, which has minimal
//! dependencies (rusqlite, serde, chrono).
//!
//! Note: To actually query data, the database needs to be populated first.
//! This example shows how to work with the repository API even with an
//! empty database.
//!
//! # Running
//!
//! ```bash
//! cargo run --example as2rel_queries --features database
//! ```

use monocle::database::{MonocleDatabase, BGPKIT_AS2REL_URL};

fn main() -> anyhow::Result<()> {
    println!("=== Monocle AS2Rel Queries Example ===\n");

    // Create an in-memory database for demonstration
    let db = MonocleDatabase::open_in_memory()?;

    // Example 1: Check repository status
    println!("1. Repository status:");
    let as2rel = db.as2rel();
    use std::time::Duration;
    let ttl = Duration::from_secs(24 * 60 * 60); // 24 hours
    println!("   Repository is empty: {}", as2rel.is_empty());
    println!(
        "   Needs refresh (24h TTL): {}",
        db.needs_as2rel_refresh(ttl)
    );
    println!("   Data source URL: {}", BGPKIT_AS2REL_URL);

    // Example 2: Understanding the data model
    println!("\n2. AS2Rel data model:");
    println!("   The AS2Rel repository stores AS-level relationships:");
    println!("   - asn1: First AS in the relationship");
    println!("   - asn2: Second AS in the relationship");
    println!("   - rel: Relationship type (-1, 0, 1)");
    println!("     -1 = asn1 is customer of asn2");
    println!("      0 = peer-to-peer relationship");
    println!("      1 = asn1 is provider of asn2");
    println!("   - paths_count: Number of AS paths containing this relationship");
    println!("   - peers_count: Number of BGP peers observing this relationship");

    // Example 3: Metadata operations
    println!("\n3. Metadata operations:");
    let max_peers = as2rel.get_max_peers_count();
    println!(
        "   Max peers count: {} (used for percentage calculations)",
        max_peers
    );

    // Example 4: Query API overview (with empty database)
    println!("\n4. Query API methods (showing with empty database):");

    // Search by single ASN
    let results = as2rel.search_asn(13335)?;
    println!("   search_asn(13335): {} results", results.len());

    // Search by ASN pair
    let results = as2rel.search_pair(13335, 15169)?;
    println!("   search_pair(13335, 15169): {} results", results.len());

    // Search with names (joins with ASInfo if available)
    let results = as2rel.search_asn_with_names(13335)?;
    println!("   search_asn_with_names(13335): {} results", results.len());

    // Note: get_connectivity_summary requires additional parameters
    // It's typically used through higher-level APIs like InspectLens
    println!("   get_connectivity_summary: requires name lookup function (see InspectLens)");

    // Example 5: Using the repository with populated data
    println!("\n5. Working with populated data (simulation):");
    println!("   In a real application, you would:");
    println!("   a) Use MonocleDatabase::open_in_dir(\"~/.monocle\")");
    println!("   b) Call db.update_as2rel() to fetch latest data");
    println!("   c) Query using the methods shown above");
    println!("\n   Example code:");
    println!("   ```rust");
    println!("   use std::time::Duration;");
    println!("   let db = MonocleDatabase::open_in_dir(\"~/.monocle\")?;");
    println!("   let ttl = Duration::from_secs(24 * 60 * 60); // 24 hours");
    println!("   if db.needs_as2rel_refresh(ttl) {{");
    println!("       let count = db.update_as2rel()?;");
    println!("       println!(\"Loaded {{}} relationships\", count);");
    println!("   }}");
    println!("   let rels = db.as2rel().search_asn(13335)?;");
    println!("   ```");

    // Example 6: Understanding aggregated relationships
    println!("\n6. Aggregated relationships:");
    println!("   The search_asn_with_names() method returns AggregatedRelationship:");
    println!("   - asn1, asn2: The AS pair");
    println!("   - asn2_name: Name of asn2 (if available from ASInfo)");
    println!("   - connected_count: Total paths where relationship was observed");
    println!("   - as1_upstream_count: Paths where asn1 appears as upstream");
    println!("   - as2_upstream_count: Paths where asn2 appears as upstream");
    println!("\n   This allows calculating:");
    println!("   - Visibility percentage (connected_count / max_peers * 100)");
    println!("   - Relationship direction confidence");

    // Example 7: Raw repository access
    println!("\n7. Accessing raw connection for custom queries:");
    let conn = db.connection();

    // Check what tables exist
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'as2rel%'")?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    println!("   AS2Rel tables in database:");
    for table in &tables {
        println!("     - {}", table);
    }

    // Example 8: Best practices
    println!("\n8. Best practices:");
    println!("   - Check needs_as2rel_refresh(ttl) before querying to ensure fresh data");
    println!("   - Use search_asn_with_names() to get human-readable results");
    println!("   - Cache max_peers_count for percentage calculations");
    println!("   - For bulk operations, use get_connectivity_summary()");
    println!("   - The database uses WAL mode for concurrent read performance");

    // Example 9: Error handling patterns
    println!("\n9. Error handling:");
    println!("   All repository methods return anyhow::Result<T>");
    println!("   Common errors:");
    println!("   - Database not found (use open_in_dir with valid path)");
    println!("   - Schema mismatch (will auto-migrate on open)");
    println!("   - Empty database (check is_empty() before querying)");

    println!("\n=== Example Complete ===");
    Ok(())
}
