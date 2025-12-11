use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bgpkit_parser::encoder::MrtUpdatesEncoder;
use bgpkit_parser::BgpElem;
use clap::Args;
use monocle::database::MsgStore;
use monocle::lens::search::SearchFilters;
use rayon::prelude::*;
use tracing::{info, warn};

use crate::commands::elem_to_string;

/// Arguments for the Search command
#[derive(Args)]
pub struct SearchArgs {
    /// Dry-run, do not download or parse.
    #[clap(long)]
    pub dry_run: bool,

    /// Pretty-print JSON output
    #[clap(long)]
    pub pretty: bool,

    /// SQLite output file path
    #[clap(long)]
    pub sqlite_path: Option<PathBuf>,

    /// MRT output file path
    #[clap(long, short = 'M')]
    pub mrt_path: Option<PathBuf>,

    /// SQLite reset database content if exists
    #[clap(long)]
    pub sqlite_reset: bool,

    /// Filter by AS path regex string
    #[clap(flatten)]
    pub filters: SearchFilters,
}

/// Maximum number of retry attempts (3 attempts total including the first attempt)
const MAX_RETRIES: u32 = 3;

/// Initial retry delay in seconds
const INITIAL_DELAY: u64 = 1;

/// Maximum retry delay in seconds
const MAX_DELAY: u64 = 30;

/// Message types sent through the writer channel
#[derive(Debug)]
enum WriterMessage {
    /// BGP element with its collector ID
    Element(Box<BgpElem>, String),
    /// Signal that a file has been completely processed
    FileComplete,
}

/// Progress update messages for real-time display
#[derive(Debug, Clone)]
enum ProgressUpdate {
    /// A file was completed with message count and success status
    FileComplete { message_count: u32, success: bool },
    /// A new page started processing
    PageStarted { page_num: i64, timestamp: String },
}

/// Structure to track failed processing attempts for retry mechanism
#[derive(Debug, Clone)]
struct FailedItem {
    item: bgpkit_broker::BrokerItem,
    attempt_count: u32,
    last_error: String,
}

impl FailedItem {
    fn new(item: bgpkit_broker::BrokerItem, error: String) -> Self {
        Self {
            item,
            attempt_count: 1,
            last_error: error,
        }
    }

    fn next_delay(&self) -> Duration {
        let base_delay = INITIAL_DELAY * 2_u64.pow(self.attempt_count - 1);
        let delay = std::cmp::min(base_delay, MAX_DELAY);
        Duration::from_secs(delay)
    }

    fn should_retry(&self) -> bool {
        self.attempt_count < MAX_RETRIES
    }

    fn increment_attempt(&mut self, error: String) {
        self.attempt_count += 1;
        self.last_error = error;
    }
}

pub fn run(args: SearchArgs, json: bool) {
    let SearchArgs {
        dry_run,
        pretty,
        sqlite_path,
        mrt_path,
        sqlite_reset,
        filters,
    } = args;

    if let Err(e) = filters.validate() {
        eprintln!("ERROR: {e}");
        return;
    }

    let mut sqlite_path_str = "".to_string();
    let sqlite_db = sqlite_path.and_then(|p| {
        p.to_str().map(|s| {
            sqlite_path_str = s.to_string();
            match MsgStore::new_from_option(&Some(sqlite_path_str.clone()), sqlite_reset) {
                Ok(store) => store,
                Err(e) => {
                    eprintln!("Failed to create SQLite store: {}", e);
                    std::process::exit(1);
                }
            }
        })
    });
    let mrt_path = mrt_path.and_then(|p| p.to_str().map(|s| s.to_string()));
    let show_progress = sqlite_db.is_some() || mrt_path.is_some();

    // Create base broker for pagination
    let base_broker = match filters.build_broker() {
        Ok(broker) => broker,
        Err(e) => {
            eprintln!("Failed to create broker: {}", e);
            std::process::exit(1);
        }
    };

    if dry_run {
        // For dry run, get first page to show what would be processed
        let items = match base_broker.clone().page(1).query_single_page() {
            Ok(items) => items,
            Err(e) => {
                eprintln!("Failed to query broker for dry run: {}", e);
                std::process::exit(1);
            }
        };

        let total_size: i64 = items.iter().map(|x| x.rough_size).sum();
        if json {
            let dry_run_info = serde_json::json!({
                "dry_run": true,
                "first_page_files": items.len(),
                "first_page_bytes": total_size,
                "note": "will process all pages with ~1000 files each"
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&dry_run_info).unwrap_or_default()
            );
        } else {
            println!(
                "First page: {} files, {} bytes (will process all pages with ~1000 files each)",
                items.len(),
                total_size
            );
        }
        return;
    }

    let (sender, receiver): (Sender<WriterMessage>, Receiver<WriterMessage>) = channel();
    // Single progress channel for all updates
    let (progress_sender, progress_receiver): (Sender<ProgressUpdate>, Receiver<ProgressUpdate>) =
        channel();

    // dedicated thread for handling output of results
    let writer_thread = thread::spawn(move || {
        let display_stdout = sqlite_db.is_none() && mrt_path.is_none();
        let mut mrt_writer = match mrt_path {
            Some(p) => match oneio::get_writer(p.as_str()) {
                Ok(writer) => Some((MrtUpdatesEncoder::new(), writer)),
                Err(e) => {
                    eprintln!("Failed to create MRT writer: {}", e);
                    None
                }
            },
            None => None,
        };

        let mut current_file_cache = vec![];
        let mut total_msg_count = 0;

        for msg in receiver {
            match msg {
                WriterMessage::Element(elem, collector) => {
                    total_msg_count += 1;

                    if display_stdout {
                        let output_str =
                            match elem_to_string(&elem, json, pretty, collector.as_str()) {
                                Ok(s) => s,
                                Err(e) => {
                                    eprintln!("Failed to format element: {}", e);
                                    continue;
                                }
                            };
                        println!("{output_str}");
                        continue;
                    }

                    current_file_cache.push((*elem, collector));
                }
                WriterMessage::FileComplete => {
                    // Commit current file's data to SQLite
                    if !current_file_cache.is_empty() {
                        if let Some(db) = &sqlite_db {
                            if let Err(e) = db.insert_elems(&current_file_cache) {
                                eprintln!("Failed to insert elements to database: {}", e);
                            }
                        }
                        if let Some((encoder, _writer)) = &mut mrt_writer {
                            for (elem, _) in &current_file_cache {
                                encoder.process_elem(elem);
                            }
                        }
                        current_file_cache.clear();
                    }
                }
            }
        }

        // Handle any remaining data in cache (in case last file didn't send FileComplete)
        if !current_file_cache.is_empty() {
            if let Some(db) = &sqlite_db {
                if let Err(e) = db.insert_elems(&current_file_cache) {
                    eprintln!("Failed to insert elements to database: {}", e);
                }
            }
            if let Some((encoder, _writer)) = &mut mrt_writer {
                for (elem, _) in &current_file_cache {
                    encoder.process_elem(elem);
                }
            }
        }

        if let Some((encoder, writer)) = &mut mrt_writer {
            let bytes = encoder.export_bytes();
            if let Err(e) = writer.write_all(&bytes) {
                eprintln!("Failed to write MRT data: {}", e);
            }
        }
        drop(mrt_writer);

        if !display_stdout {
            eprintln!("found {total_msg_count} messages, written into file {sqlite_path_str}");
        }
    });

    // Setup spinner for paginated processing
    let pb = if show_progress {
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_message("Processed 0 files, found 0 messages");
        pb.enable_steady_tick(Duration::from_millis(100));
        Some(pb)
    } else {
        None
    };

    // Simplified progress thread with single channel
    let pb_for_updates = pb.clone();
    let progress_thread = thread::spawn(move || {
        let mut files_processed: u64 = 0;
        let mut total_messages: u64 = 0;
        let mut succeeded_files: u64 = 0;
        let mut failed_files: u64 = 0;
        let mut current_page: i64 = 1;
        let mut current_timestamp = String::new();

        for update in progress_receiver {
            match update {
                ProgressUpdate::FileComplete {
                    message_count,
                    success,
                } => {
                    files_processed += 1;
                    total_messages += message_count as u64;
                    if success {
                        succeeded_files += 1;
                    } else {
                        failed_files += 1;
                    }
                }
                ProgressUpdate::PageStarted {
                    page_num,
                    timestamp,
                } => {
                    current_page = page_num;
                    current_timestamp = timestamp;
                }
            }

            // Update progress display
            if let Some(ref pb) = pb_for_updates {
                let page_info = if current_timestamp.is_empty() {
                    format!(
                        " | Page {} (succeeded: {}, failed: {})",
                        current_page, succeeded_files, failed_files
                    )
                } else {
                    format!(
                        " | Page {} (succeeded: {}, failed: {}) {}",
                        current_page, succeeded_files, failed_files, current_timestamp
                    )
                };

                pb.set_message(format!(
                    "Processed {} files, found {} messages{}",
                    files_processed, total_messages, page_info
                ));
            }
        }
    });

    // Create shared structure to collect failed items
    let failed_items = Arc::new(Mutex::new(Vec::<FailedItem>::new()));
    let failed_items_clone = Arc::clone(&failed_items);

    // Paginated processing loop
    let mut page = 1i64;

    loop {
        let items = match base_broker.clone().page(page).query_single_page() {
            Ok(items) => items,
            Err(e) => {
                eprintln!("Failed to fetch page {}: {}", page, e);
                break;
            }
        };

        if items.is_empty() {
            info!("Reached empty page {}, finishing", page);
            break;
        }

        let page_size = items.len();

        // Send page started update to progress thread
        let time_info = if let Some(first_item) = items.first() {
            format!("@ {}", first_item.ts_start.format("%Y-%m-%d %H:%M UTC"))
        } else {
            String::new()
        };

        if progress_sender
            .send(ProgressUpdate::PageStarted {
                page_num: page,
                timestamp: time_info.clone(),
            })
            .is_err()
        {
            // Progress thread may have ended, continue
        }

        if !show_progress {
            info!("Starting page {} ({} files){}", page, page_size, time_info);
            info!("Processing page {} with {} items", page, page_size);
        }

        // Process this page's items using existing parallel logic
        let progress_sender_clone = progress_sender.clone();

        items.into_par_iter().for_each_with(
            (
                sender.clone(),
                progress_sender_clone,
                failed_items_clone.clone(),
            ),
            |(s, progress_sender, failed_items), item| {
                let url = item.url.clone();
                let collector = item.collector_id.clone();

                if !show_progress {
                    info!("start parsing {}", url.as_str());
                }

                let parser = match filters.to_parser(url.as_str()) {
                    Ok(p) => p,
                    Err(e) => {
                        let error_msg = format!("Failed to parse {}: {}", url.as_str(), e);
                        if !show_progress {
                            eprintln!("{}", error_msg);
                        }
                        // Store failed item for retry
                        if let Ok(mut failed) = failed_items.lock() {
                            failed.push(FailedItem::new(item, error_msg));
                        }
                        // Send failure progress update
                        if progress_sender
                            .send(ProgressUpdate::FileComplete {
                                message_count: 0,
                                success: false,
                            })
                            .is_err()
                        {
                            // Progress thread may have ended, ignore
                        }
                        return;
                    }
                };

                let mut elems_count = 0;
                for elem in parser {
                    if s.send(WriterMessage::Element(Box::new(elem), collector.clone()))
                        .is_err()
                    {
                        // Channel closed, break out
                        break;
                    }
                    elems_count += 1;
                }

                // Send file completion signal to trigger per-file commit
                if s.send(WriterMessage::FileComplete).is_err() {
                    // Channel closed, ignore
                }

                // Send success progress update
                if progress_sender
                    .send(ProgressUpdate::FileComplete {
                        message_count: elems_count,
                        success: true,
                    })
                    .is_err()
                {
                    // Progress thread may have ended, ignore
                }

                if !show_progress {
                    info!("finished parsing {}", url.as_str());
                }
            },
        );

        // Page processing complete - no need to update counters as they're updated in real-time

        page += 1;

        // Early exit if partial page (last page)
        if page_size < 1000 {
            info!("Processed final page {} with {} items", page - 1, page_size);
            break;
        }
    }

    if let Some(pb) = pb {
        let final_message = format!("Completed {} pages", page - 1);
        pb.finish_with_message(final_message);
    }

    if !show_progress {
        info!("Completed processing across {} pages", page - 1);
    }

    // Retry phase for failed items
    let failed_count = {
        match failed_items.lock() {
            Ok(failed) => failed.len(),
            Err(e) => {
                warn!("Failed to lock failed_items mutex: {}", e);
                0
            }
        }
    };

    if failed_count > 0 {
        if !show_progress {
            info!("Starting retry phase for {} failed items", failed_count);
        }

        // Process retries sequentially to avoid overwhelming servers
        let mut retry_queue = {
            match failed_items.lock() {
                Ok(failed) => failed.clone(),
                Err(e) => {
                    warn!("Failed to lock failed_items mutex for retry: {}", e);
                    vec![]
                }
            }
        };

        let mut retry_stats = HashMap::new();
        let mut total_retries = 0;
        let mut successful_retries = 0;

        while !retry_queue.is_empty() {
            let mut new_failures = Vec::new();

            for mut failed_item in retry_queue {
                if !failed_item.should_retry() {
                    // Max retries reached
                    *retry_stats.entry("max_retries_reached").or_insert(0) += 1;
                    continue;
                }

                let delay = failed_item.next_delay();
                if !show_progress {
                    info!(
                        "Retrying {} (attempt {}/{}) after {}s delay",
                        failed_item.item.url.as_str(),
                        failed_item.attempt_count + 1,
                        MAX_RETRIES,
                        delay.as_secs()
                    );
                }

                thread::sleep(delay);
                total_retries += 1;

                let parser = match filters.to_parser(failed_item.item.url.as_str()) {
                    Ok(p) => p,
                    Err(e) => {
                        let error_msg = format!(
                            "Retry failed to parse {}: {}",
                            failed_item.item.url.as_str(),
                            e
                        );
                        if !show_progress {
                            warn!("{}", error_msg);
                        }
                        failed_item.increment_attempt(error_msg);
                        new_failures.push(failed_item);
                        continue;
                    }
                };

                let mut elems_count = 0;
                let mut parse_successful = true;

                for elem in parser {
                    if sender
                        .send(WriterMessage::Element(
                            Box::new(elem),
                            failed_item.item.collector_id.clone(),
                        ))
                        .is_err()
                    {
                        // Channel closed, mark as failed
                        parse_successful = false;
                        break;
                    }
                    elems_count += 1;
                }

                // Send file completion signal for retry as well
                if parse_successful && sender.send(WriterMessage::FileComplete).is_err() {
                    parse_successful = false;
                }

                if parse_successful {
                    successful_retries += 1;
                    // Retry successful - progress already tracked by main processing
                    if !show_progress {
                        info!(
                            "Successfully retried {} (found {} messages)",
                            failed_item.item.url.as_str(),
                            elems_count
                        );
                    }
                } else {
                    let error_msg = "Retry failed: channel closed during processing".to_string();
                    failed_item.increment_attempt(error_msg);
                    new_failures.push(failed_item);
                }
            }

            retry_queue = new_failures;
        }

        // Log retry statistics
        let final_failures = retry_queue.len();
        if !show_progress {
            info!(
                "Retry phase completed: {} total retry attempts, {} successful, {} final failures",
                total_retries, successful_retries, final_failures
            );

            if final_failures > 0 {
                warn!(
                    "Warning: {} files could not be processed after {} retry attempts",
                    final_failures, MAX_RETRIES
                );
            }
        }
    }

    // Close channels to signal completion
    drop(sender);
    drop(progress_sender);

    // wait for the output thread to stop
    if let Err(e) = writer_thread.join() {
        eprintln!("Writer thread failed: {:?}", e);
    }
    if let Err(e) = progress_thread.join() {
        eprintln!("Progress thread failed: {:?}", e);
    }
}
