//! BGP Search Example
//!
//! This example demonstrates using SearchLens for searching BGP messages
//! across multiple MRT files using the BGPKIT broker.
//!
//! # Feature Requirements
//!
//! This example requires the `lens-bgpkit` feature, which includes
//! bgpkit-broker for querying available MRT files and bgpkit-parser
//! for parsing them.
//!
//! # Running
//!
//! ```bash
//! cargo run --example bgp_search --features lens-bgpkit
//! ```
//!
//! Note: This example demonstrates the API but doesn't perform actual searches
//! to avoid long-running network operations.

use monocle::lens::parse::ParseFilters;
use monocle::lens::search::{SearchDumpType, SearchFilters, SearchLens, SearchProgress};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    println!("=== Monocle BGP Search Example ===\n");

    let _lens = SearchLens::new();

    // Example 1: Understanding SearchFilters
    println!("1. SearchFilters structure:");
    println!("   SearchFilters combines:");
    println!("   - parse_filters: All ParseFilters options (origin_asn, prefix, etc.)");
    println!("   - collector: Filter by specific collector (e.g., 'rrc00', 'route-views2')");
    println!("   - project: Filter by project ('riperis' or 'routeviews')");
    println!("   - dump_type: Type of data to search (Updates, Rib, RibUpdates)");

    // Example 2: Creating search filters
    println!("\n2. Creating search filters:");

    // Basic filter for a specific ASN
    let filters = SearchFilters {
        parse_filters: ParseFilters {
            origin_asn: Some(13335),
            ..Default::default()
        },
        ..Default::default()
    };
    println!(
        "   Search for AS13335: origin_asn={:?}",
        filters.parse_filters.origin_asn
    );

    // Filter with collector and project
    let filters = SearchFilters {
        parse_filters: ParseFilters {
            origin_asn: Some(15169),
            ..Default::default()
        },
        collector: Some("rrc00".to_string()),
        project: Some("riperis".to_string()),
        dump_type: SearchDumpType::Updates,
    };
    println!("   Search with collector: {:?}", filters.collector);
    println!("   Search with project: {:?}", filters.project);
    println!("   Dump type: {:?}", filters.dump_type);

    // Example 3: Dump types
    println!("\n3. Dump types:");
    println!("   SearchDumpType::Updates - BGP update messages only");
    println!("   SearchDumpType::Rib - RIB (Routing Information Base) dumps only");
    println!("   SearchDumpType::RibUpdates - Both RIB dumps and updates");

    // Example 4: Filter validation
    println!("\n4. Filter validation:");
    let filters = SearchFilters::default();
    match filters.validate() {
        Ok(()) => println!("   Default filters: valid"),
        Err(e) => println!("   Default filters: invalid - {}", e),
    }

    // Example 5: Building broker query
    println!("\n5. Building broker queries:");
    println!("   SearchFilters can be converted to a broker query:");
    println!("   ```rust");
    println!("   let filters = SearchFilters {{ ... }};");
    println!("   let broker = filters.build_broker()?;");
    println!("   let items = broker.query()?;");
    println!("   println!(\"Found {{}} MRT files\", items.len());");
    println!("   ```");

    // Example 6: Query broker items
    println!("\n6. Query broker for available files:");
    println!("   ```rust");
    println!("   let items = filters.to_broker_items()?;");
    println!("   for item in &items {{");
    println!("       println!(\"{{}} - {{}}\", item.collector_id, item.url);");
    println!("   }}");
    println!("   ```");

    // Example 7: Progress callback types
    println!("\n7. SearchProgress variants:");
    println!("   - QueryingBroker: Starting broker query");
    println!("   - FilesFound {{ count }}: Number of files to process");
    println!("   - FileStarted {{ file_index, total_files, file_url, collector }}");
    println!("   - FileCompleted {{ file_index, total_files, messages_found, success, error }}");
    println!("   - ProgressUpdate {{ files_completed, total_files, total_messages, percent_complete, elapsed_secs, eta_secs }}");
    println!("   - Completed {{ total_files, successful_files, failed_files, total_messages, duration_secs, files_per_sec }}");

    // Example 8: Creating a progress callback
    println!("\n8. Progress callback example:");
    let callback = Arc::new(|progress: SearchProgress| match progress {
        SearchProgress::QueryingBroker => {
            println!("   Querying BGPKIT broker...");
        }
        SearchProgress::FilesFound { count } => {
            println!("   Found {} files to process", count);
        }
        SearchProgress::FileStarted {
            file_index,
            total_files,
            collector,
            ..
        } => {
            println!(
                "   [{}/{}] Processing from {}...",
                file_index + 1,
                total_files,
                collector
            );
        }
        SearchProgress::ProgressUpdate {
            percent_complete,
            total_messages,
            ..
        } => {
            println!(
                "   Progress: {:.1}% ({} messages)",
                percent_complete, total_messages
            );
        }
        SearchProgress::Completed {
            total_files,
            total_messages,
            duration_secs,
            ..
        } => {
            println!(
                "   Done: {} messages from {} files in {:.2}s",
                total_messages, total_files, duration_secs
            );
        }
        _ => {}
    });
    println!("   Callback created");
    let _ = callback; // Suppress unused warning

    // Example 9: Element handler
    println!("\n9. Element handler example:");
    println!("   ```rust");
    println!("   let handler = Arc::new(|elem: BgpElem, collector: String| {{");
    println!("       // Called for each matching BGP element");
    println!("       println!(\"[{{}}] {{}}\", collector, elem.prefix);");
    println!("   }});");
    println!("   ```");

    // Example 10: Search with progress
    println!("\n10. Search with progress (API demonstration):");
    println!("   ```rust");
    println!("   let filters = SearchFilters {{");
    println!("       parse_filters: ParseFilters {{");
    println!("           origin_asn: Some(13335),");
    println!("           start_ts: Some(\"2024-01-01T00:00:00Z\".to_string()),");
    println!("           end_ts: Some(\"2024-01-01T00:15:00Z\".to_string()),");
    println!("           ..Default::default()");
    println!("       }},");
    println!("       collector: Some(\"rrc00\".to_string()),");
    println!("       dump_type: SearchDumpType::Updates,");
    println!("       ..Default::default()");
    println!("   }};");
    println!("");
    println!("   let summary = lens.search_with_progress(");
    println!("       &filters,");
    println!("       Some(progress_callback),");
    println!("       element_handler,");
    println!("   )?;");
    println!("");
    println!("   println!(\"Processed {{}} files, found {{}} messages\",");
    println!("       summary.total_files, summary.total_messages);");
    println!("   ```");

    // Example 11: Search and collect
    println!("\n11. Search and collect (simpler API):");
    println!("   ```rust");
    println!("   let (elements, summary) = lens.search_and_collect(&filters)?;");
    println!("   println!(\"Found {{}} elements\", elements.len());");
    println!("   ```");
    println!("   Note: This collects all results into memory - use search_with_progress");
    println!("   for large result sets.");

    // Example 12: Common search patterns
    println!("\n12. Common search patterns:");

    println!("\n   a) Find hijack candidates (wrong origin for a prefix):");
    let _filters = SearchFilters {
        parse_filters: ParseFilters {
            prefix: Some("8.8.8.0/24".to_string()),
            ..Default::default()
        },
        dump_type: SearchDumpType::Updates,
        ..Default::default()
    };

    println!("\n   b) Monitor a specific AS's announcements:");
    let _filters = SearchFilters {
        parse_filters: ParseFilters {
            origin_asn: Some(13335),
            start_ts: Some("2024-01-01T00:00:00Z".to_string()),
            duration: Some("1h".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };

    println!("\n   c) Analyze RIB snapshots from a specific collector:");
    let _filters = SearchFilters {
        parse_filters: ParseFilters {
            start_ts: Some("2024-01-01T00:00:00Z".to_string()),
            end_ts: Some("2024-01-01T08:00:00Z".to_string()),
            ..Default::default()
        },
        collector: Some("route-views2".to_string()),
        dump_type: SearchDumpType::Rib,
        ..Default::default()
    };

    println!("\n   d) Track AS path changes:");
    let _filters = SearchFilters {
        parse_filters: ParseFilters {
            as_path: Some(".*13335.*".to_string()),
            ..Default::default()
        },
        dump_type: SearchDumpType::Updates,
        ..Default::default()
    };

    // Example 13: SearchSummary
    println!("\n13. SearchSummary fields:");
    println!("   - total_files: Number of files processed");
    println!("   - successful_files: Files parsed successfully");
    println!("   - failed_files: Files that failed to parse");
    println!("   - total_messages: Total BGP messages found");
    println!("   - duration_secs: Total processing time");

    // Example 14: Parallel processing
    println!("\n14. Parallel processing:");
    println!("   SearchLens uses rayon for parallel file processing.");
    println!("   Multiple files are downloaded and parsed concurrently.");
    println!("   The element handler must be Send + Sync for thread safety.");

    // Example 15: Best practices
    println!("\n15. Best practices:");
    println!("   - Use time filters to limit the search scope");
    println!("   - Start with a specific collector before searching all");
    println!("   - Use origin_asn or prefix filters to reduce results");
    println!("   - For large searches, use search_with_progress to stream results");
    println!("   - Check the broker query result count before starting");
    println!("   - Consider dump_type: Updates is usually faster than Rib");

    // Example 16: Error handling
    println!("\n16. Error handling:");
    println!("   - Network errors: Retry with exponential backoff");
    println!("   - Parse errors: Logged via progress callback, doesn't stop search");
    println!("   - Empty results: Check filters and time range");
    println!("   - Broker errors: Check network and filter validity");

    // Example 17: Memory considerations
    println!("\n17. Memory considerations:");
    println!("   - search_and_collect(): Stores all elements in memory");
    println!("   - search_with_progress(): Streams elements, constant memory");
    println!("   - For RIB dumps, expect millions of elements per file");
    println!("   - Use specific filters to reduce memory usage");

    // Example 18: Available collectors
    println!("\n18. Common collectors:");
    println!("   RIPE RIS:");
    println!("     rrc00, rrc01, rrc03, rrc04, rrc05, rrc06, rrc07, ...");
    println!("   Route Views:");
    println!("     route-views2, route-views3, route-views4, ...");
    println!("     route-views.sydney, route-views.sg, ...");

    println!("\n=== Example Complete ===");
    Ok(())
}
