//! MRT Parsing Example
//!
//! This example demonstrates using ParseLens for parsing MRT (Multi-Threaded
//! Routing Toolkit) files, which contain BGP routing data.
//!
//! # Feature Requirements
//!
//! This example requires the `lens-bgpkit` feature, which includes
//! bgpkit-parser for MRT file parsing.
//!
//! # Running
//!
//! ```bash
//! cargo run --example mrt_parsing --features lens-bgpkit
//! ```
//!
//! Note: This example uses a sample MRT file from the internet.
//! Make sure you have network access.

use monocle::lens::parse::{ParseElemType, ParseFilters, ParseLens, ParseProgress};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    println!("=== Monocle MRT Parsing Example ===\n");

    let lens = ParseLens::new();

    // Example 1: Understanding ParseFilters
    println!("1. ParseFilters options:");
    println!("   Available filter fields:");
    println!("   - origin_asn: Filter by origin AS number");
    println!("   - prefix: Filter by network prefix");
    println!("   - include_super: Include super-prefixes when filtering");
    println!("   - include_sub: Include sub-prefixes when filtering");
    println!("   - peer_ip: Filter by peer IP address(es)");
    println!("   - peer_asn: Filter by peer AS number");
    println!("   - elem_type: Filter by announcement (A) or withdrawal (W)");
    println!("   - start_ts: Filter by start timestamp");
    println!("   - end_ts: Filter by end timestamp");
    println!("   - as_path: Filter by AS path regex");

    // Example 2: Creating filters
    println!("\n2. Creating filters:");

    // Filter for a specific origin ASN
    let filters = ParseFilters {
        origin_asn: Some(13335),
        ..Default::default()
    };
    println!("   Filter by origin ASN 13335: {:?}", filters.origin_asn);

    // Filter for a specific prefix
    let filters = ParseFilters {
        prefix: Some("1.1.1.0/24".to_string()),
        ..Default::default()
    };
    println!("   Filter by prefix: {:?}", filters.prefix);

    // Filter for announcements only
    let filters = ParseFilters {
        elem_type: Some(ParseElemType::A),
        ..Default::default()
    };
    println!("   Filter announcements only: {:?}", filters.elem_type);

    // Combined filters
    let filters = ParseFilters {
        origin_asn: Some(15169),
        elem_type: Some(ParseElemType::A),
        ..Default::default()
    };
    println!(
        "   Combined: origin_asn={:?}, elem_type={:?}",
        filters.origin_asn, filters.elem_type
    );

    // Example 3: Filter validation
    println!("\n3. Filter validation:");
    let filters = ParseFilters::default();
    match filters.validate() {
        Ok(()) => println!("   Default filters: valid"),
        Err(e) => println!("   Default filters: invalid - {}", e),
    }

    // Example 4: Time-based filtering
    println!("\n4. Time-based filtering:");
    let filters = ParseFilters {
        start_ts: Some("2024-01-01T00:00:00Z".to_string()),
        end_ts: Some("2024-01-01T01:00:00Z".to_string()),
        ..Default::default()
    };
    println!("   Start: {:?}", filters.start_ts);
    println!("   End: {:?}", filters.end_ts);

    // Parse timestamps
    match filters.parse_start_end_strings() {
        Ok((start, end)) => {
            println!("   Parsed start timestamp: {}", start);
            println!("   Parsed end timestamp: {}", end);
        }
        Err(e) => println!("   Parse error: {}", e),
    }

    // Example 5: Duration-based filtering
    println!("\n5. Duration-based filtering:");
    let filters = ParseFilters {
        start_ts: Some("2024-01-01T00:00:00Z".to_string()),
        duration: Some("1h".to_string()),
        ..Default::default()
    };
    println!("   Start: {:?}", filters.start_ts);
    println!("   Duration: {:?}", filters.duration);

    // Example 6: AS path regex filtering
    println!("\n6. AS path regex filtering:");
    let filters = ParseFilters {
        as_path: Some("13335$".to_string()), // Paths ending with AS13335
        ..Default::default()
    };
    println!("   AS path regex: {:?}", filters.as_path);
    println!("   This matches paths where AS13335 is the origin");

    // Example 7: Progress callback
    println!("\n7. Progress callback setup:");
    let message_count = Arc::new(AtomicU64::new(0));
    let count_clone = message_count.clone();

    let callback = Arc::new(move |progress: ParseProgress| match progress {
        ParseProgress::Started { file_path } => {
            println!("   Started parsing: {}", file_path);
        }
        ParseProgress::Update {
            messages_processed,
            rate,
            elapsed_secs,
        } => {
            let rate_str = rate
                .map(|r| format!("{:.0} msg/s", r))
                .unwrap_or_else(|| "N/A".to_string());
            println!(
                "   Progress: {} messages, {}, {:.1}s elapsed",
                messages_processed, rate_str, elapsed_secs
            );
        }
        ParseProgress::Completed {
            total_messages,
            duration_secs,
            rate,
        } => {
            count_clone.store(total_messages, Ordering::SeqCst);
            let rate_str = rate
                .map(|r| format!("{:.0} msg/s", r))
                .unwrap_or_else(|| "N/A".to_string());
            println!(
                "   Completed: {} messages in {:.2}s ({})",
                total_messages, duration_secs, rate_str
            );
        }
    });

    println!("   Callback created (would be used with parse_with_progress)");

    // Example 8: Parsing a remote MRT file (demonstration)
    println!("\n8. Parsing demonstration:");
    println!("   To parse a remote MRT file, use:");
    println!("   ```rust");
    println!("   let filters = ParseFilters {{");
    println!("       origin_asn: Some(13335),");
    println!("       ..Default::default()");
    println!("   }};");
    println!("   let url = \"https://data.ris.ripe.net/rrc00/2024.01/updates.20240101.0000.gz\";");
    println!("   let elems = lens.parse_with_progress(&filters, url, Some(callback))?;");
    println!("   for elem in elems {{");
    println!("       println!(\"{{:?}}\", elem);");
    println!("   }}");
    println!("   ```");

    // Example 9: Handler-based parsing (for streaming)
    println!("\n9. Handler-based parsing (streaming):");
    println!("   For memory-efficient processing of large files:");
    println!("   ```rust");
    println!("   let handler = Arc::new(|elem: BgpElem| {{");
    println!("       // Process each element as it's parsed");
    println!("       println!(\"Prefix: {{}}\", elem.prefix);");
    println!("   }});");
    println!("   lens.parse_with_handler(&filters, url, handler)?;");
    println!("   ```");

    // Example 10: Creating parser directly
    println!("\n10. Creating parser directly:");
    println!("   For more control, create a BgpkitParser directly:");
    println!("   ```rust");
    println!("   let parser = lens.create_parser(&filters, \"path/to/file.mrt\")?;");
    println!("   for elem in parser {{");
    println!("       // Process elements");
    println!("   }}");
    println!("   ```");

    // Example 11: Common use cases
    println!("\n11. Common use cases:");

    println!("\n   a) Find all announcements for a prefix:");
    let _filters = ParseFilters {
        prefix: Some("8.8.8.0/24".to_string()),
        elem_type: Some(ParseElemType::A),
        ..Default::default()
    };

    println!("\n   b) Find withdrawals from a specific peer:");
    let _filters = ParseFilters {
        peer_asn: Some(174),
        elem_type: Some(ParseElemType::W),
        ..Default::default()
    };

    println!("\n   c) Find routes with a specific AS in path:");
    let _filters = ParseFilters {
        as_path: Some(".*13335.*".to_string()),
        ..Default::default()
    };

    println!("\n   d) Find routes originated by an AS:");
    let _filters = ParseFilters {
        origin_asn: Some(13335),
        ..Default::default()
    };

    // Example 12: Element types
    println!("\n12. BGP element types:");
    println!("   ParseElemType::A - Announcement (route advertisement)");
    println!("   ParseElemType::W - Withdrawal (route removal)");
    println!(
        "   Display: A = '{}', W = '{}'",
        ParseElemType::A,
        ParseElemType::W
    );

    // Example 13: Best practices
    println!("\n13. Best practices:");
    println!("   - Use filters to reduce memory usage and processing time");
    println!("   - Use parse_with_handler() for very large files");
    println!("   - Use progress callbacks for user feedback on long operations");
    println!("   - Validate filters before parsing");
    println!("   - Consider time-based filtering for RIB dumps");
    println!("   - Use origin_asn filter for targeted analysis");

    // Example 14: Supported file formats
    println!("\n14. Supported file formats:");
    println!("   - Raw MRT files (.mrt)");
    println!("   - Gzip compressed (.gz)");
    println!("   - Bzip2 compressed (.bz2)");
    println!("   - Remote URLs (http://, https://)");
    println!("   - Local file paths");

    println!("\n=== Example Complete ===");
    Ok(())
}
