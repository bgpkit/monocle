//! Progress Callbacks Example
//!
//! This example demonstrates how to use progress callbacks in monocle for
//! long-running operations like parsing and searching. Progress callbacks
//! are essential for building responsive GUI applications or showing
//! progress bars in CLI tools.
//!
//! # Feature Requirements
//!
//! This example requires the `lens-full` feature, which includes all
//! lens functionality.
//!
//! # Running
//!
//! ```bash
//! cargo run --example progress_callbacks --features lens-full
//! ```

use monocle::lens::parse::{ParseFilters, ParseLens, ParseProgress};
use monocle::lens::search::{SearchDumpType, SearchFilters, SearchLens, SearchProgress};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    println!("=== Monocle Progress Callbacks Example ===\n");

    // ==========================================================================
    // Part 1: ParseProgress Callbacks
    // ==========================================================================

    println!("1. ParseProgress callback types:");
    println!("   ParseProgress::Started {{ file_path }}");
    println!("     - Emitted when parsing begins");
    println!("     - Contains the file path being parsed");
    println!();
    println!("   ParseProgress::Update {{ messages_processed, rate, elapsed_secs }}");
    println!("     - Emitted periodically during parsing (every 10,000 messages)");
    println!("     - messages_processed: Total count so far");
    println!("     - rate: Optional processing rate (messages/sec)");
    println!("     - elapsed_secs: Time since start");
    println!();
    println!("   ParseProgress::Completed {{ total_messages, duration_secs, rate }}");
    println!("     - Emitted when parsing finishes");
    println!("     - Contains final statistics");

    // ==========================================================================
    // Part 2: Creating a Parse Progress Callback
    // ==========================================================================

    println!("\n2. Creating a parse progress callback:");

    // Shared state for the callback
    let parse_started = Arc::new(AtomicBool::new(false));
    let parse_messages = Arc::new(AtomicU64::new(0));
    let parse_start_time = Arc::new(std::sync::Mutex::new(None::<Instant>));

    // Clone for use in callback
    let started_clone = parse_started.clone();
    let messages_clone = parse_messages.clone();
    let start_time_clone = parse_start_time.clone();

    let parse_callback = Arc::new(move |progress: ParseProgress| match progress {
        ParseProgress::Started { file_path } => {
            started_clone.store(true, Ordering::SeqCst);
            *start_time_clone.lock().unwrap() = Some(Instant::now());
            println!("   [PARSE] Started: {}", file_path);
        }
        ParseProgress::Update {
            messages_processed,
            rate,
            elapsed_secs,
        } => {
            messages_clone.store(messages_processed, Ordering::SeqCst);
            let rate_str = rate
                .map(|r| format!("{:.0} msg/s", r))
                .unwrap_or_else(|| "N/A".to_string());
            println!(
                "   [PARSE] Progress: {} messages, {}, {:.1}s",
                messages_processed, rate_str, elapsed_secs
            );
        }
        ParseProgress::Completed {
            total_messages,
            duration_secs,
            rate,
        } => {
            messages_clone.store(total_messages, Ordering::SeqCst);
            let rate_str = rate
                .map(|r| format!("{:.0} msg/s", r))
                .unwrap_or_else(|| "N/A".to_string());
            println!(
                "   [PARSE] Completed: {} messages in {:.2}s ({})",
                total_messages, duration_secs, rate_str
            );
        }
    });

    println!("   Parse callback created with shared state tracking");

    // ==========================================================================
    // Part 3: Using Parse Callback (demonstration)
    // ==========================================================================

    println!("\n3. Using parse callback (code example):");
    println!("   ```rust");
    println!("   let lens = ParseLens::new();");
    println!("   let filters = ParseFilters {{");
    println!("       origin_asn: Some(13335),");
    println!("       ..Default::default()");
    println!("   }};");
    println!();
    println!("   let elems = lens.parse_with_progress(");
    println!("       &filters,");
    println!("       \"https://example.com/updates.mrt.gz\",");
    println!("       Some(parse_callback),");
    println!("   )?;");
    println!("   ```");

    // ==========================================================================
    // Part 4: SearchProgress Callbacks
    // ==========================================================================

    println!("\n4. SearchProgress callback types:");
    println!("   SearchProgress::QueryingBroker");
    println!("     - Emitted when starting broker query");
    println!();
    println!("   SearchProgress::FilesFound {{ count }}");
    println!("     - Emitted after broker query completes");
    println!("     - count: Number of MRT files to process");
    println!();
    println!("   SearchProgress::FileStarted {{ file_index, total_files, file_url, collector }}");
    println!("     - Emitted when starting to process a file");
    println!();
    println!("   SearchProgress::FileCompleted {{ file_index, total_files, messages_found, success, error }}");
    println!("     - Emitted when a file finishes processing");
    println!();
    println!("   SearchProgress::ProgressUpdate {{ files_completed, total_files, total_messages, percent_complete, elapsed_secs, eta_secs }}");
    println!("     - Periodic overall progress update");
    println!();
    println!("   SearchProgress::Completed {{ total_files, successful_files, failed_files, total_messages, duration_secs, files_per_sec }}");
    println!("     - Final summary when search completes");

    // ==========================================================================
    // Part 5: Creating a Search Progress Callback
    // ==========================================================================

    println!("\n5. Creating a search progress callback:");

    // Shared state for search callback
    let search_files_total = Arc::new(AtomicU64::new(0));
    let search_files_done = Arc::new(AtomicU64::new(0));
    let search_messages = Arc::new(AtomicU64::new(0));

    let files_total_clone = search_files_total.clone();
    let files_done_clone = search_files_done.clone();
    let messages_clone2 = search_messages.clone();

    let search_callback = Arc::new(move |progress: SearchProgress| match progress {
        SearchProgress::QueryingBroker => {
            println!("   [SEARCH] Querying BGPKIT broker...");
        }
        SearchProgress::FilesFound { count } => {
            files_total_clone.store(count as u64, Ordering::SeqCst);
            println!("   [SEARCH] Found {} files to process", count);
        }
        SearchProgress::FileStarted {
            file_index,
            total_files,
            collector,
            ..
        } => {
            println!(
                "   [SEARCH] [{}/{}] Starting {} ...",
                file_index + 1,
                total_files,
                collector
            );
        }
        SearchProgress::FileCompleted {
            file_index,
            total_files,
            messages_found,
            success,
            error,
        } => {
            files_done_clone.fetch_add(1, Ordering::SeqCst);
            if success {
                println!(
                    "   [SEARCH] [{}/{}] Completed: {} messages",
                    file_index + 1,
                    total_files,
                    messages_found
                );
            } else {
                println!(
                    "   [SEARCH] [{}/{}] Failed: {}",
                    file_index + 1,
                    total_files,
                    error.unwrap_or_else(|| "Unknown error".to_string())
                );
            }
        }
        SearchProgress::ProgressUpdate {
            files_completed,
            total_files,
            total_messages,
            percent_complete,
            elapsed_secs,
            eta_secs,
        } => {
            messages_clone2.store(total_messages, Ordering::SeqCst);
            let eta_str = eta_secs
                .map(|e| format!("ETA: {:.0}s", e))
                .unwrap_or_else(|| "ETA: N/A".to_string());
            println!(
                "   [SEARCH] Progress: {}/{} files ({:.1}%), {} messages, {:.1}s elapsed, {}",
                files_completed,
                total_files,
                percent_complete,
                total_messages,
                elapsed_secs,
                eta_str
            );
        }
        SearchProgress::Completed {
            total_files,
            successful_files,
            failed_files,
            total_messages,
            duration_secs,
            files_per_sec,
        } => {
            let rate_str = files_per_sec
                .map(|r| format!("{:.2} files/s", r))
                .unwrap_or_else(|| "N/A".to_string());
            println!(
                "   [SEARCH] Completed: {}/{} files successful, {} failed",
                successful_files, total_files, failed_files
            );
            println!(
                "   [SEARCH] Total: {} messages in {:.2}s ({})",
                total_messages, duration_secs, rate_str
            );
        }
    });

    println!("   Search callback created with shared state tracking");

    // ==========================================================================
    // Part 6: Element Handler
    // ==========================================================================

    println!("\n6. Element handler for streaming results:");
    println!("   The element handler is called for each matching BGP element.");
    println!("   It must be Send + Sync for thread safety (parallel processing).");

    let element_count = Arc::new(AtomicU64::new(0));
    let count_clone = element_count.clone();

    // Note: BgpElem and collector would be the actual parameters
    let _element_handler = Arc::new(move |_elem: (), collector: String| {
        let count = count_clone.fetch_add(1, Ordering::SeqCst);
        if count % 1000 == 0 {
            println!("   [ELEM] Received element {} from {}", count, collector);
        }
    });

    println!("   Element handler created");

    // ==========================================================================
    // Part 7: GUI Integration Pattern
    // ==========================================================================

    println!("\n7. GUI integration pattern:");
    println!("   For GUI applications, callbacks can send messages to the UI thread:");
    println!();
    println!("   ```rust");
    println!("   use std::sync::mpsc;");
    println!();
    println!("   // Create a channel for UI updates");
    println!("   let (tx, rx) = mpsc::channel();");
    println!();
    println!("   // Clone sender for callback");
    println!("   let tx_clone = tx.clone();");
    println!("   let callback = Arc::new(move |progress: SearchProgress| {{");
    println!("       // Send progress to UI thread (non-blocking)");
    println!("       let _ = tx_clone.send(UiMessage::SearchProgress(progress));");
    println!("   }});");
    println!();
    println!("   // In UI thread, receive and handle messages");
    println!("   while let Ok(msg) = rx.try_recv() {{");
    println!("       match msg {{");
    println!("           UiMessage::SearchProgress(p) => update_progress_bar(p),");
    println!("           // ...other messages");
    println!("       }}");
    println!("   }}");
    println!("   ```");

    // ==========================================================================
    // Part 8: Async Integration Pattern
    // ==========================================================================

    println!("\n8. Async integration pattern:");
    println!("   For async applications, use tokio channels:");
    println!();
    println!("   ```rust");
    println!("   use tokio::sync::mpsc;");
    println!();
    println!("   let (tx, mut rx) = mpsc::unbounded_channel();");
    println!();
    println!("   let callback = Arc::new(move |progress: SearchProgress| {{");
    println!("       let _ = tx.send(progress);");
    println!("   }});");
    println!();
    println!("   // Spawn search on blocking thread");
    println!("   let handle = tokio::task::spawn_blocking(move || {{");
    println!("       lens.search_with_progress(&filters, Some(callback), handler)");
    println!("   }});");
    println!();
    println!("   // Process progress in async context");
    println!("   while let Some(progress) = rx.recv().await {{");
    println!("       // Update UI, log, etc.");
    println!("   }}");
    println!("   ```");

    // ==========================================================================
    // Part 9: Progress Bar Integration
    // ==========================================================================

    println!("\n9. Progress bar integration (indicatif):");
    println!("   ```rust");
    println!("   use indicatif::{{ProgressBar, ProgressStyle}};");
    println!();
    println!("   let pb = ProgressBar::new(100);");
    println!("   pb.set_style(ProgressStyle::default_bar()");
    println!("       .template(\"[{{bar:40}}] {{pos}}/{{len}} {{msg}}\"));");
    println!();
    println!("   let pb_clone = pb.clone();");
    println!("   let callback = Arc::new(move |progress: SearchProgress| {{");
    println!("       match progress {{");
    println!("           SearchProgress::FilesFound {{ count }} => {{");
    println!("               pb_clone.set_length(count as u64);");
    println!("           }}");
    println!("           SearchProgress::FileCompleted {{ file_index, .. }} => {{");
    println!("               pb_clone.set_position(file_index as u64 + 1);");
    println!("           }}");
    println!("           SearchProgress::Completed {{ .. }} => {{");
    println!("               pb_clone.finish_with_message(\"done\");");
    println!("           }}");
    println!("           _ => {{}}");
    println!("       }}");
    println!("   }});");
    println!("   ```");

    // ==========================================================================
    // Part 10: Serialization for IPC
    // ==========================================================================

    println!("\n10. Serialization for IPC:");
    println!("    Both ParseProgress and SearchProgress implement Serialize/Deserialize.");
    println!("    This allows sending progress over WebSocket, IPC, etc.");
    println!();

    // Demonstrate serialization
    let progress = SearchProgress::FilesFound { count: 42 };
    let json = serde_json::to_string(&progress)?;
    println!("    Example serialized progress:");
    println!("    {}", json);

    let progress = SearchProgress::ProgressUpdate {
        files_completed: 10,
        total_files: 42,
        total_messages: 50000,
        percent_complete: 23.8,
        elapsed_secs: 30.5,
        eta_secs: Some(97.5),
    };
    let json = serde_json::to_string_pretty(&progress)?;
    println!("\n    Complex progress:");
    println!("{}", json);

    // ==========================================================================
    // Part 11: Best Practices
    // ==========================================================================

    println!("\n11. Best practices:");
    println!("    - Keep callbacks lightweight (don't block)");
    println!("    - Use channels for cross-thread communication");
    println!("    - Handle all progress variants (use _ => {{}} for unhandled)");
    println!("    - Track state with Arc<Atomic*> for thread safety");
    println!("    - Consider rate-limiting UI updates for performance");
    println!("    - Always handle the Completed variant for cleanup");
    println!("    - Check 'success' field in FileCompleted for error handling");

    // ==========================================================================
    // Part 12: Error Handling in Callbacks
    // ==========================================================================

    println!("\n12. Error handling in callbacks:");
    println!("    Callbacks should not panic - this could crash parallel workers.");
    println!("    Use Result types and logging for error handling:");
    println!();
    println!("    ```rust");
    println!("    let callback = Arc::new(move |progress: SearchProgress| {{");
    println!("        if let Err(e) = handle_progress(&progress) {{");
    println!("            tracing::warn!(\"Progress handler error: {{}}\", e);");
    println!("        }}");
    println!("    }});");
    println!("    ```");

    println!("\n=== Example Complete ===");
    Ok(())
}
