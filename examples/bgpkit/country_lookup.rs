//! Country Lookup Example
//!
//! This example demonstrates using CountryLens for looking up country
//! codes and names using data from bgpkit-commons.
//!
//! # Feature Requirements
//!
//! This example requires the `lens-bgpkit` feature, which includes
//! bgpkit-commons for country data.
//!
//! # Running
//!
//! ```bash
//! cargo run --example country_lookup --features lens-bgpkit
//! ```

use monocle::lens::country::{CountryEntry, CountryLens, CountryLookupArgs, CountryOutputFormat};

fn main() -> anyhow::Result<()> {
    println!("=== Monocle Country Lookup Example ===\n");

    let lens = CountryLens::new();

    // Example 1: Look up by country code
    println!("1. Look up by country code (US):");
    let args = CountryLookupArgs::new("US");
    let results = lens.search(&args)?;
    print_results(&results);

    // Example 2: Look up by country code (lowercase)
    println!("\n2. Look up by country code (lowercase 'de'):");
    let args = CountryLookupArgs::new("de");
    let results = lens.search(&args)?;
    print_results(&results);

    // Example 3: Search by partial name
    println!("\n3. Search by partial name ('united'):");
    let args = CountryLookupArgs::new("united");
    let results = lens.search(&args)?;
    print_results(&results);

    // Example 4: Search by partial name (multiple matches)
    println!("\n4. Search by partial name ('island'):");
    let args = CountryLookupArgs::new("island");
    let results = lens.search(&args)?;
    print_results(&results);

    // Example 5: Direct lookup methods
    println!("\n5. Direct lookup methods:");

    // Lookup code to get name
    if let Some(name) = lens.lookup_code("JP") {
        println!("   JP -> {}", name);
    }

    if let Some(name) = lens.lookup_code("BR") {
        println!("   BR -> {}", name);
    }

    // Non-existent code
    let result = lens.lookup_code("XX");
    println!("   XX -> {:?}", result);

    // Example 6: List all countries
    println!("\n6. List all countries (first 10):");
    let all = lens.all();
    println!("   Total countries: {}", all.len());
    for country in all.iter().take(10) {
        println!("   {} - {}", country.code, country.name);
    }
    println!("   ... and {} more", all.len().saturating_sub(10));

    // Example 7: Using all_countries() args
    println!("\n7. Using all_countries() args:");
    let args = CountryLookupArgs::all_countries();
    let results = lens.search(&args)?;
    println!("   Found {} countries using all_countries()", results.len());

    // Example 8: Different output formats
    println!("\n8. Different output formats:");

    let args = CountryLookupArgs::new("scan"); // Should find Scandinavian countries
    let results = lens.search(&args)?;

    println!("   Simple format:");
    println!(
        "{}",
        lens.format_results(&results, &CountryOutputFormat::Simple)
    );

    println!("\n   JSON format:");
    println!(
        "{}",
        lens.format_results(&results, &CountryOutputFormat::Json)
    );

    // Example 9: Using format_json convenience method
    println!("\n9. Using format_json for API responses:");
    let args = CountryLookupArgs::new("AU");
    let results = lens.search(&args)?;

    println!("   Compact JSON:");
    println!("   {}", lens.format_json(&results, false));

    println!("   Pretty JSON:");
    println!("{}", lens.format_json(&results, true));

    // Example 10: Builder pattern for args
    println!("\n10. Builder pattern for args:");
    let args = CountryLookupArgs::new("FR").with_format(CountryOutputFormat::Simple);
    let results = lens.search(&args)?;
    println!("   {}", lens.format_results(&results, &args.format));

    // Example 11: Validation
    println!("\n11. Argument validation:");

    // Valid args
    let args = CountryLookupArgs::new("US");
    match args.validate() {
        Ok(()) => println!("   Args with query: valid"),
        Err(e) => println!("   Args with query: invalid - {}", e),
    }

    // Valid args (all flag)
    let args = CountryLookupArgs::all_countries();
    match args.validate() {
        Ok(()) => println!("   Args with all flag: valid"),
        Err(e) => println!("   Args with all flag: invalid - {}", e),
    }

    // Invalid args (no query, no all flag)
    let args = CountryLookupArgs::default();
    match args.validate() {
        Ok(()) => println!("   Empty args: valid"),
        Err(e) => println!("   Empty args: invalid - {}", e),
    }

    // Example 12: Handle empty results
    println!("\n12. Handling empty results:");
    let args = CountryLookupArgs::new("xyznonexistent");
    let results = lens.search(&args)?;
    println!("   Results count: {}", results.len());
    println!(
        "   JSON output: {}",
        lens.format_results(&results, &CountryOutputFormat::Json)
    );
    println!(
        "   Simple output: {}",
        lens.format_results(&results, &CountryOutputFormat::Simple)
    );

    // Example 13: Common BGP use case - mapping AS country to full name
    println!("\n13. Common BGP use case - AS country lookup:");
    let country_codes = ["US", "DE", "JP", "BR", "AU", "SG"];
    println!("   Resolving AS registration countries:");
    for code in &country_codes {
        let name = lens.lookup_code(code).unwrap_or("Unknown");
        println!("   AS registered in {} -> {}", code, name);
    }

    println!("\n=== Example Complete ===");
    Ok(())
}

fn print_results(results: &[CountryEntry]) {
    if results.is_empty() {
        println!("   No results found");
    } else {
        for country in results {
            println!("   {} - {}", country.code, country.name);
        }
    }
}
