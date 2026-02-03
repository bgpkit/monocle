//! MRT Parsing Example
//!
//! Demonstrates parsing MRT files with filters.
//!
//! # Running
//!
//! ```bash
//! cargo run --example mrt_parsing --features lib
//! ```

use monocle::lens::parse::{ParseFilters, ParseLens};

fn main() -> anyhow::Result<()> {
    let lens = ParseLens::new();

    // Parse with filters
    let filters = ParseFilters {
        origin_asn: vec!["13335".to_string()],
        ..Default::default()
    };

    println!("Parsing MRT file with filters:");
    println!("  Origin ASN: 13335 (Cloudflare)");

    let url = "https://data.ris.ripe.net/rrc00/2024.01/updates.20240101.0000.gz";
    let elems = lens.parse_with_progress(&filters, url, None)?;

    println!("\nFound {} BGP elements", elems.len());
    for elem in elems.iter().take(3) {
        println!(
            "  {} - {} - {:?}",
            elem.timestamp, elem.prefix, elem.elem_type
        );
    }

    Ok(())
}
