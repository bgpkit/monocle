#![allow(clippy::type_complexity)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bgpkit_parser::encoder::MrtUpdatesEncoder;
use bgpkit_parser::BgpElem;
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use ipnet::IpNet;
use itertools::Itertools;
use json_to_table::json_to_table;
use monocle::*;
use radar_rs::RadarClient;
use rayon::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};
use tabled::settings::object::Columns;
use tabled::settings::width::Width;
use tabled::settings::Style;
use tabled::{Table, Tabled};
use tracing::{info, warn, Level};

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

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    /// configuration file path, by default $HOME/.monocle.toml is used
    #[clap(short, long)]
    config: Option<String>,

    /// Print debug information
    #[clap(long, global = true)]
    debug: bool,

    /// Output as JSON objects
    #[clap(long, global = true)]
    json: bool,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse individual MRT files given a file path, local or remote.
    Parse {
        /// File path to an MRT file, local or remote.
        #[clap(name = "FILE")]
        file_path: PathBuf,

        /// Pretty-print JSON output
        #[clap(long)]
        pretty: bool,

        /// MRT output file path
        #[clap(long, short = 'M')]
        mrt_path: Option<PathBuf>,

        /// Filter by AS path regex string
        #[clap(flatten)]
        filters: ParseFilters,
    },
    /// Query BGPKIT Broker for the meta data of available MRT files.
    Broker {
        /// starting timestamp (RFC3339 or unix epoch)
        #[clap(long, short = 't')]
        start_ts: String,

        /// ending timestamp (RFC3339 or unix epoch)
        #[clap(long, short = 'T')]
        end_ts: String,

        /// BGP collector name: e.g. rrc00, route-views2
        #[clap(long, short = 'c')]
        collector: Option<String>,

        /// BGP collection project name, e.g. routeviews, or riperis
        #[clap(long, short = 'P')]
        project: Option<String>,

        /// Data type, e.g., updates or rib
        #[clap(long)]
        data_type: Option<String>,

        /// Page number to fetch (1-based). If set, only this page will be fetched.
        #[clap(long)]
        page: Option<i64>,

        /// Page size for broker queries (default 1000)
        #[clap(long)]
        page_size: Option<i64>,
    },

    /// Search BGP messages from all available public MRT files.
    Search {
        /// Dry-run, do not download or parse.
        #[clap(long)]
        dry_run: bool,

        /// Pretty-print JSON output
        #[clap(long)]
        pretty: bool,

        /// SQLite output file path
        #[clap(long)]
        sqlite_path: Option<PathBuf>,

        /// MRT output file path
        #[clap(long, short = 'M')]
        mrt_path: Option<PathBuf>,

        /// SQLite reset database content if exists
        #[clap(long)]
        sqlite_reset: bool,

        /// Filter by AS path regex string
        #[clap(flatten)]
        filters: SearchFilters,
    },
    /// ASN and organization lookup utility.
    Whois {
        /// Search query, an ASN (e.g. "400644") or a name (e.g. "bgpkit")
        query: Vec<String>,

        /// Search AS and Org name only
        #[clap(short, long)]
        name_only: bool,

        /// Search by ASN only
        #[clap(short, long)]
        asn_only: bool,

        /// Search by country only
        #[clap(short = 'C', long)]
        country_only: bool,

        /// Refresh the local as2org database
        #[clap(short, long)]
        update: bool,

        /// Output to pretty table, default markdown table
        #[clap(short, long)]
        pretty: bool,

        /// Display a full table (with ord_id, org_size)
        #[clap(short = 'F', long)]
        full_table: bool,

        /// Export to pipe-separated values
        #[clap(short = 'P', long)]
        psv: bool,

        /// Show full country names instead of 2-letter code
        #[clap(short, long)]
        full_country: bool,
    },

    /// Country name and code lookup utilities
    Country {
        /// Search query, e.g. "US" or "United States"
        queries: Vec<String>,
    },

    /// Time conversion utilities
    Time {
        /// Time stamp or time string to convert
        #[clap()]
        time: Vec<String>,

        /// Simple output, only print the converted time
        #[clap(short, long)]
        simple: bool,
    },

    /// RPKI utilities
    Rpki {
        #[clap(subcommand)]
        commands: RpkiCommands,
    },

    /// IP information lookup
    Ip {
        /// IP address to look up (optional)
        #[clap()]
        ip: Option<IpAddr>,

        /// Print IP address only (e.g., for getting the public IP address quickly)
        #[clap(long)]
        simple: bool,
    },

    /// Cloudflare Radar API lookup (set CF_API_TOKEN to enable)
    Radar {
        #[clap(subcommand)]
        commands: RadarCommands,
    },

    /// Bulk prefix-to-AS mapping lookup with the pre-generated data file.
    Pfx2as {
        /// Prefix-to-AS mapping data file location
        #[clap(
            long,
            default_value = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2"
        )]
        data_file_path: String,

        /// IP prefixes or prefix files (one prefix per line)
        #[clap(required = true)]
        input: Vec<String>,

        /// Only matching exact prefixes. By default, it does longest-prefix matching.
        #[clap(short, long)]
        exact_match: bool,
    },
}

#[derive(Subcommand)]
enum RpkiCommands {
    /// validate a prefix-asn pair with a RPKI validator (Cloudflare)
    Check {
        #[clap(short, long)]
        asn: u32,

        #[clap(short, long)]
        prefix: String,
    },

    /// list ROAs by ASN or prefix (Cloudflare real-time)
    List {
        /// prefix or ASN
        #[clap()]
        resource: String,
    },

    /// summarize RPKI status for a list of given ASNs (Cloudflare)
    Summary {
        #[clap()]
        asns: Vec<u32>,
    },

    /// list ROAs from RPKI data (current or historical via bgpkit-commons)
    Roas {
        /// Filter by origin ASN
        #[clap(long)]
        origin: Option<u32>,

        /// Filter by prefix
        #[clap(long)]
        prefix: Option<String>,

        /// Load historical data for this date (YYYY-MM-DD)
        #[clap(long)]
        date: Option<String>,

        /// Historical data source: ripe, rpkiviews (default: ripe)
        #[clap(long, default_value = "ripe")]
        source: String,

        /// RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost)
        #[clap(long, default_value = "soborost")]
        collector: String,
    },

    /// list ASPAs from RPKI data (current or historical via bgpkit-commons)
    Aspas {
        /// Filter by customer ASN
        #[clap(long)]
        customer: Option<u32>,

        /// Filter by provider ASN
        #[clap(long)]
        provider: Option<u32>,

        /// Load historical data for this date (YYYY-MM-DD)
        #[clap(long)]
        date: Option<String>,

        /// Historical data source: ripe, rpkiviews (default: ripe)
        #[clap(long, default_value = "ripe")]
        source: String,

        /// RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost)
        #[clap(long, default_value = "soborost")]
        collector: String,
    },
}

#[derive(Subcommand)]
enum RadarCommands {
    /// get routing stats
    Stats {
        /// a two-letter country code or asn number (e.g., US or 13335)
        #[clap(name = "QUERY")]
        query: Option<String>,
    },

    /// look up prefix-to-origin mapping on the most recent global routing table snapshot
    Pfx2as {
        /// an IP prefix or an AS number (e.g., 1.1.1.0/24 or 13335)
        #[clap(name = "QUERY")]
        query: String,

        /// filter by RPKI validation status, valid, invalid, or unknown
        #[clap(short, long)]
        rpki_status: Option<String>,
    },
}

fn elem_to_string(
    elem: &BgpElem,
    json: bool,
    pretty: bool,
    collector: &str,
) -> Result<String, anyhow::Error> {
    if json {
        let mut val = json!(elem);
        val.as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected JSON object"))?
            .insert("collector".to_string(), collector.into());
        if pretty {
            Ok(serde_json::to_string_pretty(&val)?)
        } else {
            Ok(val.to_string())
        }
    } else {
        Ok(format!("{}|{}", elem, collector))
    }
}

fn main() {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    let config = match MonocleConfig::new(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    if cli.debug {
        tracing_subscriber::fmt()
            // filter spans/events with level TRACE or higher.
            .with_max_level(Level::INFO)
            .init();
    }

    let json = cli.json;

    // You can check for the existence of subcommands, and if found, use their
    // matches just as you would the top level cmd
    match cli.command {
        Commands::Parse {
            file_path,
            pretty,
            mrt_path,
            filters,
        } => {
            if let Err(e) = filters.validate() {
                eprintln!("ERROR: {e}");
                return;
            }

            let file_path = match file_path.to_str() {
                Some(path) => path,
                None => {
                    eprintln!("Invalid file path");
                    std::process::exit(1);
                }
            };
            let parser = match filters.to_parser(file_path) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Failed to create parser for {}: {}", file_path, e);
                    std::process::exit(1);
                }
            };

            let mut stdout = std::io::stdout();

            match mrt_path {
                None => {
                    for elem in parser {
                        // output to stdout
                        let output_str = match elem_to_string(&elem, json, pretty, "") {
                            Ok(s) => s,
                            Err(e) => {
                                eprintln!("Failed to format element: {}", e);
                                continue;
                            }
                        };
                        if let Err(e) = writeln!(stdout, "{}", &output_str) {
                            if e.kind() != std::io::ErrorKind::BrokenPipe {
                                eprintln!("ERROR: {e}");
                            }
                            std::process::exit(1);
                        }
                    }
                }
                Some(p) => {
                    let path = match p.to_str() {
                        Some(path) => path.to_string(),
                        None => {
                            eprintln!("Invalid MRT path");
                            std::process::exit(1);
                        }
                    };
                    println!("processing. filtered messages output to {}...", &path);
                    let mut encoder = MrtUpdatesEncoder::new();
                    let mut writer = match oneio::get_writer(&path) {
                        Ok(w) => w,
                        Err(e) => {
                            eprintln!("ERROR: {e}");
                            std::process::exit(1);
                        }
                    };
                    let mut total_count = 0;
                    for elem in parser {
                        total_count += 1;
                        encoder.process_elem(&elem);
                    }
                    if let Err(e) = writer.write_all(&encoder.export_bytes()) {
                        eprintln!("Failed to write MRT data: {}", e);
                    }
                    drop(writer);
                    println!("done. total of {} message wrote", total_count);
                }
            }
        }
        Commands::Search {
            dry_run,
            pretty,
            mrt_path,
            sqlite_path,
            sqlite_reset,
            filters,
        } => {
            if let Err(e) = filters.validate() {
                eprintln!("ERROR: {e}");
                return;
            }

            let mut sqlite_path_str = "".to_string();
            let sqlite_db = sqlite_path.and_then(|p| {
                p.to_str().map(|s| {
                    sqlite_path_str = s.to_string();
                    match MsgStore::new(&Some(sqlite_path_str.clone()), sqlite_reset) {
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
                println!(
                    "First page: {} files, {} bytes (will process all pages with ~1000 files each)",
                    items.len(),
                    total_size
                );
                return;
            }

            let (sender, receiver): (Sender<WriterMessage>, Receiver<WriterMessage>) = channel();
            // Single progress channel for all updates
            let (progress_sender, progress_receiver): (
                Sender<ProgressUpdate>,
                Receiver<ProgressUpdate>,
            ) = channel();

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
                    println!(
                        "found {total_msg_count} messages, written into file {sqlite_path_str}"
                    );
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
                            let error_msg =
                                "Retry failed: channel closed during processing".to_string();
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

        Commands::Broker {
            start_ts,
            end_ts,
            collector,
            project,
            data_type,
            page,
            page_size,
        } => {
            // parse time strings similar to Search subcommand
            let ts_start = match string_to_time(&start_ts) {
                Ok(t) => t.timestamp(),
                Err(_) => {
                    eprintln!("start-ts is not a valid time string: {}", start_ts);
                    std::process::exit(1);
                }
            };
            let ts_end = match string_to_time(&end_ts) {
                Ok(t) => t.timestamp(),
                Err(_) => {
                    eprintln!("end-ts is not a valid time string: {}", end_ts);
                    std::process::exit(1);
                }
            };

            let mut broker = bgpkit_broker::BgpkitBroker::new()
                .ts_start(ts_start)
                .ts_end(ts_end);

            if let Some(c) = collector {
                broker = broker.collector_id(c.as_str());
            }
            if let Some(p) = project {
                broker = broker.project(p.as_str());
            }
            if let Some(dt) = data_type {
                broker = broker.data_type(dt.as_str());
            }

            let page_size = page_size.unwrap_or(1000);
            broker = broker.page_size(page_size);

            let res = if let Some(p) = page {
                broker.page(p).query_single_page()
            } else {
                // Use query() and limit to at most 10 pages worth of items
                match broker.query() {
                    Ok(mut v) => {
                        let max_items = (page_size * 10) as usize;
                        if v.len() > max_items {
                            v.truncate(max_items);
                        }
                        Ok(v)
                    }
                    Err(e) => Err(e),
                }
            };

            match res {
                Ok(items) => {
                    if items.is_empty() {
                        println!("No MRT files found");
                        return;
                    }

                    if json {
                        match serde_json::to_string_pretty(&items) {
                            Ok(json_str) => println!("{}", json_str),
                            Err(e) => eprintln!("error serializing: {}", e),
                        }
                    } else {
                        #[derive(Tabled)]
                        struct BrokerItemDisplay {
                            #[tabled(rename = "Collector")]
                            collector_id: String,
                            #[tabled(rename = "Type")]
                            data_type: String,
                            #[tabled(rename = "Start Time (UTC)")]
                            ts_start: String,
                            #[tabled(rename = "URL")]
                            url: String,
                            #[tabled(rename = "Size (Bytes)")]
                            rough_size: i64,
                        }

                        let display_items: Vec<BrokerItemDisplay> = items
                            .into_iter()
                            .map(|item| BrokerItemDisplay {
                                collector_id: item.collector_id,
                                data_type: item.data_type,
                                ts_start: item.ts_start.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                                url: item.url,
                                rough_size: item.rough_size,
                            })
                            .collect();

                        println!("{}", Table::new(display_items).with(Style::markdown()));
                    }
                }
                Err(e) => {
                    eprintln!("failed to query: {}", e);
                }
            }
        }

        Commands::Whois {
            query,
            name_only,
            asn_only,
            update,
            pretty,
            full_table,
            full_country,
            country_only,
            psv,
        } => {
            let data_dir = config.data_dir.as_str();
            let as2org = match As2org::new(&Some(format!("{data_dir}/monocle-data.sqlite3"))) {
                Ok(as2org) => as2org,
                Err(e) => {
                    eprintln!("Failed to create AS2org database: {}", e);
                    std::process::exit(1);
                }
            };

            if update {
                // if the update flag is set, clear existing as2org data and re-download later
                if let Err(e) = as2org.clear_db() {
                    eprintln!("Failed to clear database: {}", e);
                    std::process::exit(1);
                }
            }

            if as2org.is_db_empty() {
                println!("bootstrapping as2org data now... (it will take about one minute)");
                if let Err(e) = as2org.parse_insert_as2org(None) {
                    eprintln!("Failed to bootstrap AS2org data: {}", e);
                    std::process::exit(1);
                }
                println!("bootstrapping as2org data finished");
            }

            let mut search_type: SearchType = match (name_only, asn_only) {
                (true, false) => SearchType::NameOnly,
                (false, true) => SearchType::AsnOnly,
                (false, false) => SearchType::Guess,
                (true, true) => {
                    eprintln!("ERROR: name-only and asn-only cannot be both true");
                    return;
                }
            };

            if country_only {
                search_type = SearchType::CountryOnly;
            }

            let mut res = query
                .into_iter()
                .flat_map(
                    |q| match as2org.search(q.as_str(), &search_type, full_country) {
                        Ok(results) => results,
                        Err(e) => {
                            eprintln!("Search error for '{}': {}", q, e);
                            Vec::new()
                        }
                    },
                )
                .collect::<Vec<SearchResult>>();

            // order search results by AS number
            res.sort_by_key(|v| v.asn);

            match full_table {
                false => {
                    let res_concise = res.into_iter().map(|x: SearchResult| SearchResultConcise {
                        asn: x.asn,
                        as_name: x.as_name,
                        org_name: x.org_name,
                        org_country: x.org_country,
                    });
                    if psv {
                        println!("asn|asn_name|org_name|org_country");
                        for res in res_concise {
                            println!(
                                "{}|{}|{}|{}",
                                res.asn, res.as_name, res.org_name, res.org_country
                            );
                        }
                        return;
                    }

                    match pretty {
                        true => {
                            println!("{}", Table::new(res_concise).with(Style::rounded()));
                        }
                        false => {
                            println!("{}", Table::new(res_concise).with(Style::markdown()));
                        }
                    };
                }
                true => {
                    if psv {
                        println!("asn|asn_name|org_name|org_id|org_country|org_size");
                        for entry in res {
                            println!(
                                "{}|{}|{}|{}|{}|{}",
                                entry.asn,
                                entry.as_name,
                                entry.org_name,
                                entry.org_id,
                                entry.org_country,
                                entry.org_size
                            );
                        }
                        return;
                    }
                    match pretty {
                        true => {
                            println!("{}", Table::new(res).with(Style::rounded()));
                        }
                        false => {
                            println!("{}", Table::new(res).with(Style::markdown()));
                        }
                    };
                }
            }
        }
        Commands::Time { time, simple } => {
            let timestring_res = match simple {
                true => parse_time_string_to_rfc3339(&time),
                false => time_to_table(&time),
            };
            match timestring_res {
                Ok(t) => {
                    println!("{t}")
                }
                Err(e) => {
                    eprintln!("ERROR: {e}")
                }
            };
        }
        Commands::Country { queries } => {
            let lookup = CountryLookup::new();
            let res: Vec<CountryEntry> = queries
                .into_iter()
                .flat_map(|query| lookup.lookup(query.as_str()))
                .collect();
            println!("{}", Table::new(res).with(Style::rounded()));
        }
        Commands::Rpki { commands } => match commands {
            RpkiCommands::Check { asn, prefix } => {
                let (validity, roas) = match validate(asn, prefix.as_str()) {
                    Ok((v1, v2)) => (v1, v2),
                    Err(e) => {
                        eprintln!("ERROR: unable to check RPKI validity: {}", e);
                        return;
                    }
                };
                if json {
                    let roa_items: Vec<RoaTableItem> =
                        roas.into_iter().map(RoaTableItem::from).collect();
                    let output = json!({
                        "validation": validity,
                        "covering_roas": roa_items
                    });
                    println!("{}", output);
                } else {
                    println!("RPKI validation result:");
                    println!("{}", Table::new(vec![validity]).with(Style::markdown()));
                    println!();
                    println!("Covering prefixes:");
                    println!(
                        "{}",
                        Table::new(
                            roas.into_iter()
                                .map(RoaTableItem::from)
                                .collect::<Vec<RoaTableItem>>()
                        )
                        .with(Style::markdown())
                    );
                }
            }
            RpkiCommands::List { resource } => {
                let resources = match resource.parse::<u32>() {
                    Ok(asn) => match list_by_asn(asn) {
                        Ok(resources) => resources,
                        Err(e) => {
                            eprintln!("Failed to list ROAs for ASN {}: {}", asn, e);
                            return;
                        }
                    },
                    Err(_) => match resource.parse::<IpNet>() {
                        Ok(prefix) => match list_by_prefix(&prefix) {
                            Ok(resources) => resources,
                            Err(e) => {
                                eprintln!("Failed to list ROAs for prefix {}: {}", prefix, e);
                                return;
                            }
                        },
                        Err(_) => {
                            eprintln!(
                                "ERROR: list resource not an AS number or a prefix: {}",
                                resource
                            );
                            return;
                        }
                    },
                };

                let roas: Vec<RoaTableItem> = resources
                    .into_iter()
                    .flat_map(Into::<Vec<RoaTableItem>>::into)
                    .collect();
                if json {
                    match serde_json::to_string(&roas) {
                        Ok(json_str) => println!("{}", json_str),
                        Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                    }
                } else if roas.is_empty() {
                    println!("no matching ROAS found for {}", resource);
                } else {
                    println!("{}", Table::new(roas).with(Style::markdown()));
                }
            }
            RpkiCommands::Summary { asns } => {
                let res: Vec<SummaryTableItem> = asns
                    .into_iter()
                    .filter_map(|v| match summarize_asn(v) {
                        Ok(summary) => Some(summary),
                        Err(e) => {
                            eprintln!("Failed to summarize ASN {}: {}", v, e);
                            None
                        }
                    })
                    .collect();

                if json {
                    match serde_json::to_string(&res) {
                        Ok(json_str) => println!("{}", json_str),
                        Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                    }
                } else {
                    println!("{}", Table::new(res).with(Style::markdown()));
                }
            }
            RpkiCommands::Roas {
                origin,
                prefix,
                date,
                source,
                collector,
            } => {
                // Parse date if provided
                let parsed_date = match &date {
                    Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                        Ok(date) => Some(date),
                        Err(e) => {
                            eprintln!("ERROR: Invalid date format '{}': {}. Use YYYY-MM-DD", d, e);
                            return;
                        }
                    },
                    None => None,
                };

                // Load RPKI data
                let commons = match load_rpki_data(
                    parsed_date,
                    Some(source.as_str()),
                    Some(collector.as_str()),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("ERROR: Failed to load RPKI data: {}", e);
                        return;
                    }
                };

                // Get ROAs with filters
                let roas = match get_roas(&commons, prefix.as_deref(), origin) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("ERROR: Failed to get ROAs: {}", e);
                        return;
                    }
                };

                if json {
                    match serde_json::to_string(&roas) {
                        Ok(json_str) => println!("{}", json_str),
                        Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                    }
                } else if roas.is_empty() {
                    println!("No ROAs found matching the criteria");
                } else {
                    println!(
                        "Found {} ROAs{}",
                        roas.len(),
                        match &date {
                            Some(d) => format!(" (historical data from {})", d),
                            None => " (current data)".to_string(),
                        }
                    );
                    println!("{}", Table::new(roas).with(Style::markdown()));
                }
            }
            RpkiCommands::Aspas {
                customer,
                provider,
                date,
                source,
                collector,
            } => {
                // Parse date if provided
                let parsed_date = match &date {
                    Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
                        Ok(date) => Some(date),
                        Err(e) => {
                            eprintln!("ERROR: Invalid date format '{}': {}. Use YYYY-MM-DD", d, e);
                            return;
                        }
                    },
                    None => None,
                };

                // Load RPKI data
                let commons = match load_rpki_data(
                    parsed_date,
                    Some(source.as_str()),
                    Some(collector.as_str()),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("ERROR: Failed to load RPKI data: {}", e);
                        return;
                    }
                };

                // Get ASPAs with filters
                let aspas = match get_aspas(&commons, customer, provider) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("ERROR: Failed to get ASPAs: {}", e);
                        return;
                    }
                };

                if json {
                    match serde_json::to_string(&aspas) {
                        Ok(json_str) => println!("{}", json_str),
                        Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                    }
                } else if aspas.is_empty() {
                    println!("No ASPAs found matching the criteria");
                } else {
                    println!(
                        "Found {} ASPAs{}",
                        aspas.len(),
                        match &date {
                            Some(d) => format!(" (historical data from {})", d),
                            None => " (current data)".to_string(),
                        }
                    );
                    let table_entries: Vec<AspaTableEntry> =
                        aspas.iter().map(AspaTableEntry::from).collect();
                    println!(
                        "{}",
                        Table::new(table_entries)
                            .with(Style::markdown())
                            .modify(Columns::last(), Width::wrap(60).keep_words(true))
                    );
                }
            }
        },
        Commands::Radar { commands } => {
            let client = match RadarClient::new() {
                Ok(client) => client,
                Err(e) => {
                    eprintln!("Failed to create Radar client: {}", e);
                    std::process::exit(1);
                }
            };

            match commands {
                RadarCommands::Stats { query } => {
                    let (country, asn) = match query {
                        None => (None, None),
                        Some(q) => match q.parse::<u32>() {
                            Ok(asn) => (None, Some(asn)),
                            Err(_) => (Some(q), None),
                        },
                    };

                    let res = match client.get_bgp_routing_stats(asn, country.clone()) {
                        Ok(res) => res,
                        Err(e) => {
                            eprintln!("ERROR: unable to get routing stats: {}", e);
                            return;
                        }
                    };

                    let scope = match (country, &asn) {
                        (None, None) => "global".to_string(),
                        (Some(c), None) => c,
                        (None, Some(asn)) => format!("as{}", asn),
                        (Some(_), Some(_)) => {
                            eprintln!("ERROR: cannot specify both country and ASN");
                            return;
                        }
                    };

                    #[derive(Tabled, Serialize)]
                    struct Stats {
                        pub scope: String,
                        pub origins: u32,
                        pub prefixes: u32,
                        pub rpki_valid: String,
                        pub rpki_invalid: String,
                        pub rpki_unknown: String,
                    }
                    let table_data = vec![
                        Stats {
                            scope: scope.clone(),
                            origins: res.stats.distinct_origins,
                            prefixes: res.stats.distinct_prefixes,
                            rpki_valid: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_valid,
                                (res.stats.routes_valid as f64 / res.stats.routes_total as f64)
                                    * 100.0
                            ),
                            rpki_invalid: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_invalid,
                                (res.stats.routes_invalid as f64 / res.stats.routes_total as f64)
                                    * 100.0
                            ),
                            rpki_unknown: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_unknown,
                                (res.stats.routes_unknown as f64 / res.stats.routes_total as f64)
                                    * 100.0
                            ),
                        },
                        Stats {
                            scope: format!("{} ipv4", scope),
                            origins: res.stats.distinct_origins_ipv4,
                            prefixes: res.stats.distinct_prefixes_ipv4,
                            rpki_valid: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_valid_ipv4,
                                (res.stats.routes_valid_ipv4 as f64
                                    / res.stats.routes_total_ipv4 as f64)
                                    * 100.0
                            ),
                            rpki_invalid: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_invalid_ipv4,
                                (res.stats.routes_invalid_ipv4 as f64
                                    / res.stats.routes_total_ipv4 as f64)
                                    * 100.0
                            ),
                            rpki_unknown: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_unknown_ipv4,
                                (res.stats.routes_unknown_ipv4 as f64
                                    / res.stats.routes_total_ipv4 as f64)
                                    * 100.0
                            ),
                        },
                        Stats {
                            scope: format!("{} ipv6", scope),
                            origins: res.stats.distinct_origins_ipv6,
                            prefixes: res.stats.distinct_prefixes_ipv6,
                            rpki_valid: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_valid_ipv6,
                                (res.stats.routes_valid_ipv6 as f64
                                    / res.stats.routes_total_ipv6 as f64)
                                    * 100.0
                            ),
                            rpki_invalid: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_invalid_ipv6,
                                (res.stats.routes_invalid_ipv6 as f64
                                    / res.stats.routes_total_ipv6 as f64)
                                    * 100.0
                            ),
                            rpki_unknown: format!(
                                "{} ({:.2}%)",
                                res.stats.routes_unknown_ipv6,
                                (res.stats.routes_unknown_ipv6 as f64
                                    / res.stats.routes_total_ipv6 as f64)
                                    * 100.0
                            ),
                        },
                    ];
                    if json {
                        match serde_json::to_string_pretty(&table_data) {
                            Ok(json_str) => println!("{}", json_str),
                            Err(e) => eprintln!("Failed to serialize JSON: {}", e),
                        }
                    } else {
                        println!("{}", Table::new(table_data).with(Style::modern()));
                        println!("\nData generated at {} UTC.", res.meta.data_time);
                    }
                }
                RadarCommands::Pfx2as { query, rpki_status } => {
                    let (asn, prefix) = match query.parse::<u32>() {
                        Ok(asn) => (Some(asn), None),
                        Err(_) => (None, Some(query)),
                    };

                    let rpki = if let Some(rpki_status) = rpki_status {
                        match rpki_status.to_lowercase().as_str() {
                            "valid" | "invalid" | "unknown" => Some(rpki_status),
                            _ => {
                                eprintln!("ERROR: invalid rpki status: {}", rpki_status);
                                return;
                            }
                        }
                    } else {
                        None
                    };

                    let res = match client.get_bgp_prefix_origins(asn, prefix, rpki) {
                        Ok(res) => res,
                        Err(e) => {
                            eprintln!("ERROR: unable to get prefix origins: {}", e);
                            return;
                        }
                    };

                    #[derive(Tabled, Serialize)]
                    struct Pfx2origin {
                        pub prefix: String,
                        pub origin: String,
                        pub rpki: String,
                        pub visibility: String,
                    }

                    if res.prefix_origins.is_empty() {
                        println!("no prefix origins found for the given query");
                        return;
                    }

                    fn count_to_visibility(count: u32, total: u32) -> String {
                        let ratio = count as f64 / total as f64;
                        if ratio > 0.8 {
                            format!("high ({:.2}%)", ratio * 100.0)
                        } else if ratio < 0.2 {
                            format!("low ({:.2}%)", ratio * 100.0)
                        } else {
                            format!("mid ({:.2}%)", ratio * 100.0)
                        }
                    }

                    let table_data = res
                        .prefix_origins
                        .into_iter()
                        .map(|entry| Pfx2origin {
                            prefix: entry.prefix,
                            origin: format!("as{}", entry.origin),
                            rpki: entry.rpki_validation.to_lowercase(),
                            visibility: count_to_visibility(
                                entry.peer_count as u32,
                                res.meta.total_peers as u32,
                            ),
                        })
                        .collect::<Vec<Pfx2origin>>();
                    if json {
                        match serde_json::to_string_pretty(&table_data) {
                            Ok(json_str) => println!("{}", json_str),
                            Err(e) => eprintln!("Error serializing data to JSON: {}", e),
                        }
                    } else {
                        println!("{}", Table::new(table_data).with(Style::modern()));
                        println!("\nData generated at {} UTC.", res.meta.data_time);
                    }
                }
            }
        }
        Commands::Ip { ip, simple } => match fetch_ip_info(ip, simple) {
            Ok(ipinfo) => {
                if simple {
                    println!("{}", ipinfo.ip);
                    return;
                }

                let json_value = json!(&ipinfo);
                if json {
                    if let Err(e) = serde_json::to_writer_pretty(std::io::stdout(), &json_value) {
                        eprintln!("Error writing JSON to stdout: {}", e);
                    }
                } else {
                    let mut table = json_to_table(&json_value);
                    table.collapse();
                    println!("{}", table);
                }
            }
            Err(e) => {
                eprintln!("ERROR: unable to get ip information: {e}");
            }
        },
        Commands::Pfx2as {
            data_file_path,
            input,
            exact_match,
        } => {
            let pfx2as = match Pfx2as::new(Some(data_file_path)) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("ERROR: unable to open data file: {}", e);
                    std::process::exit(1);
                }
            };

            // collect all prefixes to look up
            let mut prefixes: Vec<IpNet> = vec![];
            for i in input {
                match i.parse::<IpNet>() {
                    Ok(p) => prefixes.push(p),
                    Err(_) => {
                        // it might be a data file
                        if let Ok(lines) = oneio::read_lines(i.as_str()) {
                            for line in lines.map_while(Result::ok) {
                                if line.starts_with('#') {
                                    continue;
                                }
                                let trimmed =
                                    line.trim().split(',').next().unwrap_or(line.as_str());
                                if let Ok(p) = trimmed.parse::<IpNet>() {
                                    prefixes.push(p);
                                }
                            }
                        }
                    }
                }
            }

            // map prefix to origins. one prefix may be mapped to multiple origins
            prefixes.sort();
            let mut prefix_origins_map: HashMap<IpNet, HashSet<u32>> = HashMap::new();
            for p in prefixes {
                let origins = match exact_match {
                    true => pfx2as.lookup_exact(p),
                    false => pfx2as.lookup_longest(p),
                };
                prefix_origins_map.entry(p).or_default().extend(origins);
            }

            // display
            if json {
                // map prefix_origin_pairs to a vector of JSON objects each with a
                // "prefix" and "origin" field
                let data = prefix_origins_map
                    .iter()
                    .map(|(p, o)| json!({"prefix": p.to_string(), "origins": o.iter().cloned().collect::<Vec<u32>>()}))
                    .collect::<Vec<Value>>();
                if let Err(e) = serde_json::to_writer_pretty(std::io::stdout(), &data) {
                    eprintln!("Error writing JSON to stdout: {}", e);
                }
            } else {
                for (prefix, origins) in prefix_origins_map {
                    let mut origins_vec = origins.iter().cloned().collect::<Vec<u32>>();
                    origins_vec.sort();
                    println!("{},{}", prefix, origins.iter().join(","));
                }
            }
        }
    }
}
