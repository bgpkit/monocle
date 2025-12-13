#![allow(clippy::type_complexity)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

use clap::{Parser, Subcommand};
use monocle::lens::utils::OutputFormat;
use monocle::*;
use tracing::Level;

mod commands;

// Re-export argument types from command modules for use in the Commands enum
use commands::as2rel::As2relArgs;
use commands::config::ConfigArgs;
use commands::country::CountryArgs;
use commands::ip::IpArgs;
use commands::parse::ParseArgs;
use commands::pfx2as::Pfx2asArgs;
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

    /// Output format: table (default), markdown, json, json-pretty, json-line, psv
    #[clap(long, global = true, value_name = "FORMAT")]
    format: Option<OutputFormat>,

    /// Output as JSON objects (shortcut for --format json-pretty)
    #[clap(long, global = true)]
    json: bool,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse individual MRT files given a file path, local or remote.
    Parse(ParseArgs),

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

    /// Bulk prefix-to-AS mapping lookup with the pre-generated data file.
    Pfx2as(Pfx2asArgs),

    /// AS-level relationship lookup between ASNs.
    As2rel(As2relArgs),

    /// Show monocle configuration and data paths.
    Config(ConfigArgs),
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

    // Determine output format: explicit --format takes precedence, then --json flag
    let output_format = if let Some(fmt) = cli.format {
        fmt
    } else if cli.json {
        OutputFormat::JsonPretty
    } else {
        OutputFormat::Table
    };

    // You can check for the existence of subcommands, and if found, use their
    // matches just as you would the top level cmd
    match cli.command {
        Commands::Parse(args) => commands::parse::run(args, output_format),
        Commands::Search(args) => commands::search::run(args, output_format),
        Commands::Whois(args) => commands::whois::run(&config, args, output_format),
        Commands::Time(args) => commands::time::run(args, output_format),
        Commands::Country(args) => commands::country::run(args, output_format),
        Commands::Rpki { commands } => {
            commands::rpki::run(commands, output_format, &config.data_dir)
        }
        Commands::Ip(args) => commands::ip::run(args, output_format),
        Commands::Pfx2as(args) => commands::pfx2as::run(&config, args, output_format),
        Commands::As2rel(args) => commands::as2rel::run(&config, args, output_format),
        Commands::Config(args) => commands::config::run(&config, args, output_format),
    }
}
