//! Prefix-to-ASN Example
//!
//! Demonstrates prefix-to-ASN mapping lookups with RPKI validation.
//!
//! # Running
//!
//! ```bash
//! cargo run --example pfx2as_lens --features lib
//! ```

use monocle::database::MonocleDatabase;
use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asSearchArgs};
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let db = MonocleDatabase::open_in_memory()?;
    let lens = Pfx2asLens::new(&db);

    // Refresh cache if needed
    let ttl = Duration::from_secs(24 * 60 * 60);
    if lens.needs_refresh(ttl)? {
        println!("Refreshing pfx2as cache...");
        lens.refresh(None)?;
    }

    // Search by prefix
    println!("\nSearching for 1.1.1.0/24:");
    let args = Pfx2asSearchArgs::new("1.1.1.0/24").with_show_name(true);
    let results = lens.search(&args)?;

    for r in &results {
        println!("  {} -> AS{} (RPKI: {})", r.prefix, r.origin_asn, r.rpki);
    }

    // Search by ASN
    println!("\nSearching for AS13335 prefixes:");
    let args = Pfx2asSearchArgs::new("13335").with_limit(5);
    let results = lens.search(&args)?;

    for r in &results {
        println!("  {} -> AS{}", r.prefix, r.origin_asn);
    }

    Ok(())
}
