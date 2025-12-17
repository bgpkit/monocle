//! Unified Inspection Example
//!
//! This example demonstrates using InspectLens for unified AS and prefix
//! information lookup, which aggregates data from multiple sources:
//! - ASInfo (core AS information, AS2Org, PeeringDB, hegemony, population)
//! - AS2Rel (AS-level relationships and connectivity)
//! - RPKI (ROAs and ASPAs)
//! - Pfx2as (prefix-to-ASN mappings)
//!
//! # Feature Requirements
//!
//! This example requires the `lens-full` feature, which includes all
//! lens functionality and dependencies.
//!
//! # Running
//!
//! ```bash
//! cargo run --example inspect_unified --features lens-full
//! ```
//!
//! Note: This example requires network access to fetch data on first run.
//! It may take a minute to bootstrap all data sources.

use monocle::database::MonocleDatabase;
use monocle::lens::inspect::{
    InspectDataSection, InspectDisplayConfig, InspectLens, InspectQueryOptions,
};
use std::collections::HashSet;

fn main() -> anyhow::Result<()> {
    println!("=== Monocle Unified Inspection Example ===\n");

    // Create an in-memory database for this example
    // In production, use MonocleDatabase::open_in_dir("~/.monocle")
    println!("Creating database...");
    let db = MonocleDatabase::open_in_memory()?;
    let lens = InspectLens::new(&db);

    // Example 1: Check data availability
    println!("\n1. Data availability:");
    println!("   ASInfo data available: {}", lens.is_data_available());
    println!("   Needs bootstrap: {}", lens.needs_bootstrap());
    println!("   Needs refresh: {}", lens.needs_refresh());

    // Example 2: Ensure data is available (bootstrap/refresh as needed)
    println!("\n2. Ensuring data availability...");
    println!("   (This may take a moment on first run)");
    match lens.ensure_data_available() {
        Ok(summary) => {
            println!("   Data refresh summary:");
            for msg in summary.format_messages() {
                println!("     {}", msg);
            }
            if !summary.any_refreshed {
                println!("     All data sources were up to date");
            }
        }
        Err(e) => {
            println!("   Warning: Could not ensure data availability: {}", e);
            println!("   (Continuing with available data...)");
        }
    }

    // Example 3: Query type detection
    println!("\n3. Query type detection:");
    let test_queries = [
        "13335",
        "AS13335",
        "as15169",
        "1.1.1.0/24",
        "8.8.8.0/24",
        "2001:4860::/32",
        "cloudflare",
        "google",
    ];

    for query in &test_queries {
        let query_type = lens.detect_query_type(query);
        println!("   '{}' -> {:?}", query, query_type);
    }

    // Example 4: Understanding query options
    println!("\n4. InspectQueryOptions:");
    println!("   Available data sections:");
    for section in InspectDataSection::all() {
        println!("     - {:?}", section);
    }

    // Default options
    let default_options = InspectQueryOptions::default();
    println!("\n   Default options:");
    println!("     max_roas: {}", default_options.max_roas);
    println!("     max_prefixes: {}", default_options.max_prefixes);
    println!("     max_neighbors: {}", default_options.max_neighbors);
    println!(
        "     max_search_results: {}",
        default_options.max_search_results
    );

    // Full options (no limits)
    let full_options = InspectQueryOptions::full();
    println!("\n   Full options (no limits):");
    println!("     max_roas: {}", full_options.max_roas);
    println!("     max_prefixes: {}", full_options.max_prefixes);

    // Example 5: Selective data sections
    println!("\n5. Selecting specific data sections:");
    let mut select = HashSet::new();
    select.insert(InspectDataSection::Basic);
    select.insert(InspectDataSection::Rpki);

    let selective_options = InspectQueryOptions {
        select: Some(select),
        ..Default::default()
    };
    println!("   Selected sections: Basic, Rpki");
    println!("   Other sections will be omitted from results");

    // Example 6: Query by ASN
    println!("\n6. Query by ASN:");
    let options = InspectQueryOptions::default();
    match lens.query_as_asn(&["13335".to_string()], &options) {
        Ok(result) => {
            println!("   Query successful!");
            println!("   Number of query results: {}", result.queries.len());

            // Format as JSON
            let json = lens.format_json(&result, true);
            println!("   JSON output (truncated):");
            for line in json.lines().take(20) {
                println!("   {}", line);
            }
            if json.lines().count() > 20 {
                println!("   ... (truncated)");
            }
        }
        Err(e) => {
            println!("   Query failed: {}", e);
        }
    }

    // Example 7: Query by prefix
    println!("\n7. Query by prefix:");
    match lens.query_as_prefix(&["1.1.1.0/24".to_string()], &options) {
        Ok(result) => {
            println!("   Query successful!");
            println!("   Number of query results: {}", result.queries.len());
        }
        Err(e) => {
            println!("   Query failed: {}", e);
        }
    }

    // Example 8: Query by name (search)
    println!("\n8. Query by name:");
    match lens.query_as_name(&["cloudflare".to_string()], &options) {
        Ok(result) => {
            println!("   Query successful!");
            println!("   Number of query results: {}", result.queries.len());
        }
        Err(e) => {
            println!("   Query failed: {}", e);
        }
    }

    // Example 9: Auto-detect query type
    println!("\n9. Auto-detect query type:");
    let queries = vec![
        "13335".to_string(),
        "1.1.1.0/24".to_string(),
        "cloudflare".to_string(),
    ];
    match lens.query(&queries, &options) {
        Ok(result) => {
            println!("   Query successful!");
            println!(
                "   Processed {} queries with {} results",
                queries.len(),
                result.queries.len()
            );
        }
        Err(e) => {
            println!("   Query failed: {}", e);
        }
    }

    // Example 10: Query by country
    println!("\n10. Query by country:");
    match lens.query_by_country("US", &options) {
        Ok(result) => {
            println!("   Query for country 'US' successful!");
            println!("   Number of ASes found: {}", result.queries.len());
        }
        Err(e) => {
            println!("   Query failed: {}", e);
        }
    }

    // Example 11: Name lookup utilities
    println!("\n11. Name lookup utilities:");
    if let Some(name) = lens.lookup_name(13335) {
        println!("   AS13335 name: {}", name);
    } else {
        println!("   AS13335 name: not found");
    }

    if let Some(org) = lens.lookup_org(13335) {
        println!("   AS13335 org: {}", org);
    } else {
        println!("   AS13335 org: not found");
    }

    // Example 12: Output formatting
    println!("\n12. Output formatting:");
    println!("   Available formats:");
    println!("   - format_json(&result, pretty): JSON output");
    println!("   - format_table(&result, &config): Table output (requires display feature)");

    let config = InspectDisplayConfig::auto();
    println!("\n   Display config:");
    println!("   - terminal_width: {:?}", config.terminal_width);
    println!("   - truncate_names: {}", config.truncate_names);

    // Example 13: Best practices
    println!("\n13. Best practices:");
    println!("   - Call ensure_data_available() before queries");
    println!("   - Use selective options to reduce data transfer");
    println!("   - Cache InspectLens instance - it reuses database connection");
    println!("   - For bulk lookups, use query() with multiple items");
    println!("   - Use format_json() for API responses");

    // Example 14: Data sources
    println!("\n14. Data sources aggregated by InspectLens:");
    println!("   - ASInfo: Core AS information (name, country)");
    println!("   - AS2Org: Organization mapping from CAIDA");
    println!("   - PeeringDB: Network information");
    println!("   - Hegemony: IHR AS hegemony scores");
    println!("   - Population: APNIC user population estimates");
    println!("   - AS2Rel: AS-level relationships");
    println!("   - RPKI: ROAs and ASPAs");
    println!("   - Pfx2as: Prefix-to-ASN mappings");

    println!("\n=== Example Complete ===");
    Ok(())
}
