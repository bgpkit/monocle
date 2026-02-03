//! Country Lookup Example
//!
//! Demonstrates looking up country codes and names.
//!
//! # Running
//!
//! ```bash
//! cargo run --example country_lookup --features lib
//! ```

use monocle::lens::country::{CountryLens, CountryLookupArgs};

fn main() -> anyhow::Result<()> {
    let lens = CountryLens::new();

    // Look up by country code
    println!("Looking up 'US':");
    let args = CountryLookupArgs::new("US");
    let results = lens.search(&args)?;
    for country in &results {
        println!("  {} - {}", country.code, country.name);
    }

    // Search by partial name
    println!("\nSearching for 'united':");
    let args = CountryLookupArgs::new("united");
    let results = lens.search(&args)?;
    for country in &results {
        println!("  {} - {}", country.code, country.name);
    }

    Ok(())
}
