use clap::{Args, Subcommand};
use monocle::config::{
    format_size, get_data_source_info, get_sqlite_info, DataSource, DataSourceStatus,
    SqliteDatabaseInfo,
};
use monocle::database::{MonocleDatabase, Pfx2asDbRecord};
use monocle::lens::rpki::commons::load_current_rpki;
use monocle::lens::utils::OutputFormat;
use monocle::server::ServerConfig;
use monocle::MonocleConfig;
use serde::Serialize;
use std::path::Path;
use std::time::Instant;

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
    /// Refresh data source(s)
    DbRefresh {
        /// Refresh asinfo data
        #[clap(long)]
        asinfo: bool,

        /// Refresh as2rel data
        #[clap(long)]
        as2rel: bool,

        /// Refresh RPKI data
        #[clap(long)]
        rpki: bool,

        /// Refresh pfx2as data
        #[clap(long)]
        pfx2as: bool,
    },

    /// Backup the database to a destination
    DbBackup {
        /// Destination path for the backup
        #[clap(value_name = "DEST")]
        destination: String,
    },

    /// List available data sources and their status
    DbSources,
}

#[derive(Debug, Serialize)]
struct ConfigInfo {
    config_file: String,
    data_dir: String,
    database: SqliteDatabaseInfo,
    server_defaults: ServerDefaults,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileInfo>>,
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
        Some(ConfigCommands::DbRefresh {
            asinfo,
            as2rel,
            rpki,
            pfx2as,
        }) => run_db_refresh(config, asinfo, as2rel, rpki, pfx2as, output_format),
        Some(ConfigCommands::DbBackup { destination }) => {
            run_db_backup(config, &destination, output_format)
        }
        Some(ConfigCommands::DbSources) => run_db_sources(config, output_format),
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
        database: database_info,
        server_defaults,
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
    eprintln!("  Edit ~/.monocle/monocle.toml to customize settings");
    eprintln!("  Use 'monocle config db-sources' to see data source status");
    eprintln!("  Use 'monocle config db-refresh' to refresh data sources");
}

// =============================================================================
// db-refresh subcommand
// =============================================================================

fn run_db_refresh(
    config: &MonocleConfig,
    asinfo: bool,
    as2rel: bool,
    rpki: bool,
    pfx2as: bool,
    output_format: OutputFormat,
) {
    // If no specific flags are set, refresh all
    let refresh_all = !asinfo && !as2rel && !rpki && !pfx2as;

    let sources_to_refresh: Vec<DataSource> = if refresh_all {
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

    refresh_sources(config, &sources_to_refresh, output_format);
}

#[derive(Debug, Serialize)]
struct RefreshResult {
    source: String,
    result: String,
    duration_secs: f64,
}

fn refresh_sources(config: &MonocleConfig, sources: &[DataSource], output_format: OutputFormat) {
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
        eprintln!("[monocle] Refreshing {}...", source.name());
        let start = Instant::now();

        let result = do_refresh(&db, source, config);
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

        results.push(RefreshResult {
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
        eprintln!("[monocle] Refresh completed.");
    }
}

fn do_refresh(
    db: &MonocleDatabase,
    source: &DataSource,
    _config: &MonocleConfig,
) -> Result<String, String> {
    match source {
        DataSource::Asinfo => {
            // Use the database's bootstrap_asinfo method with the passed db connection
            let counts = db
                .bootstrap_asinfo()
                .map_err(|e| format!("Failed to refresh asinfo: {}", e))?;

            Ok(format!(
                "Stored {} core, {} as2org, {} peeringdb, {} hegemony, {} population records",
                counts.core, counts.as2org, counts.peeringdb, counts.hegemony, counts.population
            ))
        }
        DataSource::As2rel => {
            // Use the database's update_as2rel method with the passed db connection
            let count = db
                .update_as2rel()
                .map_err(|e| format!("Failed to refresh as2rel: {}", e))?;

            Ok(format!("Stored {} relationship entries", count))
        }
        DataSource::Rpki => {
            // Refresh from Cloudflare RPKI endpoint
            let trie = load_current_rpki()
                .map_err(|e| format!("Failed to load RPKI data from Cloudflare: {}", e))?;

            // Convert to database format
            let roas: Vec<monocle::database::RpkiRoaRecord> = trie
                .trie
                .iter()
                .flat_map(|(prefix, roas)| {
                    roas.iter()
                        .map(move |roa| monocle::database::RpkiRoaRecord {
                            prefix: prefix.to_string(),
                            max_length: roa.max_length,
                            origin_asn: roa.asn,
                            ta: roa.rir.map(|r| format!("{:?}", r)).unwrap_or_default(),
                        })
                })
                .collect();

            let aspas: Vec<monocle::database::RpkiAspaRecord> = trie
                .aspas
                .iter()
                .map(|aspa| monocle::database::RpkiAspaRecord {
                    customer_asn: aspa.customer_asn,
                    provider_asns: aspa.providers.clone(),
                })
                .collect();

            let roa_count = roas.len();
            let aspa_count = aspas.len();

            db.rpki()
                .store(&roas, &aspas)
                .map_err(|e| format!("Failed to store RPKI data: {}", e))?;

            Ok(format!(
                "Stored {} ROAs and {} ASPAs",
                roa_count, aspa_count
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
// db-backup subcommand
// =============================================================================

fn run_db_backup(config: &MonocleConfig, destination: &str, output_format: OutputFormat) {
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
// db-sources subcommand
// =============================================================================

fn run_db_sources(config: &MonocleConfig, output_format: OutputFormat) {
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
            "  {:<15} {:<45} {:<15} Last Updated",
            "Name", "Description", "Status"
        );
        println!("  {}", "-".repeat(95));

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

            let updated_str = source.last_updated.as_deref().unwrap_or("-");

            println!(
                "  {:<15} {:<45} {:<15} {}",
                source.name, source.description, status_str, updated_str
            );
        }

        println!();
        println!("Usage:");
        println!("  monocle config db-refresh              Refresh all data sources");
        println!("  monocle config db-refresh --rpki       Refresh only RPKI data");
        println!("  monocle config db-refresh --asinfo     Refresh only ASInfo data");
        println!("  monocle config db-backup <path>        Backup database to path");
    }
}
