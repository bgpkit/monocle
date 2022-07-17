use clap::{Args, Parser, Subcommand};
use serde_json::json;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;

use monocle::{As2org, MonocleConfig, parser_with_filters, SearchType, string_to_time, time_to_table};
use rayon::prelude::*;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use bgpkit_broker::QueryParams;
use tracing::{info, Level};

use anyhow::{anyhow, Result};
use bgpkit_parser::BgpElem;
use chrono::DateTime;
use tabled::Table;

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
        #[clap(name = "FILE", parse(from_os_str))]
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
        /// Print debug information
        #[clap(long)]
        debug: bool,

        /// Dry-run, do not download or parse.
        #[clap(long)]
        dry_run: bool,

        /// Output as JSON objects
        #[clap(long)]
        json: bool,

        /// Pretty-print JSON output
        #[clap(long)]
        pretty: bool,

        /// Filter by AS path regex string
        #[clap(flatten)]
        filters: SearchFilters,
    },
    /// offline ASN and organization name lookup
    As {
        /// search query, an ASN (e.g. "400644") or a name (e.g. "bgpkit")
        query: String,

        /// search AS and Org name only
        name_only: bool,

        /// search by ASN only
        asn_only: bool,
    },
    /// Time conversion utilities
    Time {
        /// Time stamp or time string to convert
        #[clap()]
        time: Option<String>,
    },
    /// Investigative toolbox
    Scouter {
        /// Measure the power of your enemy
        #[clap()]
        power: bool
    }
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

    let _config = MonocleConfig::load(&cli.config);

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
                eprintln!("{}", e.to_string());
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
                        eprintln!("{}", e);
                    }
                    std::process::exit(1);
                }
            }
        },
        Commands::Search { debug, dry_run, json, pretty, filters } => {
            if let Err(e) = filters.validate() {
                eprintln!("{}", e.to_string());
                return
            }

            if debug {
                tracing_subscriber::fmt()
                    // filter spans/events with level TRACE or higher.
                    .with_max_level(Level::INFO)
                    .init();
            }

            let broker = bgpkit_broker::BgpkitBroker::new("https://api.broker.bgpkit.com/v2");
            let ts_start = string_to_time(filters.start_ts.as_str()).unwrap().to_string();
            let ts_end = string_to_time(filters.end_ts.as_str()).unwrap().to_string();
            let mut params = QueryParams{
                ts_start: Some(ts_start),
                ts_end: Some(ts_end),
                data_type: Some("update".to_string()),
                page_size: 1000,
                ..Default::default()
            };

            if let Some(project) = filters.project {
                params.project = Some(project);
            }
            if let Some(collector) = filters.collector {
                params.collector_id = Some(collector)
            }

            let items = broker.query_all(
                &params
            ).expect("broker query error: please check filters are valid");

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

            // dedicated thread for handling output of results
            let writer_thread = thread::spawn(move || {
                for elem in receiver {
                    let output_str = elem_to_string(&elem, json, pretty);
                    println!("{}", output_str);
                }
            });

            let urls = items.iter().map(|x| x.url.to_string()).collect::<Vec<String>>();

            urls.into_par_iter().for_each_with(sender, |s, url| {
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
                info!("finished parsing {}", url.as_str());
            });

            // wait for the output thread to stop
            writer_thread.join().unwrap();
        }
        Commands::As { query, name_only, asn_only } => {
            // TODO: use configuration file to get data location
            // TODO: crawl the newest data file and save the data file url in sqlite db.

            let as2org = As2org::new(&Some("monocle-test.sqlite3".to_string())).unwrap();
            if as2org.is_db_empty() {
                as2org.parse_as2org("https://publicdata.caida.org/datasets/as-organizations/20220701.as-org2info.jsonl.gz").unwrap();
            }

            let search_type: SearchType = match (name_only, asn_only) {
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

            let res = as2org.search(query.as_str(), &search_type).unwrap();
            println!("{}", Table::new(res).to_string());
        }
        Commands::Time { time} => {
            match time_to_table(&time) {
                Ok(t) => {
                    println!("{}", t)
                }
                Err(e) => {
                    eprintln!("{}", e)
                }
            }
        }
        Commands::Scouter {
            power: _
        } => {
            // https://dragonball.fandom.com/wiki/It%27s_Over_9000!
            println!("It's Over 9000!");
            println!("What!? 9000!? There's no way that can be right! Can it!?");
        }
    }
}