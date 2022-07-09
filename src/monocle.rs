use serde_json::json;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use clap::{Parser, Subcommand};

use std::sync::mpsc::channel;
use rayon::prelude::*;
use bgpkit_parser::*;
use ipnetwork::IpNetwork;
use monocle::parser_with_filters;

#[allow(dead_code)]
fn try_parallel() {
    let (sender, receiver) = channel();

    let urls = [
        ("amsix", "http://archive.routeviews.org/route-views.amsix/bgpdata/2022.07/UPDATES/updates.20220708.2045.bz2"),
        ("eqix", "http://archive.routeviews.org/route-views.eqix/bgpdata/2022.07/UPDATES/updates.20220709.2200.bz2"),
    ];

    urls.into_par_iter().for_each_with(sender, |s, x| {
        let parser = BgpkitParser::new(x.1).unwrap();
        for elem in parser {
            s.send((x.0, elem)).unwrap()
        }
    });

    for (collector, elem) in receiver {
        println!("{}\t{}", collector, elem);
    }
}

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Adds files to myapp
    Parse {
        /// File path to a MRT file, local or remote.
        #[clap(name="FILE", parse(from_os_str))]
        file_path: PathBuf,

        /// Output as JSON objects
        #[clap(long)]
        json: bool,

        /// Pretty-print JSON output
        #[clap(long)]
        pretty: bool,

        /// Filter by origin AS Number
        #[clap(short = 'o', long)]
        origin_asn: Option<u32>,

        /// Filter by network prefix
        #[clap(short = 'p', long)]
        prefix: Option<IpNetwork>,

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
    },
}

fn main() {
    let cli = Cli::parse();

    // You can check for the existence of subcommands, and if found use their
    // matches just as you would the top level cmd
    match cli.command {
        Commands::Parse {
            file_path, json, pretty,
            origin_asn, prefix, include_super,
            include_sub, peer_ip, peer_asn, elem_type,
            start_ts, end_ts, as_path
        } => {

            let parser = parser_with_filters(file_path.to_str().unwrap(), &origin_asn, &prefix, include_super, include_sub, &peer_ip, &peer_asn, &elem_type, &start_ts, &end_ts, &as_path);

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
        }
    }
}