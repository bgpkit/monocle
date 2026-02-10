//! Search BGP Messages Example
//!
//! Demonstrates searching for BGP messages using the broker.
//!
//! # Running
//!
//! ```bash
//! cargo run --example search_bgp_messages --features lib
//! ```

use monocle::lens::parse::ParseFilters;
use monocle::lens::search::{SearchDumpType, SearchFilters};

fn main() -> anyhow::Result<()> {
    // Search for BGP updates from first hour of 2025
    let filters = SearchFilters {
        parse_filters: ParseFilters {
            start_ts: Some("2025-01-01T00:00:00Z".to_string()),
            end_ts: Some("2025-01-01T01:00:00Z".to_string()),
            ..Default::default()
        },
        collector: Some("rrc00".to_string()),
        project: Some("riperis".to_string()),
        dump_type: SearchDumpType::Updates,
    };

    println!("Searching for BGP messages...");

    let broker = filters.build_broker()?;
    let items = broker.query()?;

    println!("Found {} MRT files", items.len());

    if let Some(first) = items.first() {
        println!("\nProcessing: {}", first.url);

        let parser = filters.to_parser(&first.url)?;

        let mut count = 0;
        for elem in parser.into_iter().take(5) {
            count += 1;
            println!(
                "  {} - {} - {:?}",
                elem.timestamp, elem.prefix, elem.elem_type
            );
        }

        println!("\nShowing first {} elements", count);
    }

    Ok(())
}
