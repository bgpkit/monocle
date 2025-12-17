//! RPKI Validation Example
//!
//! This example demonstrates using RpkiLens for RPKI validation operations
//! including ROA lookups and prefix-ASN validation.
//!
//! # Feature Requirements
//!
//! This example requires the `lens-bgpkit` feature, which includes
//! bgpkit-commons for RPKI data.
//!
//! # Running
//!
//! ```bash
//! cargo run --example rpki_validation --features lens-bgpkit
//! ```
//!
//! Note: This example requires network access to fetch RPKI data on first run.

use monocle::database::MonocleDatabase;
use monocle::lens::rpki::{
    RpkiAspaLookupArgs, RpkiDataSource, RpkiLens, RpkiOutputFormat, RpkiRoaLookupArgs,
    RpkiValidationState,
};

fn main() -> anyhow::Result<()> {
    println!("=== Monocle RPKI Validation Example ===\n");

    // Create an in-memory database for this example
    // In production, use MonocleDatabase::open_in_dir("~/.monocle")
    let db = MonocleDatabase::open_in_memory()?;
    let mut lens = RpkiLens::new(&db);

    // Example 1: Check cache status
    println!("1. Cache status:");
    println!("   Cache is empty: {:?}", lens.is_empty()?);
    println!("   Needs refresh: {:?}", lens.needs_refresh()?);

    // Example 2: Refresh RPKI cache (fetch from Cloudflare)
    println!("\n2. Refreshing RPKI cache from Cloudflare...");
    match lens.refresh() {
        Ok((roa_count, aspa_count)) => {
            println!("   Loaded {} ROAs and {} ASPAs", roa_count, aspa_count);
        }
        Err(e) => {
            println!("   Warning: Could not refresh cache: {}", e);
            println!("   (This may be due to network issues)");
            println!("   Continuing with demonstration of API...");
        }
    }

    // Example 3: Get cache metadata
    println!("\n3. Cache metadata:");
    if let Ok(Some(meta)) = lens.get_metadata() {
        println!("   Updated at: {}", meta.updated_at);
        println!("   ROA count: {}", meta.roa_count);
        println!("   ASPA count: {}", meta.aspa_count);
    } else {
        println!("   No metadata available (cache may be empty)");
    }

    // Example 4: Validate prefix-ASN pairs
    println!("\n4. RPKI validation examples:");

    let test_cases = [
        ("1.1.1.0/24", 13335, "Cloudflare"),
        ("8.8.8.0/24", 15169, "Google DNS"),
        ("1.1.1.0/24", 12345, "Wrong ASN for Cloudflare prefix"),
        ("192.0.2.0/24", 64496, "Documentation prefix"),
    ];

    for (prefix, asn, description) in &test_cases {
        match lens.validate(prefix, *asn) {
            Ok(result) => {
                let state_icon = match result.state {
                    RpkiValidationState::Valid => "✓",
                    RpkiValidationState::Invalid => "✗",
                    RpkiValidationState::NotFound => "?",
                };
                println!(
                    "   {} {} AS{}: {} - {}",
                    state_icon, prefix, asn, result.state, description
                );
                if !result.covering_roas.is_empty() {
                    println!("     Covering ROAs: {}", result.covering_roas.len());
                }
            }
            Err(e) => {
                println!("   ! {} AS{}: Error - {} ({})", prefix, asn, e, description);
            }
        }
    }

    // Example 5: Get covering ROAs for a prefix
    println!("\n5. Get covering ROAs for a prefix:");
    let prefix = "1.1.1.0/24";
    match lens.get_covering_roas(prefix) {
        Ok(roas) => {
            println!("   Covering ROAs for {}:", prefix);
            if roas.is_empty() {
                println!("     No covering ROAs found");
            } else {
                for roa in roas.iter().take(5) {
                    println!(
                        "     {} max:{} AS{} ({})",
                        roa.prefix, roa.max_length, roa.origin_asn, roa.ta
                    );
                }
                if roas.len() > 5 {
                    println!("     ... and {} more", roas.len() - 5);
                }
            }
        }
        Err(e) => println!("   Error: {}", e),
    }

    // Example 6: Look up ROAs by ASN
    println!("\n6. Look up ROAs by ASN:");
    let args = RpkiRoaLookupArgs::new().with_asn(13335);
    match lens.get_roas(&args) {
        Ok(roas) => {
            println!("   ROAs for AS13335 (Cloudflare):");
            if roas.is_empty() {
                println!("     No ROAs found (cache may be empty)");
            } else {
                for roa in roas.iter().take(10) {
                    println!("     {} max:{} ({})", roa.prefix, roa.max_length, roa.ta);
                }
                if roas.len() > 10 {
                    println!("     ... and {} more", roas.len() - 10);
                }
            }
        }
        Err(e) => println!("   Error: {}", e),
    }

    // Example 7: Look up ROAs by prefix
    println!("\n7. Look up ROAs by prefix:");
    let args = RpkiRoaLookupArgs::new().with_prefix("8.8.8.0/24");
    match lens.get_roas(&args) {
        Ok(roas) => {
            println!("   ROAs covering 8.8.8.0/24:");
            if roas.is_empty() {
                println!("     No ROAs found");
            } else {
                for roa in &roas {
                    println!(
                        "     {} max:{} AS{} ({})",
                        roa.prefix, roa.max_length, roa.origin_asn, roa.ta
                    );
                }
            }
        }
        Err(e) => println!("   Error: {}", e),
    }

    // Example 8: ASPA lookups
    println!("\n8. ASPA (AS Provider Authorization) lookups:");
    let args = RpkiAspaLookupArgs::new().with_customer(13335);
    match lens.get_aspas(&args) {
        Ok(aspas) => {
            println!("   ASPAs for AS13335 as customer:");
            if aspas.is_empty() {
                println!("     No ASPAs found");
            } else {
                for aspa in &aspas {
                    println!(
                        "     Customer AS{} authorized providers:",
                        aspa.customer_asn
                    );
                    let providers: Vec<String> =
                        aspa.providers.iter().map(|a| format!("AS{}", a)).collect();
                    println!("       {}", providers.join(", "));
                }
            }
        }
        Err(e) => println!("   Error: {}", e),
    }

    // Example 9: Data sources
    println!("\n9. Available data sources:");
    println!("   - Cloudflare: Current RPKI data (default)");
    println!("   - RIPE NCC: Historical RPKI data");
    println!("   - RPKIviews: Historical RPKI data with multiple collectors");

    let args = RpkiRoaLookupArgs::new()
        .with_asn(13335)
        .with_source(RpkiDataSource::Cloudflare);
    println!("\n   Query with explicit source:");
    println!("   Source: {:?}", args.source);
    println!("   Is historical: {}", args.is_historical());

    // Example 10: Output formatting
    println!("\n10. Output formatting:");
    let args = RpkiRoaLookupArgs::new().with_asn(15169);
    match lens.get_roas(&args) {
        Ok(roas) => {
            if !roas.is_empty() {
                let sample = &roas[..roas.len().min(3)];

                println!("   JSON format:");
                println!("{}", lens.format_roas(sample, &RpkiOutputFormat::Json));

                println!("\n   Pretty format:");
                println!("{}", lens.format_roas(sample, &RpkiOutputFormat::Pretty));
            } else {
                println!("   (No ROAs to format - cache may be empty)");
            }
        }
        Err(e) => println!("   Error: {}", e),
    }

    // Example 11: Validation result formatting
    println!("\n11. Validation result formatting:");
    if let Ok(result) = lens.validate("1.1.1.0/24", 13335) {
        println!("   Table format:");
        println!(
            "{}",
            lens.format_validation(&result, &RpkiOutputFormat::Table)
        );

        println!("   JSON format:");
        println!(
            "{}",
            lens.format_validation(&result, &RpkiOutputFormat::Json)
        );
    }

    // Example 12: Best practices
    println!("\n12. Best practices:");
    println!("   - Check needs_refresh() and refresh cache periodically (24h default TTL)");
    println!("   - Use validate() for single prefix-ASN validation");
    println!("   - Use get_covering_roas() to understand why validation failed");
    println!("   - Cache the RpkiLens instance - it reuses the database connection");
    println!("   - For historical queries, use with_date() and appropriate source");

    println!("\n=== Example Complete ===");
    Ok(())
}
