#![allow(clippy::type_complexity)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

use bgpkit_parser::BgpElem;
use clap::{Parser, Subcommand};
use monocle::*;
use serde_json::json;
use tracing::Level;

mod commands;

// Re-export argument types from command modules for use in the Commands enum
use commands::as2rel::As2relArgs;
use commands::broker::BrokerArgs;
use commands::country::CountryArgs;
use commands::ip::IpArgs;
use commands::parse::ParseArgs;
use commands::pfx2as::Pfx2asArgs;
use commands::radar::RadarCommands;
use commands::rpki::RpkiCommands;
use commands::search::SearchArgs;
use commands::time::TimeArgs;
use commands::whois::WhoisArgs;

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
    Parse(ParseArgs),

    /// Query BGPKIT Broker for the meta data of available MRT files.
    Broker(BrokerArgs),

    /// Search BGP messages from all available public MRT files.
    Search(SearchArgs),

    /// ASN and organization lookup utility.
    Whois(WhoisArgs),

    /// Country name and code lookup utilities
    Country(CountryArgs),

    /// Time conversion utilities
    Time(TimeArgs),

    /// RPKI utilities
    Rpki {
        #[clap(subcommand)]
        commands: RpkiCommands,
    },

    /// IP information lookup
    Ip(IpArgs),

    /// Cloudflare Radar API lookup (set CF_API_TOKEN to enable)
    Radar {
        #[clap(subcommand)]
        commands: RadarCommands,
    },

    /// Bulk prefix-to-AS mapping lookup with the pre-generated data file.
    Pfx2as(Pfx2asArgs),

    /// AS-level relationship lookup between ASNs.
    As2rel(As2relArgs),
}

pub(crate) fn elem_to_string(
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
        Commands::Parse(args) => commands::parse::run(args, json),
        Commands::Search(args) => commands::search::run(args, json),
        Commands::Broker(args) => commands::broker::run(args, json),
        Commands::Whois(args) => commands::whois::run(&config, args),
        Commands::Time(args) => commands::time::run(args),
        Commands::Country(args) => commands::country::run(args),
        Commands::Rpki { commands } => commands::rpki::run(commands, json),
        Commands::Radar { commands } => commands::radar::run(commands, json),
        Commands::Ip(args) => commands::ip::run(args, json),
        Commands::Pfx2as(args) => commands::pfx2as::run(args, json),
        Commands::As2rel(args) => commands::as2rel::run(&config, args, json),
    }
}
