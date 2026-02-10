//! Inspect Example
//!
//! Demonstrates unified AS and prefix information lookup.
//!
//! # Running
//!
//! ```bash
//! cargo run --example inspect --features lib
//! ```
//!
//! Note: Requires network access to fetch data on first run.

use monocle::config::MonocleConfig;
use monocle::database::MonocleDatabase;
use monocle::lens::inspect::{InspectLens, InspectQueryOptions};

fn main() -> anyhow::Result<()> {
    let db = MonocleDatabase::open_in_memory()?;
    let config = MonocleConfig::default();
    let lens = InspectLens::new(&db, &config);

    // Ensure data is available
    println!("Loading data...");
    lens.ensure_data_available()?;

    // Query by ASN
    println!("\nQuerying AS13335:");
    let result = lens.query_as_asn(&["13335".to_string()], &InspectQueryOptions::default())?;
    if let Some(q) = result.queries.first() {
        if let Some(ref info) = q.asinfo {
            if let Some(ref detail) = info.detail {
                println!("  Name: {}", detail.core.name);
            }
        }
    }

    // Query by prefix
    println!("\nQuerying 1.1.1.0/24:");
    let result =
        lens.query_as_prefix(&["1.1.1.0/24".to_string()], &InspectQueryOptions::default())?;
    if let Some(q) = result.queries.first() {
        if let Some(ref pfx) = q.prefix {
            if let Some(ref info) = pfx.pfx2as {
                println!("  Origin AS: {:?}", info.origin_asns);
            }
        }
    }

    Ok(())
}
