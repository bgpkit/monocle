use clap::{Args, Parser, Subcommand};
use serde_json::json;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;

use monocle::{As2org, CountryLookup, MonocleConfig, MsgStore, parser_with_filters, SearchResult, SearchResultConcise, SearchType, string_to_time, time_to_table};
use rayon::prelude::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use tracing::{info, Level};

use anyhow::{anyhow, Result};
use bgpkit_parser::BgpElem;
use chrono::DateTime;
use tabled::{Style, Table};

trait Validate{
    fn validate(&self) -> Result<()>;
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    /// configuration file path, by default $HOME/.monocle.toml is used
    #[clap(short, long)]
    config: Option<String>,

    /// Print debug information
    #[clap(long)]
    debug: bool,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Args, Debug)]
struct ParseFilters {
    /// Filter by origin AS Number
    #[clap(short = 'o', long)]
    origin_asn: Option<u32>,

    /// Filter by network prefix
    #[clap(short = 'p', long)]
    prefix: Option<String>,

    /// Include super-prefix when filtering
    #[clap(short = 's', long)]
    include_super: bool,

    /// Include sub-prefix when filtering
    #[clap(short = 'S', long)]
    include_sub: bool,

    /// Filter by peer IP address
    #[clap(short = 'j', long)]
    peer_ip: Vec<IpAddr>,

    /// Filter by peer ASN
    #[clap(short = 'J', long)]
    peer_asn: Option<u32>,

    /// Filter by elem type: announce (a) or withdraw (w)
    #[clap(short = 'm', long)]
    elem_type: Option<String>,

    /// Filter by start unix timestamp inclusive
    #[clap(short = 't', long)]
    start_ts: Option<String>,

    /// Filter by end unix timestamp inclusive
    #[clap(short = 'T', long)]
    end_ts: Option<String>,

    /// Filter by AS path regex string
    #[clap(short = 'a', long)]
    as_path: Option<String>,
}

#[derive(Args, Debug)]
struct SearchFilters {
    /*
    REQUIRED ARGUMENTS
     */

    /// Filter by start unix timestamp inclusive
    #[clap(short = 't', long)]
    start_ts: String,

    /// Filter by end unix timestamp inclusive
    #[clap(short = 'T', long)]
    end_ts: String,

    /*
    OPTIONAL ARGUMENTS
     */

    /// Filter by collector, e.g. rrc00 or route-views2
    #[clap(short = 'c', long)]
    collector: Option<String>,

    /// Filter by route collection project, i.e. riperis or routeviews
    #[clap(short = 'P', long)]
    project: Option<String>,

    /// Filter by origin AS Number
    #[clap(short = 'o', long)]
    origin_asn: Option<u32>,

    /// Filter by network prefix
    #[clap(short = 'p', long)]
    prefix: Option<String>,

    /// Include super-prefix when filtering
    #[clap(short = 's', long)]
    include_super: bool,

    /// Include sub-prefix when filtering
    #[clap(short = 'S', long)]
    include_sub: bool,

    /// Filter by peer IP address
    #[clap(short = 'j', long)]
    peer_ip: Vec<IpAddr>,

    /// Filter by peer ASN
    #[clap(short = 'J', long)]
    peer_asn: Option<u32>,

    /// Filter by elem type: announce (a) or withdraw (w)
    #[clap(short = 'm', long)]
    elem_type: Option<String>,

    /// Filter by AS path regex string
    #[clap(short = 'a', long)]
    as_path: Option<String>,
}

impl Validate for ParseFilters {
    fn validate(&self) -> Result<()> {
        if let Some(ts) = &self.start_ts {
            if ! (ts.parse::<f64>().is_ok() || DateTime::parse_from_rfc3339(ts).is_ok()){
                // not a number or a rfc3339 string
                return Err(anyhow!("start-ts must be either a unix-timestamp or a RFC3339-compliant string"))
            }
        }
        if let Some(ts) = &self.end_ts {
            if ! (ts.parse::<f64>().is_ok() || DateTime::parse_from_rfc3339(ts).is_ok()){
                // not a number or a rfc3339 string
                return Err(anyhow!("end-ts must be either a unix-timestamp or a RFC3339-compliant string"))
            }
        }

        Ok(())
    }
}

impl Validate for SearchFilters {
    fn validate(&self) -> Result<()> {
        if ! (self.start_ts.as_str().parse::<f64>().is_ok() || DateTime::parse_from_rfc3339(self.start_ts.as_str()).is_ok()){
            // not a number or a rfc3339 string
            return Err(anyhow!("start-ts must be either a unix-timestamp or a RFC3339-compliant string: {}", self.start_ts))
        }
        if ! (self.end_ts.as_str().parse::<f64>().is_ok() || DateTime::parse_from_rfc3339(self.end_ts.as_str()).is_ok()){
            // not a number or a rfc3339 string
            return Err(anyhow!("end-ts must be either a unix-timestamp or a RFC3339-compliant string: {}", self.end_ts))
        }

        Ok(())
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Parse individual MRT files given a file path, local or remote.
    Parse {
        /// File path to a MRT file, local or remote.
        #[clap(name = "FILE")]
        file_path: PathBuf,

        /// Output as JSON objects
        #[clap(long)]
        json: bool,

        /// Pretty-print JSON output
        #[clap(long)]
        pretty: bool,

        /// Filter by AS path regex string
        #[clap(flatten)]
        filters: ParseFilters,
    },

    /// Search BGP messages from all available public MRT files.
    Search {
        /// Dry-run, do not download or parse.
        #[clap(long)]
        dry_run: bool,

        /// Output as JSON objects
        #[clap(long)]
        json: bool,

        /// Pretty-print JSON output
        #[clap(long)]
        pretty: bool,

        /// SQLite output file path
        #[clap(long)]
        sqlite_path: Option<PathBuf>,

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
        #[clap(short='C', long)]
        country_only: bool,

        /// Refresh local as2org database
        #[clap(short, long)]
        update: bool,

        /// Output to pretty table, default markdown table
        #[clap(short, long)]
        pretty: bool,

        /// Display full table (with ord_id, org_size)
        #[clap(short='F', long)]
        full_table: bool,

        /// Export to pipe-separated values
        #[clap(short='P', long)]
        psv: bool,

        /// Show full country names instead of 2-letter code
        #[clap(short, long)]
        full_country: bool,
    },

    /// Country name and code lookup utilities
    Country {
        /// Search query, e.g. "US" or "United States"
        query: String,
    },
    /// Time conversion utilities
    Time {
        /// Time stamp or time string to convert
        #[clap()]
        time: Option<String>,
    },
}

fn elem_to_string(elem: &BgpElem, json: bool, pretty: bool) -> String {
    if json {
        let val = json!(elem);
        if pretty {
            serde_json::to_string_pretty(&val).unwrap()
        } else {
            val.to_string()
        }
    } else {
        elem.to_string()
    }
}

fn main() {
    let cli = Cli::parse();

    let config = MonocleConfig::new(&cli.config);

    if cli.debug {
        tracing_subscriber::fmt()
            // filter spans/events with level TRACE or higher.
            .with_max_level(Level::INFO)
            .init();
    }


    // You can check for the existence of subcommands, and if found use their
    // matches just as you would the top level cmd
    match cli.command {
        Commands::Parse {
            file_path,
            json,
            pretty,
            filters,
        } => {
            if let Err(e) = filters.validate() {
                eprintln!("{e}");
                return
            }

            let parser = parser_with_filters(
                file_path.to_str().unwrap(),
                &filters.origin_asn,
                &filters.prefix,
                &filters.include_super,
                &filters.include_sub,
                &filters.peer_ip,
                &filters.peer_asn,
                &filters.elem_type,
                &filters.start_ts.clone(),
                &filters.end_ts.clone(),
                &filters.as_path,
            ).unwrap();

            let mut stdout = std::io::stdout();
            for elem in parser {
                let output_str = elem_to_string(&elem, json, pretty);
                if let Err(e) = writeln!(stdout, "{}", &output_str) {
                    if e.kind() != std::io::ErrorKind::BrokenPipe {
                        eprintln!("{e}");
                    }
                    std::process::exit(1);
                }
            }
        },
        Commands::Search { dry_run, json, pretty, sqlite_path, sqlite_reset, filters } => {
            if let Err(e) = filters.validate() {
                eprintln!("{e}");
                return
            }

            let mut show_progress = false;
            let mut sqlite_path_str = "".to_string();
            let sqlite_db = match sqlite_path {
                Some(path) => {
                    show_progress = true;
                    sqlite_path_str = path.to_str().unwrap().to_string();
                    Some(MsgStore::new(&Some(sqlite_path_str.clone()), sqlite_reset))
                }
                None => {None}
            };

            let ts_start = string_to_time(filters.start_ts.as_str()).unwrap().to_string();
            let ts_end = string_to_time(filters.end_ts.as_str()).unwrap().to_string();

            let mut broker = bgpkit_broker::BgpkitBroker::new()
                .ts_start(ts_start.as_str())
                .ts_end(ts_end.as_str())
                .data_type("update")
                .page_size(1000);

            if let Some(project) = filters.project {
                broker = broker.project(project.as_str());

            }
            if let Some(collector) = filters.collector {
                broker = broker.collector_id(collector.as_str());
            }



            let items = broker.query(
            ).expect("broker query error: please check filters are valid");

            let total_items = items.len();

            let total_size: i64 = items.iter().map(|x|{
                info!("{},{},{}", x.collector_id.as_str(), x.url.as_str(), x.rough_size);
                x.rough_size
            }).sum::<i64>();
            info!("total of {} files, {} bytes to parse", items.len(), total_size);

            if dry_run {
                println!("total of {} files, {} bytes to parse", items.len(), total_size);
                return
            }

            let (sender, receiver): (Sender<BgpElem>, Receiver<BgpElem>) = channel();
            // progress bar
            let (pb_sender, pb_receiver): (Sender<u8>, Receiver<u8>) = channel();

            // dedicated thread for handling output of results
            let writer_thread = thread::spawn(move || {
                match sqlite_db {
                    Some(db) => {
                        let mut msg_cache = vec![];
                        let mut msg_count = 0;
                        for elem in receiver {
                            msg_count += 1;
                            msg_cache.push(elem);
                            if msg_cache.len()>=500 {
                                db.insert_elems(&msg_cache);
                                msg_cache.clear();
                            }
                        }
                        if !msg_cache.is_empty() {
                            db.insert_elems(&msg_cache);
                        }

                        println!("processed {total_items} files, found {msg_count} messages, written into file {sqlite_path_str}");
                    }
                    None => {
                        for elem in receiver {
                            let output_str = elem_to_string(&elem, json, pretty);
                            println!("{output_str}");
                        }
                    }
                }
            });

            // dedicated thread for progress bar
            let progress_thread = thread::spawn(move || {
                if !show_progress {
                    return
                }

                let sty = indicatif::ProgressStyle::with_template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {eta}")
                    .unwrap()
                    .progress_chars("##-");
                let pb = indicatif::ProgressBar::new(total_items as u64);
                pb.set_style(sty);
                for _ in pb_receiver.iter() {
                    pb.inc(1);
                }
            });

            let urls = items.iter().map(|x| x.url.to_string()).collect::<Vec<String>>();

            urls.into_par_iter().for_each_with((sender, pb_sender), |(s, pb_sender), url| {
                info!("start parsing {}", url.as_str());
                let parser = parser_with_filters(
                    url.as_str(),
                    &filters.origin_asn,
                    &filters.prefix,
                    &filters.include_super,
                    &filters.include_sub,
                    &filters.peer_ip,
                    &filters.peer_asn,
                    &filters.elem_type,
                    &Some(filters.start_ts.clone()),
                    &Some(filters.end_ts.clone()),
                    &filters.as_path,
                ).unwrap();

                for elem in parser {
                    s.send(elem).unwrap()
                }

                if show_progress{
                    pb_sender.send(0).unwrap();
                }
                info!("finished parsing {}", url.as_str());
            });

            // wait for the output thread to stop
            writer_thread.join().unwrap();
            progress_thread.join().unwrap();
        }
        Commands::Whois { query, name_only, asn_only ,update, pretty, full_table, full_country, country_only, psv} => {
            let data_dir = config.data_dir.as_str();
            let as2org = As2org::new(&Some(format!("{data_dir}/monocle-data.sqlite3"))).unwrap();

            if update{
                // if update flag is set, clear existing as2org data and re-download later
                as2org.clear_db();
            }

            if as2org.is_db_empty() {
                println!("bootstrapping as2org data now... (it will take about one minute)");
                as2org.parse_insert_as2org(None).unwrap();
                println!("bootstrapping as2org data finished");
            }

            let mut search_type: SearchType = match (name_only, asn_only) {
                (true, false) => {
                    SearchType::NameOnly
                }
                (false, true) => {
                    SearchType::AsnOnly
                }
                (false, false) => {
                    SearchType::Guess
                }
                (true, true) => {
                    eprintln!("name-only and asn-only cannot be both true");
                    return
                }
            };

            if country_only {
                search_type = SearchType::CountryOnly;
            }

            let mut res = query.into_iter().flat_map(|q| {
                as2org.search(q.as_str(), &search_type, full_country).unwrap()
            }).collect::<Vec<SearchResult>>();

            // order search results by AS number
            res.sort_by_key(|v| v.asn);

            match full_table {
                false => {
                    let res_concise = res.into_iter().map(|x: SearchResult|{
                        SearchResultConcise { asn: x.asn, as_name: x.as_name, org_name: x.org_name, org_country: x.org_country }
                    });
                    if psv {
                        println!("asn|asn_name|org_name|org_country");
                        for res in res_concise {
                            println!("{}|{}|{}|{}", res.asn, res.as_name, res.org_name, res.org_country);
                        }
                        return
                    }

                    match pretty {
                        true => {
                            println!("{}", Table::new(res_concise).with(Style::rounded()));
                        }
                        false => {
                            println!("{}", Table::new(res_concise).with(Style::markdown()));
                        }
                    };
                },
                true => {
                    if psv {
                        println!("asn|asn_name|org_name|org_id|org_country|org_size");
                        for entry in res {
                            format!("{}|{}|{}|{}|{}|{}", entry.asn, entry.as_name, entry.org_name, entry.org_id, entry.org_country, entry.org_size);
                        }
                        return
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
        Commands::Time { time} => {
            match time_to_table(&time) {
                Ok(t) => {
                    println!("{t}")
                }
                Err(e) => {
                    eprintln!("{e}")
                }
            }
        }
        Commands::Country {query} => {
            let lookup = CountryLookup::new();
            let res = lookup.lookup(query.as_str());
            println!("{}", Table::new(res).with(Style::rounded()));
        }
    }
}
