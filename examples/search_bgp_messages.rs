//! Example: Search BGP announcement messages
//!
//! This example demonstrates how to use monocle as a library to search for
//! BGP announcement messages from the first hour of 2025 using the rrc00 collector.
//!
//! Run with: cargo run --example search_bgp_messages

use monocle::lens::parse::ParseFilters;
use monocle::lens::search::{SearchDumpType, SearchFilters};

fn main() -> anyhow::Result<()> {
    // Create search filters for the first hour of 2025 using rrc00 collector
    let filters = SearchFilters {
        parse_filters: ParseFilters {
            // Time range: 2025-01-01 00:00:00 to 2025-01-01 01:00:00 UTC
            start_ts: Some("2025-01-01T00:00:00Z".to_string()),
            end_ts: Some("2025-01-01T01:00:00Z".to_string()),
            // Optional: filter by origin ASN (e.g., Cloudflare)
            // origin_asn: Some(13335),
            // Optional: filter by prefix
            // prefix: Some("1.1.1.0/24".to_string()),
            ..Default::default()
        },
        // Use rrc00 collector from RIPE RIS
        collector: Some("rrc00".to_string()),
        // Only RIPE RIS project
        project: Some("riperis".to_string()),
        // Only BGP updates (not RIB dumps)
        dump_type: SearchDumpType::Updates,
    };

    // Validate filters
    filters.validate()?;

    // Build the broker query to find MRT files
    let broker = filters.build_broker()?;

    println!("Searching for BGP messages from rrc00 during the first hour of 2025...");
    println!();

    // Query the broker for available MRT files
    let items = broker.query()?;

    println!("Found {} MRT files to process", items.len());
    println!();

    // Process the first file as a demonstration
    if let Some(first_item) = items.first() {
        println!(
            "Processing first file: {} ({})",
            first_item.url, first_item.collector_id
        );
        println!(
            "Time range: {} - {}",
            first_item.ts_start.format("%Y-%m-%d %H:%M:%S UTC"),
            first_item.ts_end.format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!();

        // Create a parser for the MRT file with filters applied
        let parser = filters.to_parser(&first_item.url)?;

        // Count and display the first few BGP elements
        let mut count = 0;
        let max_display = 5;

        for elem in parser {
            count += 1;

            // Display the first few elements
            if count <= max_display {
                println!("BGP Element #{}:", count);
                println!("  Type: {:?}", elem.elem_type);
                println!("  Timestamp: {}", elem.timestamp);
                println!("  Peer IP: {}", elem.peer_ip);
                println!("  Peer ASN: {}", elem.peer_asn);
                println!("  Prefix: {}", elem.prefix);
                if let Some(path) = &elem.as_path {
                    println!("  AS Path: {:?}", path.to_u32_vec_opt(true));
                }
                if let Some(origin) = &elem.origin_asns {
                    let asns: Vec<u32> = origin.iter().map(|a| a.to_u32()).collect();
                    println!("  Origin ASN(s): {:?}", asns);
                }
                println!();
            }

            // Stop after processing some messages for the demo
            if count >= 100 {
                break;
            }
        }

        println!("Processed {} BGP elements (limited to 100 for demo)", count);
    } else {
        println!("No MRT files found for the specified time range");
    }

    Ok(())
}
