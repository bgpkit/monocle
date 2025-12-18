#![allow(clippy::type_complexity)]
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]

use clap::{Args, Parser, Subcommand};
use monocle::lens::utils::OutputFormat;
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
use commands::rpki::RpkiCommands;
use commands::search::SearchArgs;
use commands::time::TimeArgs;

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

    /// Disable automatic data refresh (use existing cached data only)
    #[clap(long, global = true)]
    no_refresh: bool,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse individual MRT files given a file path, local or remote.
    Parse(ParseArgs),

    /// Search BGP messages from all available public MRT files.
    Search(SearchArgs),

    /// Start the WebSocket server (ws://<address>:<port>/ws, health: http://<address>:<port>/health)
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

    /// Show monocle configuration, data paths, and database management.
    Config(ConfigArgs),
}

#[derive(Args, Debug, Clone)]
struct ServerArgs {
    /// Address to bind to (default: 127.0.0.1)
    #[clap(long, default_value = "127.0.0.1")]
    address: String,

    /// Port to listen on (default: 8080)
    #[clap(long, default_value_t = 8080)]
    port: u16,

    /// Monocle data directory (default: $HOME/.monocle)
    #[clap(long)]
    data_dir: Option<String>,

    /// Maximum concurrent operations per connection (0 = unlimited)
    #[clap(long)]
    max_concurrent_ops: Option<usize>,

    /// Maximum websocket message size in bytes
    #[clap(long)]
    max_message_size: Option<usize>,

    /// Idle timeout in seconds
    #[clap(long)]
    connection_timeout_secs: Option<u64>,

    /// Ping interval in seconds
    #[clap(long)]
    ping_interval_secs: Option<u64>,
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

        Commands::Server(args) => {
            // The server requires the `server` feature (axum + tokio). Keep the CLI
            // binary as the entrypoint, but compile this arm only when `server` is enabled.
            #[cfg(feature = "cli")]
            {
                let data_dir = args.data_dir.unwrap_or_else(|| config.data_dir.clone());

                let router = monocle::server::create_router();
                let context = monocle::server::WsContext::new(data_dir);

                let mut server_config = monocle::server::ServerConfig::default()
                    .with_address(args.address)
                    .with_port(args.port);

                if let Some(v) = args.max_concurrent_ops {
                    server_config.max_concurrent_ops = v;
                }
                if let Some(v) = args.max_message_size {
                    server_config.max_message_size = v;
                }
                if let Some(v) = args.connection_timeout_secs {
                    server_config.connection_timeout_secs = v;
                }
                if let Some(v) = args.ping_interval_secs {
                    server_config.ping_interval_secs = v;
                }

                // Start server (blocks current thread until shutdown)
                let rt = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("Failed to create tokio runtime for server: {e}");
                        std::process::exit(1);
                    }
                };
                if let Err(e) = rt.block_on(monocle::server::start_server(
                    router,
                    context,
                    server_config,
                )) {
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
            commands::inspect::run(&config, args, output_format, cli.no_refresh)
        }
        Commands::Time(args) => commands::time::run(args, output_format),
        Commands::Country(args) => commands::country::run(args, output_format),
        Commands::Rpki { commands } => {
            commands::rpki::run(commands, output_format, &config.data_dir, cli.no_refresh)
        }
        Commands::Ip(args) => commands::ip::run(args, output_format),
        Commands::As2rel(args) => {
            commands::as2rel::run(&config, args, output_format, cli.no_refresh)
        }
        Commands::Config(args) => commands::config::run(&config, args, output_format),
    }
}
