//! Pfx2as Search Example
//!
//! This example demonstrates using the Pfx2asLens for prefix-to-ASN mapping
//! operations, including search by prefix, search by ASN, and RPKI validation.
//!
//! # Feature Requirements
//!
//! This example requires the `lens-bgpkit` feature for full functionality,
//! but the basic database operations work with just `database`.
//!
//! # Running
//!
//! ```bash
//! cargo run --example pfx2as_search --features lens-bgpkit
//! ```
//!
//! Note: First run may take time to download pfx2as and RPKI data.

use monocle::database::MonocleDatabase;

fn main() -> anyhow::Result<()> {
    println!("=== Monocle Pfx2as Search Example ===\n");

    // Example 1: Check repository status with in-memory database
    println!("1. Repository status (in-memory database):");
    let db = MonocleDatabase::open_in_memory()?;
    let pfx2as = db.pfx2as();
    println!("   Repository is empty: {}", pfx2as.is_empty());
    println!("   Needs refresh: {}", db.needs_pfx2as_refresh());

    // Example 2: Understanding the data model
    println!("\n2. Pfx2as data model:");
    println!("   The Pfx2as repository stores prefix-to-ASN mappings:");
    println!("   - prefix: IP prefix (e.g., '1.1.1.0/24')");
    println!("   - origin_asn: The ASN announcing this prefix");
    println!("   - validation: RPKI validation status ('valid', 'invalid', 'unknown')");
    println!("\n   Prefixes are stored with blob-based range queries for efficient lookups:");
    println!("   - Exact match: Find prefixes that exactly match");
    println!("   - Longest match: Find the most specific covering prefix");
    println!("   - Covering: Find all super-prefixes (less specific)");
    println!("   - Covered: Find all sub-prefixes (more specific)");

    // Example 3: Query API overview (with empty database)
    println!("\n3. Low-level query API methods (showing with empty database):");

    // Exact lookup
    let results = pfx2as.lookup_exact("1.1.1.0/24")?;
    println!("   lookup_exact('1.1.1.0/24'): {} ASNs", results.len());

    // Longest prefix match
    let result = pfx2as.lookup_longest("1.1.1.1/32")?;
    println!(
        "   lookup_longest('1.1.1.1/32'): {} ASNs",
        result.origin_asns.len()
    );

    // Covering prefixes (supernets)
    let results = pfx2as.lookup_covering("1.1.1.0/24")?;
    println!(
        "   lookup_covering('1.1.1.0/24'): {} prefixes",
        results.len()
    );

    // Covered prefixes (subnets)
    let results = pfx2as.lookup_covered("1.0.0.0/8")?;
    println!("   lookup_covered('1.0.0.0/8'): {} prefixes", results.len());

    // Get prefixes by ASN
    let records = pfx2as.get_by_asn(13335)?;
    println!("   get_by_asn(13335): {} prefixes", records.len());

    // Example 4: Using the lens for high-level operations
    println!("\n4. High-level Pfx2asLens API (requires lens-bgpkit feature):");
    println!("   The Pfx2asLens provides:");
    println!("   - search(): Auto-detect query type (ASN or prefix)");
    println!("   - search_by_asn(): Get all prefixes for an ASN with RPKI validation");
    println!("   - search_by_prefix(): Get origin ASNs with sub/super prefix options");
    println!("   - RPKI validation integration");
    println!("   - AS name enrichment (from ASInfo database)");

    // Example 5: Working with populated data (simulation)
    println!("\n5. Working with populated data:");
    println!("   In a real application, you would:");
    println!("   a) Use MonocleDatabase::open_in_dir(\"~/.monocle\")");
    println!("   b) Check lens.needs_refresh() and call lens.refresh() if needed");
    println!("   c) Use lens.search() for high-level queries");

    println!("\n   Example code (search by prefix):");
    println!("   ```rust");
    println!("   use monocle::database::MonocleDatabase;");
    println!("   use monocle::lens::pfx2as::{{Pfx2asLens, Pfx2asSearchArgs}};");
    println!();
    println!("   let db = MonocleDatabase::open_in_dir(\"~/.monocle\")?;");
    println!("   let lens = Pfx2asLens::new(&db);");
    println!();
    println!("   // Ensure data is available");
    println!("   if lens.needs_refresh()? {{");
    println!("       lens.refresh(None)?;");
    println!("   }}");
    println!();
    println!("   // Search by prefix with RPKI validation and AS names");
    println!("   let args = Pfx2asSearchArgs::new(\"1.1.1.0/24\")");
    println!("       .with_show_name(true);");
    println!("   let results = lens.search(&args)?;");
    println!();
    println!("   for r in &results {{");
    println!("       println!(\"{{}} -> AS{{}} ({{}})\", r.prefix, r.origin_asn, r.rpki);");
    println!("   }}");
    println!("   ```");

    println!("\n   Example code (search by ASN):");
    println!("   ```rust");
    println!("   // Search by ASN - get all prefixes announced by an AS");
    println!("   let args = Pfx2asSearchArgs::new(\"13335\")");
    println!("       .with_show_name(true)");
    println!("       .with_limit(10);");
    println!("   let results = lens.search(&args)?;");
    println!();
    println!("   for r in &results {{");
    println!("       println!(\"{{}} ({{}})\", r.prefix, r.rpki);");
    println!("   }}");
    println!("   ```");

    println!("\n   Example code (include sub/super prefixes):");
    println!("   ```rust");
    println!("   // Search with sub-prefixes (more specific)");
    println!("   let args = Pfx2asSearchArgs::new(\"8.0.0.0/8\")");
    println!("       .with_include_sub(true)");
    println!("       .with_limit(20);");
    println!("   let results = lens.search(&args)?;");
    println!();
    println!("   // Search with super-prefixes (less specific)");
    println!("   let args = Pfx2asSearchArgs::new(\"1.1.1.0/24\")");
    println!("       .with_include_super(true);");
    println!("   let results = lens.search(&args)?;");
    println!("   ```");

    // Example 6: Query type detection
    println!("\n6. Query type auto-detection:");
    println!("   The lens automatically detects query type:");
    println!("   - '13335' or 'AS13335' -> ASN query");
    println!("   - '1.1.1.0/24' -> Prefix query");
    println!("   - '2001:db8::/32' -> IPv6 prefix query");

    // Example 7: Statistics
    println!("\n7. Database statistics:");
    let record_count = pfx2as.record_count()?;
    let prefix_count = pfx2as.prefix_count()?;
    println!("   Total records: {}", record_count);
    println!("   Unique prefixes: {}", prefix_count);

    let stats = pfx2as.validation_stats()?;
    println!("   Validation stats:");
    println!("     - Valid: {}", stats.valid);
    println!("     - Invalid: {}", stats.invalid);
    println!("     - Unknown: {}", stats.unknown);

    // Example 8: Raw repository access
    println!("\n8. Accessing raw connection for custom queries:");
    let conn = db.connection();

    // Check what tables exist
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'pfx2as%'")?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    println!("   Pfx2as tables in database:");
    for table in &tables {
        println!("     - {}", table);
    }

    // Example 9: Best practices
    println!("\n9. Best practices:");
    println!("   - Check needs_refresh() before querying to ensure fresh data");
    println!("   - Use search() for most use cases (handles query type detection)");
    println!("   - Use with_limit() for large result sets");
    println!("   - RPKI validation requires RPKI data to be loaded (auto-loaded by lens)");
    println!("   - AS names require ASInfo data to be loaded");
    println!("   - For bulk operations, use the low-level repository methods");

    // Example 10: Output formatting
    println!("\n10. Output formatting:");
    println!("    The lens provides format_search_results() for various output formats:");
    println!("    - OutputFormat::Table: Pretty table with borders");
    println!("    - OutputFormat::Markdown: Markdown table");
    println!("    - OutputFormat::Json: Compact JSON");
    println!("    - OutputFormat::JsonPretty: Pretty-printed JSON");
    println!("    - OutputFormat::JsonLine: One JSON object per line");
    println!("    - OutputFormat::Psv: Pipe-separated values");

    println!("\n=== Example Complete ===");
    Ok(())
}
