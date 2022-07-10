use clap::{Args, Parser, Subcommand};
use serde_json::json;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;

use monocle::parser_with_filters;
use rayon::prelude::*;
use std::sync::mpsc::channel;
use std::thread;
use bgpkit_broker::QueryParams;
use tracing::{info, Level};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
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
    start_ts: Option<f64>,

    /// Filter by end unix timestamp inclusive
    #[clap(short = 'T', long)]
    end_ts: Option<f64>,

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
    start_ts: f64,

    /// Filter by end unix timestamp inclusive
    #[clap(short = 'T', long)]
    end_ts: f64,

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

#[derive(Subcommand)]
enum Commands {
    /// Adds files to myapp
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
    Search {

        /// Print debug information
        #[clap(short = 'd', long)]
        debug: bool,

        /// Dry-run, do not download or parse.
        #[clap(short = 'd', long)]
        dry_run: bool,

        /// Filter by AS path regex string
        #[clap(flatten)]
        filters: SearchFilters,
    },
    Scouter {
        /// Measure the power of your enemy
        #[clap()]
        power: bool
    }
}

fn main() {
    let cli = Cli::parse();

    // You can check for the existence of subcommands, and if found use their
    // matches just as you would the top level cmd
    match cli.command {
        Commands::Parse {
            file_path,
            json,
            pretty,
            filters,
        } => {
            let parser = parser_with_filters(
                file_path.to_str().unwrap(),
                &filters.origin_asn,
                &filters.prefix,
                &filters.include_super,
                &filters.include_sub,
                &filters.peer_ip,
                &filters.peer_asn,
                &filters.elem_type,
                &filters.start_ts,
                &filters.end_ts,
                &filters.as_path,
            );

            let mut stdout = std::io::stdout();
            for elem in parser {
                let output_str = if json {
                    let val = json!(elem);
                    if pretty {
                        serde_json::to_string_pretty(&val).unwrap()
                    } else {
                        val.to_string()
                    }
                } else {
                    elem.to_string()
                };
                if let Err(e) = writeln!(stdout, "{}", &output_str) {
                    if e.kind() != std::io::ErrorKind::BrokenPipe {
                        eprintln!("{}", e);
                    }
                    std::process::exit(1);
                }
            }
        },
        Commands::Search { debug, dry_run, filters } => {

            if debug {
                tracing_subscriber::fmt()
                    // filter spans/events with level TRACE or higher.
                    .with_max_level(Level::INFO)
                    .init();
            }

            let broker = bgpkit_broker::BgpkitBroker::new("https://api.broker.bgpkit.com/v2");
            let mut params = QueryParams{
                ts_start: Some(filters.start_ts.to_string()),
                ts_end: Some(filters.end_ts.to_string()),
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
            ).unwrap();

            let total_size: i64 = items.iter().map(|x|x.rough_size).sum::<i64>();
            info!("total of {} files, {} bytes to parse", items.len(), total_size);

            if dry_run {
                println!("total of {} files, {} bytes to parse", items.len(), total_size);
                return
            }

            let (sender, receiver) = channel();

            // dedicated thread for handling output of results
            let writer_thread = thread::spawn(move || {
                for elem in receiver {
                    println!("{}", elem);
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
                    &Some(filters.start_ts),
                    &Some(filters.end_ts),
                    &filters.as_path,
                );

                for elem in parser {
                    s.send(elem).unwrap()
                }
                info!("finished parsing {}", url.as_str());
            });

            // wait for the output thread to stop
            writer_thread.join().unwrap();

            /*
             */
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
