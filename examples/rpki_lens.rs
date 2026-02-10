//! RPKI Validation Example
//!
//! Demonstrates RPKI validation for prefix-ASN pairs.
//!
//! # Running
//!
//! ```bash
//! cargo run --example rpki_validation --features lib
//! ```
//!
//! Note: Requires network access to fetch RPKI data on first run.

use monocle::database::MonocleDatabase;
use monocle::lens::rpki::{RpkiLens, RpkiValidationState};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let db = MonocleDatabase::open_in_memory()?;
    let lens = RpkiLens::new(&db);

    // Refresh cache if needed
    let ttl = Duration::from_secs(24 * 60 * 60);
    if lens.needs_refresh(ttl)? {
        println!("Refreshing RPKI cache...");
        lens.refresh()?;
    }

    // Validate prefix-ASN pairs
    let tests = [
        ("1.1.1.0/24", 13335, "Cloudflare"),
        ("8.8.8.0/24", 15169, "Google"),
        ("1.1.1.0/24", 12345, "Invalid ASN"),
    ];

    println!("\nRPKI Validation Results:");
    for (prefix, asn, desc) in &tests {
        match lens.validate(prefix, *asn) {
            Ok(result) => {
                let icon = match result.state {
                    RpkiValidationState::Valid => "✓",
                    RpkiValidationState::Invalid => "✗",
                    RpkiValidationState::NotFound => "?",
                };
                println!(
                    "  {} {} AS{} - {} ({})",
                    icon, prefix, asn, result.state, desc
                );
            }
            Err(e) => println!("  ! Error: {}", e),
        }
    }

    Ok(())
}
