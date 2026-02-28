use chrono_humanize::HumanTime;
use clap::{Args, Subcommand};
use monocle::config::{
    format_size, get_data_source_info, get_sqlite_info, DataSource, DataSourceStatus,
    SqliteDatabaseInfo,
};
use monocle::database::{MonocleDatabase, Pfx2asDbRecord};
use monocle::lens::rpki::RpkiLens;
use monocle::server::ServerConfig;
use monocle::utils::OutputFormat;
use monocle::MonocleConfig;
use serde::Serialize;
use std::path::Path;
use std::time::Instant;

/// Convert a timestamp string like "2024-01-15 10:30:00 UTC" to relative time like "2 hours ago"
fn to_relative_time(timestamp_str: &str) -> String {
    // Parse the timestamp string format: "YYYY-MM-DD HH:MM:SS UTC"
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(timestamp_str, "%Y-%m-%d %H:%M:%S UTC")
    {
        let dt = naive.and_utc();
        HumanTime::from(dt).to_string()
    } else {
        // If parsing fails, return the original string
        timestamp_str.to_string()
    }
}

/// Arguments for the Config command
#[derive(Args)]
pub struct ConfigArgs {
    #[clap(subcommand)]
    pub command: Option<ConfigCommands>,

    /// Show detailed information about all data files
    #[clap(short, long)]
    pub verbose: bool,
}

/// Config subcommands
#[derive(Subcommand)]
#[allow(clippy::enum_variant_names)]
pub enum ConfigCommands {
    /// Update data source(s)
    Update {
        /// Update asinfo data
        #[clap(long)]
        asinfo: bool,

        /// Update as2rel data
        #[clap(long)]
        as2rel: bool,

        /// Update RPKI data
        #[clap(long)]
        rpki: bool,

        /// Update pfx2as data
        #[clap(long)]
        pfx2as: bool,

        /// RTR endpoint for fetching ROAs (format: host:port)
        /// Overrides config file setting for this update only.
        /// Example: --rtr-endpoint rtr.rpki.cloudflare.com:8282
        #[clap(long, value_name = "HOST:PORT")]
        rtr_endpoint: Option<String>,
    },

    /// Backup the database to a destination
    Backup {
        /// Destination path for the backup
        #[clap(value_name = "DEST")]
        destination: String,
    },

    /// List available data sources and their status
    Sources,
}

#[derive(Debug, Serialize)]
struct ConfigInfo {
    config_file: String,
    data_dir: String,
    cache_dir: String,
    cache_ttl: CacheTtlConfig,
    database: SqliteDatabaseInfo,
    server_defaults: ServerDefaults,
    #[serde(skip_serializing_if = "Option::is_none")]
    rtr_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileInfo>>,
}

#[derive(Debug, Serialize)]
struct CacheTtlConfig {
    asinfo_secs: u64,
    as2rel_secs: u64,
    rpki_secs: u64,
    pfx2as_secs: u64,
}

#[derive(Debug, Serialize)]
struct ServerDefaults {
    address: String,
    port: u16,
    max_concurrent_ops: usize,
    max_message_size: usize,
    connection_timeout_secs: u64,
    ping_interval_secs: u64,
}

impl From<&ServerConfig> for ServerDefaults {
    fn from(config: &ServerConfig) -> Self {
        Self {
            address: config.address.clone(),
            port: config.port,
            max_concurrent_ops: config.max_concurrent_ops,
            max_message_size: config.max_message_size,
            connection_timeout_secs: config.connection_timeout_secs,
            ping_interval_secs: config.ping_interval_secs,
        }
    }
}

#[derive(Debug, Serialize)]
struct FileInfo {
    name: String,
    path: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified: Option<String>,
}

pub fn run(config: &MonocleConfig, args: ConfigArgs, output_format: OutputFormat) {
    match args.command {
        None => run_status(config, args.verbose, output_format),
        Some(ConfigCommands::Update {
            asinfo,
            as2rel,
            rpki,
            pfx2as,
            rtr_endpoint,
        }) => run_update(
            config,
            asinfo,
            as2rel,
            rpki,
            pfx2as,
            rtr_endpoint,
            output_format,
        ),
        Some(ConfigCommands::Backup { destination }) => {
            run_backup(config, &destination, output_format)
        }
        Some(ConfigCommands::Sources) => run_sources(config, output_format),
    }
}

fn run_status(config: &MonocleConfig, verbose: bool, output_format: OutputFormat) {
    // Get config file path
    let config_file = MonocleConfig::config_file_path();

    // Get database info
    let database_info = get_sqlite_info(config);
    let server_defaults = ServerDefaults::from(&ServerConfig::default());

    // Collect file info if verbose
    let files = if verbose {
        collect_file_info(config)
    } else {
        None
    };

    let config_info = ConfigInfo {
        config_file,
        data_dir: config.data_dir.clone(),
        cache_dir: config.cache_dir(),
        cache_ttl: CacheTtlConfig {
            asinfo_secs: config.asinfo_cache_ttl_secs,
            as2rel_secs: config.as2rel_cache_ttl_secs,
            rpki_secs: config.rpki_cache_ttl_secs,
            pfx2as_secs: config.pfx2as_cache_ttl_secs,
        },
        database: database_info,
        server_defaults,
        rtr_endpoint: config.rtr_endpoint().map(|(h, p)| format!("{}:{}", h, p)),
        files,
    };

    match output_format {
        OutputFormat::Json => match serde_json::to_string(&config_info) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing config info: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(&config_info) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing config info: {}", e),
        },
        OutputFormat::JsonLine => match serde_json::to_string(&config_info) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing config info: {}", e),
        },
        OutputFormat::Table | OutputFormat::Markdown | OutputFormat::Psv => {
            // All non-JSON formats use the same human-readable format
            print_config_table(&config_info, verbose);
        }
    }
}

fn collect_file_info(config: &MonocleConfig) -> Option<Vec<FileInfo>> {
    let mut file_list = Vec::new();
    let data_dir = &config.data_dir;

    // List data directory files
    if let Ok(entries) = std::fs::read_dir(data_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    let modified = metadata.modified().ok().map(|t| {
                        let datetime: chrono::DateTime<chrono::Utc> = t.into();
                        datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
                    });

                    file_list.push(FileInfo {
                        name: entry.file_name().to_string_lossy().to_string(),
                        path: entry.path().to_string_lossy().to_string(),
                        size_bytes: metadata.len(),
                        modified,
                    });
                }
            }
        }
    }

    file_list.sort_by(|a, b| a.name.cmp(&b.name));
    Some(file_list)
}

fn print_config_table(info: &ConfigInfo, verbose: bool) {
    println!("Monocle Configuration");
    println!("=====================\n");

    println!("General:");
    println!("  Config file:    {}", info.config_file);
    println!("  Data dir:       {}", info.data_dir);
    println!("  Cache dir:      {}", info.cache_dir);
    println!();

    println!("Cache TTL:");
    println!(
        "  ASInfo:         {}",
        format_duration(info.cache_ttl.asinfo_secs)
    );
    println!(
        "  AS2Rel:         {}",
        format_duration(info.cache_ttl.as2rel_secs)
    );
    println!(
        "  RPKI:           {}",
        format_duration(info.cache_ttl.rpki_secs)
    );
    println!(
        "  Pfx2as:         {}",
        format_duration(info.cache_ttl.pfx2as_secs)
    );
    if let Some(ref endpoint) = info.rtr_endpoint {
        println!("  RTR endpoint:   {}", endpoint);
    }
    println!();

    println!("Database:");
    println!("  Path:           {}", info.database.path);
    println!(
        "  Status:         {}",
        if info.database.exists {
            "exists"
        } else {
            "not created"
        }
    );
    if let Some(size) = info.database.size_bytes {
        println!("  Size:           {}", format_size(size));
    }
    println!(
        "  Schema:         {}",
        if info.database.schema_initialized {
            format!(
                "initialized (v{})",
                info.database.schema_version.unwrap_or(0)
            )
        } else {
            "not initialized".to_string()
        }
    );

    // ASInfo
    if let Some(count) = info.database.asinfo_count {
        if count > 0 {
            if let Some(ref updated) = info.database.asinfo_last_updated {
                println!("  ASInfo:         {} records (updated: {})", count, updated);
            } else {
                println!("  ASInfo:         {} records", count);
            }
        } else {
            println!("  ASInfo:         empty");
        }
    } else {
        println!("  ASInfo:         not initialized");
    }

    // AS2Rel
    if let Some(count) = info.database.as2rel_count {
        if count > 0 {
            if let Some(ref updated) = info.database.as2rel_last_updated {
                println!("  AS2Rel:         {} records (updated: {})", count, updated);
            } else {
                println!("  AS2Rel:         {} records", count);
            }
        } else {
            println!("  AS2Rel:         empty");
        }
    } else {
        println!("  AS2Rel:         not initialized");
    }

    // RPKI ROAs and ASPAs
    match (info.database.rpki_roa_count, info.database.rpki_aspa_count) {
        (Some(roa), Some(aspa)) if roa > 0 || aspa > 0 => {
            if let Some(ref updated) = info.database.rpki_last_updated {
                println!(
                    "  RPKI:           {} ROAs, {} ASPAs (updated: {})",
                    roa, aspa, updated
                );
            } else {
                println!("  RPKI:           {} ROAs, {} ASPAs", roa, aspa);
            }
        }
        (Some(0), Some(0)) => {
            println!("  RPKI:           empty");
        }
        _ => {
            println!("  RPKI:           not initialized");
        }
    }

    // Pfx2as
    if let Some(count) = info.database.pfx2as_count {
        if count > 0 {
            if let Some(ref updated) = info.database.pfx2as_last_updated {
                println!("  Pfx2as:         {} records (updated: {})", count, updated);
            } else {
                println!("  Pfx2as:         {} records", count);
            }
        } else {
            println!("  Pfx2as:         empty");
        }
    } else {
        println!("  Pfx2as:         not initialized");
    }
    println!();

    println!("Server Defaults:");
    println!(
        "  Address:        {}:{}",
        info.server_defaults.address, info.server_defaults.port
    );
    println!(
        "  Max concurrent: {} operations",
        info.server_defaults.max_concurrent_ops
    );
    println!(
        "  Max message:    {} bytes",
        info.server_defaults.max_message_size
    );
    println!(
        "  Timeout:        {} seconds",
        info.server_defaults.connection_timeout_secs
    );
    println!(
        "  Ping interval:  {} seconds",
        info.server_defaults.ping_interval_secs
    );

    if verbose {
        if let Some(ref files) = info.files {
            println!();
            println!("Data Directory Files:");
            println!("  {:<40} {:>12}  Modified", "Name", "Size");
            println!("  {}", "-".repeat(80));
            for file in files {
                println!(
                    "  {:<40} {:>12}  {}",
                    file.name,
                    format_size(file.size_bytes),
                    file.modified.as_deref().unwrap_or("-")
                );
            }
        }
    }

    eprintln!();
    eprintln!("Tips:");
    eprintln!("  Use --verbose (-v) to see all files in the data directory");
    eprintln!("  Use --format json for machine-readable output");
    eprintln!(
        "  Edit {} to customize settings",
        MonocleConfig::config_file_path()
    );
    eprintln!("  Use 'monocle config sources' to see data source status");
    eprintln!("  Use 'monocle config update' to update data sources");
}

// =============================================================================
// update subcommand
// =============================================================================

fn run_update(
    config: &MonocleConfig,
    asinfo: bool,
    as2rel: bool,
    rpki: bool,
    pfx2as: bool,
    rtr_endpoint: Option<String>,
    output_format: OutputFormat,
) {
    // If no specific flags are set, update all
    let update_all = !asinfo && !as2rel && !rpki && !pfx2as;

    let sources_to_update: Vec<DataSource> = if update_all {
        DataSource::all()
    } else {
        let mut sources = Vec::new();
        if asinfo {
            sources.push(DataSource::Asinfo);
        }
        if as2rel {
            sources.push(DataSource::As2rel);
        }
        if rpki {
            sources.push(DataSource::Rpki);
        }
        if pfx2as {
            sources.push(DataSource::Pfx2as);
        }
        sources
    };

    update_sources(
        config,
        &sources_to_update,
        rtr_endpoint.as_deref(),
        output_format,
    );
}

#[derive(Debug, Serialize)]
struct UpdateResult {
    source: String,
    result: String,
    duration_secs: f64,
}

fn update_sources(
    config: &MonocleConfig,
    sources: &[DataSource],
    rtr_endpoint: Option<&str>,
    output_format: OutputFormat,
) {
    let sqlite_path = config.sqlite_path();

    // Open database
    let db = match MonocleDatabase::open(&sqlite_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("ERROR: Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    let mut results = Vec::new();

    for source in sources {
        eprintln!("[monocle] Updating {}...", source.name());
        let start = Instant::now();

        let result = do_update(&db, source, config, rtr_endpoint);
        let duration = start.elapsed().as_secs_f64();

        let result_str = match &result {
            Ok(msg) => {
                eprintln!("[monocle]   ✓ {} ({:.2}s)", msg, duration);
                msg.clone()
            }
            Err(e) => {
                eprintln!("[monocle]   ✗ Failed: {} ({:.2}s)", e, duration);
                format!("Failed: {}", e)
            }
        };

        results.push(UpdateResult {
            source: source.name().to_string(),
            result: result_str,
            duration_secs: duration,
        });
    }

    if output_format.is_json() {
        let output = serde_json::json!({
            "results": results,
        });

        match output_format {
            OutputFormat::JsonPretty => {
                if let Ok(json) = serde_json::to_string_pretty(&output) {
                    println!("{}", json);
                }
            }
            _ => {
                if let Ok(json) = serde_json::to_string(&output) {
                    println!("{}", json);
                }
            }
        }
    } else {
        eprintln!();
        eprintln!("[monocle] Update completed.");
    }
}

fn do_update(
    db: &MonocleDatabase,
    source: &DataSource,
    config: &MonocleConfig,
    rtr_endpoint: Option<&str>,
) -> Result<String, String> {
    match source {
        DataSource::Asinfo => {
            // Use the database's bootstrap_asinfo method with the passed db connection
            let counts = db
                .refresh_asinfo()
                .map_err(|e| format!("Failed to refresh asinfo: {}", e))?;

            Ok(format!(
                "Stored {} core, {} as2org, {} peeringdb, {} hegemony, {} population records",
                counts.core, counts.as2org, counts.peeringdb, counts.hegemony, counts.population
            ))
        }
        DataSource::As2rel => {
            // Use the database's update_as2rel method with the passed db connection
            let count = db
                .refresh_as2rel()
                .map_err(|e| format!("Failed to refresh as2rel: {}", e))?;

            Ok(format!("Stored {} relationship entries", count))
        }
        DataSource::Rpki => {
            // Determine RTR endpoint: CLI override > config > none
            let effective_rtr_endpoint = if rtr_endpoint.is_some() {
                rtr_endpoint.map(|s| s.to_string())
            } else {
                config
                    .rtr_endpoint()
                    .map(|(host, port)| format!("{}:{}", host, port))
            };

            if let Some(ref endpoint) = effective_rtr_endpoint {
                eprintln!(
                    "[monocle]   Using RTR endpoint: {} (connection timeout: {}s)",
                    endpoint, config.rpki_rtr_timeout_secs
                );
            }

            let lens = RpkiLens::new(db);
            let result = lens
                .refresh_with_rtr(
                    effective_rtr_endpoint.as_deref(),
                    config.rtr_timeout(),
                    config.rpki_rtr_no_fallback,
                )
                .map_err(|e| format!("Failed to refresh RPKI data: {}", e))?;

            // Display warning if there was a fallback
            if let Some(ref warning) = result.warning {
                eprintln!("[monocle]   WARNING: {}", warning);
            }

            Ok(format!(
                "Stored {} ROAs (from {}), {} ASPAs (from Cloudflare)",
                result.roa_count, result.roa_source, result.aspa_count
            ))
        }
        DataSource::Pfx2as => {
            use ipnet::IpNet;

            use std::str::FromStr;

            // Fetch pfx2as data from BGPKIT
            let url = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";

            let entries: Vec<monocle::lens::pfx2as::Pfx2asEntry> = oneio::read_json_struct(url)
                .map_err(|e| format!("Failed to fetch pfx2as data: {}", e))?;

            // Filter out invalid /0 prefixes
            let entries: Vec<_> = entries
                .into_iter()
                .filter(|e| !e.prefix.ends_with("/0"))
                .collect();

            let entry_count = entries.len();

            // Load RPKI data for validation using bgpkit-commons directly
            let trie = monocle::lens::rpki::commons::load_current_rpki().ok();

            let records: Vec<Pfx2asDbRecord> = entries
                .into_iter()
                .map(|e| {
                    let validation = if let Some(trie) = &trie {
                        if let Ok(prefix) = IpNet::from_str(&e.prefix) {
                            let roas = trie.lookup_by_prefix(&prefix);
                            if roas.is_empty() {
                                "unknown".to_string()
                            } else {
                                let prefix_len = prefix.prefix_len();
                                let is_valid = roas
                                    .iter()
                                    .any(|roa| roa.asn == e.asn && prefix_len <= roa.max_length);
                                if is_valid {
                                    "valid".to_string()
                                } else {
                                    "invalid".to_string()
                                }
                            }
                        } else {
                            "unknown".to_string()
                        }
                    } else {
                        "unknown".to_string()
                    };

                    Pfx2asDbRecord {
                        prefix: e.prefix,
                        origin_asn: e.asn,
                        validation,
                    }
                })
                .collect();

            db.pfx2as()
                .store(&records, url)
                .map_err(|e| format!("Failed to store pfx2as data: {}", e))?;

            let stats = db
                .pfx2as()
                .validation_stats()
                .map_err(|e| format!("Failed to get validation stats: {}", e))?;

            Ok(format!(
                "Stored {} pfx2as records (valid: {}, invalid: {}, unknown: {})",
                entry_count, stats.valid, stats.invalid, stats.unknown
            ))
        }
    }
}

// =============================================================================
// backup subcommand
// =============================================================================

fn run_backup(config: &MonocleConfig, destination: &str, output_format: OutputFormat) {
    let sqlite_path = config.sqlite_path();
    let dest_path = Path::new(destination);

    // Determine if destination is a directory or file
    let dest_file = if dest_path.is_dir() || destination.ends_with('/') {
        // Create directory if needed
        if let Err(e) = std::fs::create_dir_all(dest_path) {
            eprintln!("ERROR: Failed to create destination directory: {}", e);
            std::process::exit(1);
        }
        dest_path.join("monocle-data.sqlite3")
    } else {
        // Ensure parent directory exists
        if let Some(parent) = dest_path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("ERROR: Failed to create destination directory: {}", e);
                    std::process::exit(1);
                }
            }
        }
        dest_path.to_path_buf()
    };

    // Check if source exists
    if !Path::new(&sqlite_path).exists() {
        eprintln!("ERROR: Database file does not exist: {}", sqlite_path);
        std::process::exit(1);
    }

    // Copy the database file
    eprintln!("Backing up database...");
    if let Err(e) = std::fs::copy(&sqlite_path, &dest_file) {
        eprintln!("ERROR: Failed to backup database: {}", e);
        std::process::exit(1);
    }

    let backed_up_file = dest_file.to_string_lossy().to_string();

    // Output result
    if output_format.is_json() {
        let result = serde_json::json!({
            "success": true,
            "file": backed_up_file,
        });

        match output_format {
            OutputFormat::JsonPretty => {
                if let Ok(json) = serde_json::to_string_pretty(&result) {
                    println!("{}", json);
                }
            }
            _ => {
                if let Ok(json) = serde_json::to_string(&result) {
                    println!("{}", json);
                }
            }
        }
    } else {
        println!("✓ Backup completed successfully");
        println!("  - {}", backed_up_file);
    }
}

// =============================================================================
// sources subcommand
// =============================================================================

fn run_sources(config: &MonocleConfig, output_format: OutputFormat) {
    let sources = get_data_source_info(config);

    if output_format.is_json() {
        match output_format {
            OutputFormat::JsonPretty => {
                if let Ok(json) = serde_json::to_string_pretty(&sources) {
                    println!("{}", json);
                }
            }
            _ => {
                if let Ok(json) = serde_json::to_string(&sources) {
                    println!("{}", json);
                }
            }
        }
    } else {
        println!("Data Sources:");
        println!();
        println!(
            "  {:<12} {:<15} {:<10} Last Updated",
            "Name", "Status", "Stale"
        );
        println!("  {}", "-".repeat(60));

        for source in &sources {
            let status_str = match source.status {
                DataSourceStatus::Ready => {
                    if let Some(count) = source.record_count {
                        format!("{} records", count)
                    } else {
                        "ready".to_string()
                    }
                }
                DataSourceStatus::Empty => "empty".to_string(),
                DataSourceStatus::NotInitialized => "not initialized".to_string(),
            };

            let updated_str = source
                .last_updated
                .as_deref()
                .map(to_relative_time)
                .unwrap_or_else(|| "-".to_string());

            let stale_str = if source.is_stale { "yes" } else { "no" };

            println!(
                "  {:<12} {:<15} {:<10} {}",
                source.name, status_str, stale_str, updated_str
            );
        }

        // Configuration section
        println!();
        println!("Configuration:");
        println!(
            "  ASInfo cache TTL: {}",
            format_duration(config.asinfo_cache_ttl_secs)
        );
        println!(
            "  AS2Rel cache TTL: {}",
            format_duration(config.as2rel_cache_ttl_secs)
        );
        println!(
            "  RPKI cache TTL:   {}",
            format_duration(config.rpki_cache_ttl_secs)
        );
        println!(
            "  Pfx2as cache TTL: {}",
            format_duration(config.pfx2as_cache_ttl_secs)
        );
        if let Some((host, port)) = config.rtr_endpoint() {
            println!("  RTR endpoint:     {}:{}", host, port);
        }

        println!();
        println!("Usage:");
        println!("  monocle config update              Update all data sources");
        println!("  monocle config update --rpki       Update only RPKI data");
        println!("  monocle config update --asinfo     Update only ASInfo data");
        println!("  monocle config backup <path>       Backup database to path");
    }
}

/// Format duration in seconds to human-readable string
fn format_duration(secs: u64) -> String {
    if secs >= 86400 {
        let days = secs / 86400;
        if days == 1 {
            "1 day".to_string()
        } else {
            format!("{} days", days)
        }
    } else if secs >= 3600 {
        let hours = secs / 3600;
        if hours == 1 {
            "1 hour".to_string()
        } else {
            format!("{} hours", hours)
        }
    } else if secs >= 60 {
        let mins = secs / 60;
        if mins == 1 {
            "1 minute".to_string()
        } else {
            format!("{} minutes", mins)
        }
    } else {
        format!("{} seconds", secs)
    }
}
