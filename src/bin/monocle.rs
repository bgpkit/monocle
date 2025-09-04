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
use clap::{Parser, Subcommand};
use ipnet::IpNet;
use itertools::Itertools;
use json_to_table::json_to_table;
use monocle::*;
use radar_rs::RadarClient;
use rayon::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};
use tabled::settings::{Merge, Style};
use tabled::{Table, Tabled};
use tracing::{info, warn, Level};

/// Maximum number of retry attempts (3 attempts total including the first attempt)
const MAX_RETRIES: u32 = 3;

/// Initial retry delay in seconds
const INITIAL_DELAY: u64 = 1;

/// Maximum retry delay in seconds
const MAX_DELAY: u64 = 30;

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
    /// parse a RPKI ROA file
    ReadRoa {
        /// File path to a ROA file (.roa), local or remote.
        #[clap(name = "FILE")]
        file_path: PathBuf,
    },

    /// parse a RPKI ASPA file
    ReadAspa {
        /// File path to an ASPA file (.asa), local or remote.
        #[clap(name = "FILE")]
        file_path: PathBuf,

        #[clap(long)]
        no_merge_dups: bool,
    },

    /// validate a prefix-asn pair with a RPKI validator
    Check {
        #[clap(short, long)]
        asn: u32,

        #[clap(short, long)]
        prefix: String,
    },

    /// list ROAs by ASN or prefix
    List {
        /// prefix or ASN
        #[clap()]
        resource: String,
    },

    /// summarize RPKI status for a list of given ASNs
    Summary {
        #[clap()]
        asns: Vec<u32>,
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

            // it's fine to unwrap as the filters.validate() function has already checked for issues
            let items = match filters.to_broker_items() {
                Ok(items) => items,
                Err(e) => {
                    eprintln!("Failed to convert filters to broker items: {}", e);
                    std::process::exit(1);
                }
            };

            let total_items = items.len();

            let total_size: i64 = items
                .iter()
                .map(|x| {
                    info!(
                        "{},{},{}",
                        x.collector_id.as_str(),
                        x.url.as_str(),
                        x.rough_size
                    );
                    x.rough_size
                })
                .sum::<i64>();
            info!(
                "total of {} files, {} bytes to parse",
                items.len(),
                total_size
            );

            if dry_run {
                println!(
                    "total of {} files, {} bytes to parse",
                    items.len(),
                    total_size
                );
                return;
            }

            let (sender, receiver): (Sender<(BgpElem, String)>, Receiver<(BgpElem, String)>) =
                channel();
            // progress bar
            let (pb_sender, pb_receiver): (Sender<u32>, Receiver<u32>) = channel();

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

                let mut msg_cache = vec![];
                let mut msg_count = 0;

                for (elem, collector) in receiver {
                    msg_count += 1;

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

                    msg_cache.push((elem, collector));
                    if msg_cache.len() >= 100000 {
                        if let Some(db) = &sqlite_db {
                            if let Err(e) = db.insert_elems(&msg_cache) {
                                eprintln!("Failed to insert elements to database: {}", e);
                            }
                        }
                        if let Some((encoder, _writer)) = &mut mrt_writer {
                            for (elem, _) in &msg_cache {
                                encoder.process_elem(elem);
                            }
                        }
                        msg_cache.clear();
                    }
                }

                if !msg_cache.is_empty() {
                    if let Some(db) = &sqlite_db {
                        if let Err(e) = db.insert_elems(&msg_cache) {
                            eprintln!("Failed to insert elements to database: {}", e);
                        }
                    }
                    if let Some((encoder, _writer)) = &mut mrt_writer {
                        for (elem, _) in &msg_cache {
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
                    println!("processed {total_items} files, found {msg_count} messages, written into file {sqlite_path_str}");
                }
            });

            // dedicated thread for progress bar
            let progress_thread = thread::spawn(move || {
                if !show_progress {
                    return;
                }

                let sty = indicatif::ProgressStyle::with_template(
                    "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {eta} left; {msg}",
                )
                .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
                .progress_chars("##-");
                let pb = indicatif::ProgressBar::new(total_items as u64);
                pb.set_style(sty);
                let mut total_count: u64 = 0;
                for count in pb_receiver.iter() {
                    total_count += count as u64;
                    pb.set_message(format!("found {total_count} messages"));
                    pb.inc(1);
                }
            });

            // Create shared structure to collect failed items
            let failed_items = Arc::new(Mutex::new(Vec::<FailedItem>::new()));
            let failed_items_clone = Arc::clone(&failed_items);

            // Initial parallel processing with failure collection
            let pb_sender_clone = pb_sender.clone();
            items.into_par_iter().for_each_with(
                (sender.clone(), pb_sender_clone, failed_items_clone),
                |(s, pb_sender, failed_items), item| {
                    let url = item.url.clone();
                    let collector = item.collector_id.clone();
                    info!("start parsing {}", url.as_str());
                    let parser = match filters.to_parser(url.as_str()) {
                        Ok(p) => p,
                        Err(e) => {
                            let error_msg = format!("Failed to parse {}: {}", url.as_str(), e);
                            eprintln!("{}", error_msg);
                            // Store failed item for retry
                            if let Ok(mut failed) = failed_items.lock() {
                                failed.push(FailedItem::new(item, error_msg));
                            }
                            return;
                        }
                    };

                    let mut elems_count = 0;
                    for elem in parser {
                        if s.send((elem, collector.clone())).is_err() {
                            // Channel closed, break out
                            break;
                        }
                        elems_count += 1;
                    }

                    if show_progress && pb_sender.send(elems_count).is_err() {
                        // Progress channel closed, ignore
                    }
                    info!("finished parsing {}", url.as_str());
                },
            );

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
                info!("Starting retry phase for {} failed items", failed_count);

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
                        info!(
                            "Retrying {} (attempt {}/{}) after {}s delay",
                            failed_item.item.url.as_str(),
                            failed_item.attempt_count + 1,
                            MAX_RETRIES,
                            delay.as_secs()
                        );

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
                                warn!("{}", error_msg);
                                failed_item.increment_attempt(error_msg);
                                new_failures.push(failed_item);
                                continue;
                            }
                        };

                        let mut elems_count = 0;
                        let mut parse_successful = true;

                        for elem in parser {
                            if sender
                                .send((elem, failed_item.item.collector_id.clone()))
                                .is_err()
                            {
                                // Channel closed, mark as failed
                                parse_successful = false;
                                break;
                            }
                            elems_count += 1;
                        }

                        if parse_successful {
                            successful_retries += 1;
                            if show_progress && pb_sender.send(elems_count).is_err() {
                                // Progress channel closed, ignore
                            }
                            info!(
                                "Successfully retried {} (found {} messages)",
                                failed_item.item.url.as_str(),
                                elems_count
                            );
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

            // Close channels to signal completion
            drop(sender);
            drop(pb_sender);

            // wait for the output thread to stop
            if let Err(e) = writer_thread.join() {
                eprintln!("Writer thread failed: {:?}", e);
            }
            if let Err(e) = progress_thread.join() {
                eprintln!("Progress thread failed: {:?}", e);
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
            RpkiCommands::ReadRoa { file_path } => {
                let file_str = match file_path.to_str() {
                    Some(s) => s,
                    None => {
                        eprintln!("Invalid ROA file path");
                        return;
                    }
                };
                let res = match read_roa(file_str) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("ERROR: unable to read ROA file: {}", e);
                        return;
                    }
                };
                println!("{}", Table::new(res).with(Style::markdown()));
            }
            RpkiCommands::ReadAspa {
                file_path,
                no_merge_dups,
            } => {
                let file_str = match file_path.to_str() {
                    Some(s) => s,
                    None => {
                        eprintln!("Invalid ASPA file path");
                        return;
                    }
                };
                let res = match read_aspa(file_str) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("ERROR: unable to read ASPA file: {}", e);
                        return;
                    }
                };
                match no_merge_dups {
                    true => println!("{}", Table::new(res).with(Style::markdown())),
                    false => println!(
                        "{}",
                        Table::new(res)
                            .with(Style::markdown())
                            .with(Merge::vertical())
                    ),
                };
            }
            RpkiCommands::Check { asn, prefix } => {
                let (validity, roas) = match validate(asn, prefix.as_str()) {
                    Ok((v1, v2)) => (v1, v2),
                    Err(e) => {
                        eprintln!("ERROR: unable to check RPKI validity: {}", e);
                        return;
                    }
                };
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
                if roas.is_empty() {
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

                println!("{}", Table::new(res).with(Style::markdown()));
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
