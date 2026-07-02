#![allow(clippy::type_complexity)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

use clap::{Args, Parser, Subcommand};
use monocle::utils::OutputFormat;
use monocle::*;
use tracing::Level;

mod commands;

// Re-export argument types from command modules for use in the Commands enum
use commands::as2rel::As2relArgs;
use commands::config::ConfigArgs;
use commands::country::CountryArgs;
use commands::inspect::InspectArgs;
use commands::ip::IpArgs;
use commands::parse::ParseArgs;
use commands::pfx2as::Pfx2asArgs;
use commands::rib::RibArgs;
use commands::rpki::RpkiCommands;
use commands::search::SearchArgs;
use commands::time::TimeArgs;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
struct Cli {
    /// configuration file path (default: $XDG_CONFIG_HOME/monocle/monocle.toml)
    #[clap(short, long)]
    config: Option<String>,

    /// Print debug information
    #[clap(long, global = true)]
    debug: bool,

    /// Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
    #[clap(long, global = true, value_name = "FORMAT")]
    format: Option<OutputFormat>,

    /// Output as JSON objects (shortcut for --format json-pretty)
    #[clap(long, global = true)]
    json: bool,

    /// Disable automatic database updates (use existing cached data only)
    #[clap(long, global = true)]
    no_update: bool,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse individual MRT files given a file path, local or remote.
    Parse(ParseArgs),

    /// Search BGP messages from all available public MRT files.
    Search(SearchArgs),

    /// Reconstruct final RIB state at one or more arbitrary timestamps.
    Rib(RibArgs),

    /// Start the Monocle HTTP service (REST: /api/v1, search stream: /api/v1/search/stream)
    ///
    /// Note: This requires building with the `server` feature enabled.
    Server(ServerArgs),

    /// Unified AS and prefix information lookup
    Inspect(InspectArgs),

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

    /// AS-level relationship lookup between ASNs.
    As2rel(As2relArgs),

    /// Prefix-to-ASN mapping lookup
    ///
    /// Query by prefix to find origin ASNs, or by ASN to find announced prefixes.
    /// Includes RPKI validation status for each prefix-ASN pair.
    Pfx2as(Pfx2asArgs),

    /// Show monocle configuration, data paths, and database management.
    Config(ConfigArgs),
}

#[derive(Args, Debug, Clone)]
struct ServerArgs {
    /// Address to bind to (overrides config server_address)
    #[clap(long)]
    address: Option<String>,

    /// Port to listen on (overrides config server_port)
    #[clap(long)]
    port: Option<u16>,

    /// Maximum number of elements per SSE batch (overrides config)
    #[clap(long)]
    max_search_batch_size: Option<usize>,

    /// Maximum search results per request (0 = unlimited, overrides config)
    #[clap(long)]
    max_search_results: Option<u64>,

    /// Search concurrency (0 = auto/rayon default, overrides config)
    #[clap(long)]
    concurrency: Option<usize>,

    /// Search timeout in seconds (0 = no timeout, overrides config)
    #[clap(long)]
    search_timeout_secs: Option<u64>,

    /// Maximum concurrent SSE search requests (0 = unlimited, overrides config)
    #[clap(long)]
    max_concurrent_searches: Option<usize>,

    /// Enable token auth for /api/v1/* endpoints (overrides config)
    #[clap(long)]
    auth_enabled: Option<bool>,

    /// Bearer token for auth (overrides config)
    #[clap(long)]
    auth_token: Option<String>,
}

fn main() {
    // Reset SIGPIPE signal handling to default behavior (terminate on broken pipe)
    // This prevents panics when output is piped to commands like `head`
    #[cfg(unix)]
    {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }

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
    // Default is Table for most commands, but PSV for parse/search (streaming data)
    let output_format = if let Some(fmt) = cli.format {
        fmt
    } else if cli.json {
        OutputFormat::JsonPretty
    } else {
        OutputFormat::Table
    };

    // Parse and Search commands default to PSV format (better for streaming data)
    let streaming_output_format = if let Some(fmt) = cli.format {
        fmt
    } else if cli.json {
        OutputFormat::JsonPretty
    } else {
        OutputFormat::Psv
    };

    // You can check for the existence of subcommands, and if found, use their
    // matches just as you would the top level cmd
    match cli.command {
        Commands::Parse(args) => commands::parse::run(args, streaming_output_format),
        Commands::Search(args) => commands::search::run(&config, args, streaming_output_format),
        Commands::Rib(args) => {
            commands::rib::run(&config, args, streaming_output_format, cli.no_update)
        }

        Commands::Server(args) => {
            // The server requires the `server` feature (axum + tokio). Keep the CLI
            // binary as the entrypoint, but compile this arm only when `server` is enabled.
            #[cfg(feature = "cli")]
            {
                let mut server_config = config.clone();

                if let Some(addr) = args.address {
                    server_config.server_address = addr;
                }
                if let Some(port) = args.port {
                    server_config.server_port = port;
                }
                if let Some(v) = args.max_search_batch_size {
                    server_config.server_max_search_batch_size = v;
                }
                if let Some(v) = args.max_search_results {
                    server_config.server_max_search_results = v;
                }
                if let Some(v) = args.concurrency {
                    server_config.search_concurrency = v;
                }
                if let Some(v) = args.search_timeout_secs {
                    server_config.server_search_timeout_secs = v;
                }
                if let Some(v) = args.max_concurrent_searches {
                    server_config.server_max_concurrent_searches = v;
                }
                if let Some(v) = args.auth_enabled {
                    server_config.server_auth_enabled = v;
                }
                if let Some(v) = args.auth_token {
                    server_config.server_auth_token = v;
                }

                // Start server (blocks current thread until shutdown)
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("Failed to create tokio runtime for server: {e}");
                        std::process::exit(1);
                    }
                };
                if let Err(e) = rt.block_on(monocle::server::start_server(server_config)) {
                    eprintln!("Server failed: {e}");
                    std::process::exit(1);
                }
            }

            #[cfg(not(feature = "cli"))]
            {
                let _ = args;
                eprintln!("ERROR: server subcommand requires building with --features cli");
                std::process::exit(2);
            }
        }

        Commands::Inspect(args) => {
            commands::inspect::run(&config, args, output_format, cli.no_update)
        }
        Commands::Time(args) => commands::time::run(args, output_format),
        Commands::Country(args) => commands::country::run(args, output_format),
        Commands::Rpki { commands } => {
            commands::rpki::run(commands, output_format, &config, cli.no_update)
        }
        Commands::Ip(args) => commands::ip::run(args, output_format),
        Commands::As2rel(args) => {
            commands::as2rel::run(&config, args, output_format, cli.no_update)
        }
        Commands::Pfx2as(args) => {
            commands::pfx2as::run(&config, args, output_format, cli.no_update)
        }
        Commands::Config(args) => commands::config::run(&config, args, output_format),
    }
}
