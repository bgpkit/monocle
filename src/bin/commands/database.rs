use clap::{Args, Subcommand};
use monocle::config::{
    format_size, get_cache_info, get_cache_settings, get_data_source_info, get_sqlite_info,
    CacheInfo, CacheSettings, DataSource, DataSourceInfo, DataSourceStatus, SqliteDatabaseInfo,
};
use monocle::database::{MonocleDatabase, Pfx2asDbRecord};
use monocle::lens::pfx2as::Pfx2asEntry;
use monocle::lens::rpki::commons::load_current_rpki;
use monocle::lens::utils::OutputFormat;
use monocle::MonocleConfig;
use serde::Serialize;
use std::path::Path;
use std::time::Instant;

/// Arguments for the Database command
#[derive(Args)]
pub struct DatabaseArgs {
    #[clap(subcommand)]
    pub command: Option<DatabaseCommands>,
}

/// Database subcommands
#[derive(Subcommand)]
pub enum DatabaseCommands {
    /// Refresh data source(s)
    Refresh {
        /// Name of the data source to refresh: as2org, as2rel, rpki, pfx2as
        #[clap(value_name = "DB_NAME")]
        source: Option<String>,

        /// Refresh all data sources
        #[clap(long, short)]
        all: bool,
    },

    /// Backup the database to a destination
    Backup {
        /// Destination path for the backup
        #[clap(value_name = "DEST")]
        destination: String,

        /// Include cache files in the backup
        #[clap(long)]
        include_cache: bool,
    },

    /// Show database status (default when no subcommand)
    Status,

    /// Clear data source(s)
    Clear {
        /// Name of the data source to clear: as2org, as2rel, rpki, pfx2as, all
        #[clap(value_name = "DB_NAME")]
        source: String,

        /// Skip confirmation prompt
        #[clap(long, short = 'y')]
        yes: bool,
    },

    /// List available data sources
    Sources,
}

#[derive(Debug, Serialize)]
struct DatabaseStatus {
    sqlite: SqliteDatabaseInfo,
    cache: CacheInfo,
    settings: CacheSettings,
    sources: Vec<DataSourceInfo>,
}

pub fn run(config: &MonocleConfig, args: DatabaseArgs, output_format: OutputFormat) {
    match args.command {
        None | Some(DatabaseCommands::Status) => run_status(config, output_format),
        Some(DatabaseCommands::Refresh { source, all }) => {
            run_refresh(config, source, all, output_format)
        }
        Some(DatabaseCommands::Backup {
            destination,
            include_cache,
        }) => run_backup(config, &destination, include_cache, output_format),
        Some(DatabaseCommands::Clear { source, yes }) => {
            run_clear(config, &source, yes, output_format)
        }
        Some(DatabaseCommands::Sources) => run_sources(config, output_format),
    }
}

fn run_status(config: &MonocleConfig, output_format: OutputFormat) {
    let sqlite_info = get_sqlite_info(config);
    let cache_info = get_cache_info(config);
    let cache_settings = get_cache_settings(config);
    let sources = get_data_source_info(config);

    let status = DatabaseStatus {
        sqlite: sqlite_info.clone(),
        cache: cache_info.clone(),
        settings: cache_settings.clone(),
        sources: sources.clone(),
    };

    match output_format {
        OutputFormat::Json => match serde_json::to_string(&status) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing status: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(&status) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing status: {}", e),
        },
        OutputFormat::JsonLine => match serde_json::to_string(&status) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing status: {}", e),
        },
        _ => print_status_table(&sqlite_info, &cache_info, &cache_settings, &sources),
    }
}

fn print_status_table(
    sqlite: &SqliteDatabaseInfo,
    cache: &CacheInfo,
    settings: &CacheSettings,
    sources: &[DataSourceInfo],
) {
    println!("Monocle Database Status");
    println!("=======================\n");

    println!("SQLite Database:");
    println!("  Path:           {}", sqlite.path);
    println!(
        "  Status:         {}",
        if sqlite.exists {
            "exists"
        } else {
            "not created"
        }
    );
    if let Some(size) = sqlite.size_bytes {
        println!("  Size:           {}", format_size(size));
    }
    println!(
        "  Schema:         {}",
        if sqlite.schema_initialized {
            format!("initialized (v{})", sqlite.schema_version.unwrap_or(0))
        } else {
            "not initialized".to_string()
        }
    );

    println!();
    println!("Data Sources:");

    for source in sources {
        let status_str = match source.status {
            DataSourceStatus::Ready => {
                let mut parts = Vec::new();
                if let Some(count) = source.record_count {
                    // Special handling for RPKI to show ROA/ASPA breakdown
                    if source.name == "rpki" {
                        if let (Some(roa), Some(aspa)) =
                            (sqlite.rpki_roa_count, sqlite.rpki_aspa_count)
                        {
                            parts.push(format!("{} ROAs, {} ASPAs", roa, aspa));
                        } else {
                            parts.push(format!("{} records", count));
                        }
                    } else {
                        parts.push(format!("{} records", count));
                    }
                }
                if let Some(ref updated) = source.last_updated {
                    parts.push(format!("updated: {}", updated));
                }
                if parts.is_empty() {
                    "ready".to_string()
                } else {
                    parts.join(", ")
                }
            }
            DataSourceStatus::Empty => {
                format!("empty (run: monocle database refresh {})", source.name)
            }
            DataSourceStatus::NotInitialized => "not initialized".to_string(),
        };
        println!("  {:15} {}", format!("{}:", source.name), status_str);
    }

    println!();
    println!("File Cache:");
    println!("  Directory:      {}", cache.directory);
    println!(
        "  Status:         {}",
        if cache.exists {
            "exists"
        } else {
            "not created"
        }
    );
    if let Some(size) = cache.size_bytes {
        println!("  Total Size:     {}", format_size(size));
    }

    println!();
    println!("Cache Settings:");
    println!(
        "  RPKI TTL:       {} seconds ({} hours)",
        settings.rpki_ttl_secs,
        settings.rpki_ttl_secs / 3600
    );
    println!(
        "  Pfx2as TTL:     {} seconds ({} hours)",
        settings.pfx2as_ttl_secs,
        settings.pfx2as_ttl_secs / 3600
    );

    eprintln!();
    eprintln!("Commands:");
    eprintln!("  monocle database refresh <source>  Refresh a data source");
    eprintln!("  monocle database refresh --all     Refresh all data sources");
    eprintln!("  monocle database backup <dest>     Backup database to destination");
    eprintln!("  monocle database clear <source>    Clear a data source");
    eprintln!("  monocle database sources           List available data sources");
}

fn run_refresh(
    config: &MonocleConfig,
    source: Option<String>,
    all: bool,
    output_format: OutputFormat,
) {
    if all {
        // Refresh all data sources
        refresh_all_sources(config, output_format);
    } else if let Some(source_name) = source {
        // Refresh specific source
        match DataSource::from_str(&source_name) {
            Some(ds) => refresh_source(config, ds, output_format),
            None => {
                eprintln!(
                    "ERROR: Unknown data source '{}'. Available sources: as2org, as2rel, rpki, pfx2as-cache",
                    source_name
                );
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("ERROR: Please specify a data source to refresh or use --all");
        eprintln!();
        eprintln!("Usage:");
        eprintln!("  monocle database refresh <source>  Refresh a specific data source");
        eprintln!("  monocle database refresh --all     Refresh all data sources");
        eprintln!();
        eprintln!("Available sources: as2org, as2rel, rpki, pfx2as-cache");
        std::process::exit(1);
    }
}

/// Result of a refresh operation with timing
struct RefreshResult {
    source: &'static str,
    result: Result<String, String>,
    duration_secs: f64,
}

fn refresh_all_sources(config: &MonocleConfig, output_format: OutputFormat) {
    eprintln!("Refreshing all data sources...\n");

    let total_start = Instant::now();
    let mut results: Vec<RefreshResult> = Vec::new();

    for source in DataSource::all() {
        eprintln!("Refreshing {}...", source.name());
        let start = Instant::now();
        let result = do_refresh(config, source);
        let duration_secs = start.elapsed().as_secs_f64();
        results.push(RefreshResult {
            source: source.name(),
            result,
            duration_secs,
        });
    }

    let total_duration = total_start.elapsed().as_secs_f64();

    // Output results
    if output_format.is_json() {
        let json_results: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "source": r.source,
                    "success": r.result.is_ok(),
                    "message": match &r.result {
                        Ok(msg) => msg.clone(),
                        Err(msg) => msg.clone(),
                    },
                    "duration_secs": r.duration_secs
                })
            })
            .collect();

        let output = serde_json::json!({
            "results": json_results,
            "total_duration_secs": total_duration
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
        println!("\nRefresh Results:");
        println!("{}", "-".repeat(70));
        for r in &results {
            match &r.result {
                Ok(msg) => println!("  ✓ {}: {} ({:.2}s)", r.source, msg, r.duration_secs),
                Err(msg) => println!("  ✗ {}: {} ({:.2}s)", r.source, msg, r.duration_secs),
            }
        }
        println!("{}", "-".repeat(70));
        println!("  Total time: {:.2}s", total_duration);
    }
}

fn refresh_source(config: &MonocleConfig, source: DataSource, output_format: OutputFormat) {
    eprintln!("Refreshing {}...", source.name());

    let start = Instant::now();
    let result = do_refresh(config, source);
    let duration_secs = start.elapsed().as_secs_f64();

    if output_format.is_json() {
        let json_result = serde_json::json!({
            "source": source.name(),
            "success": result.is_ok(),
            "message": match &result {
                Ok(msg) => msg.clone(),
                Err(msg) => msg.clone(),
            },
            "duration_secs": duration_secs
        });

        match output_format {
            OutputFormat::JsonPretty => {
                if let Ok(json) = serde_json::to_string_pretty(&json_result) {
                    println!("{}", json);
                }
            }
            _ => {
                if let Ok(json) = serde_json::to_string(&json_result) {
                    println!("{}", json);
                }
            }
        }
    } else {
        match result {
            Ok(msg) => println!("✓ {} ({:.2}s)", msg, duration_secs),
            Err(msg) => {
                eprintln!("✗ {} ({:.2}s)", msg, duration_secs);
                std::process::exit(1);
            }
        }
    }
}

fn do_refresh(config: &MonocleConfig, source: DataSource) -> Result<String, String> {
    match source {
        DataSource::As2org => {
            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            let (as_count, org_count) = db
                .bootstrap_as2org()
                .map_err(|e| format!("Failed to refresh as2org: {}", e))?;

            Ok(format!(
                "Loaded {} ASes and {} organizations",
                as_count, org_count
            ))
        }
        DataSource::As2rel => {
            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            let count = db
                .update_as2rel()
                .map_err(|e| format!("Failed to refresh as2rel: {}", e))?;

            Ok(format!("Loaded {} relationships", count))
        }
        DataSource::Rpki => {
            // Load RPKI data from Cloudflare and store in SQLite
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

            // Store in database
            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            db.rpki()
                .store(&roas, &aspas)
                .map_err(|e| format!("Failed to store RPKI data: {}", e))?;

            Ok(format!(
                "Loaded {} ROAs and {} ASPAs",
                roa_count, aspa_count
            ))
        }
        DataSource::Pfx2asCache => {
            // Fetch pfx2as data from BGPKIT and store in SQLite
            let url = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";
            eprintln!("Loading pfx2as data from {}...", url);

            let entries: Vec<Pfx2asEntry> = oneio::read_json_struct(url)
                .map_err(|e| format!("Failed to fetch pfx2as data: {}", e))?;

            let records: Vec<Pfx2asDbRecord> = entries
                .into_iter()
                .map(|e| Pfx2asDbRecord {
                    prefix: e.prefix,
                    origin_asn: e.asn,
                })
                .collect();

            let count = records.len();

            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            db.pfx2as()
                .store(&records, url)
                .map_err(|e| format!("Failed to store pfx2as data: {}", e))?;

            Ok(format!("Loaded {} pfx2as records", count))
        }
    }
}

fn run_backup(
    config: &MonocleConfig,
    destination: &str,
    include_cache: bool,
    output_format: OutputFormat,
) {
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

    let mut backed_up_files = vec![dest_file.to_string_lossy().to_string()];

    // Optionally copy cache files
    if include_cache {
        let cache_dir = config.cache_dir();
        if Path::new(&cache_dir).exists() {
            let dest_cache_dir = dest_path
                .parent()
                .unwrap_or(dest_path)
                .join("monocle-cache");

            eprintln!("Backing up cache files...");
            match copy_dir_recursive(&cache_dir, &dest_cache_dir) {
                Ok(count) => {
                    backed_up_files.push(format!(
                        "{} ({} files)",
                        dest_cache_dir.to_string_lossy(),
                        count
                    ));
                }
                Err(e) => {
                    eprintln!("Warning: Failed to backup cache files: {}", e);
                }
            }
        }
    }

    // Output result
    if output_format.is_json() {
        let result = serde_json::json!({
            "success": true,
            "files": backed_up_files,
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
        for file in &backed_up_files {
            println!("  - {}", file);
        }
    }
}

fn copy_dir_recursive(src: &str, dest: &Path) -> Result<usize, std::io::Error> {
    std::fs::create_dir_all(dest)?;

    let mut count = 0;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if path.is_dir() {
            count += copy_dir_recursive(path.to_str().unwrap_or(""), &dest_path)?;
        } else {
            std::fs::copy(&path, &dest_path)?;
            count += 1;
        }
    }

    Ok(count)
}

fn run_clear(
    config: &MonocleConfig,
    source: &str,
    skip_confirm: bool,
    output_format: OutputFormat,
) {
    let sources_to_clear: Vec<DataSource> = if source.to_lowercase() == "all" {
        DataSource::all()
    } else {
        match DataSource::from_str(source) {
            Some(ds) => vec![ds],
            None => {
                eprintln!(
                    "ERROR: Unknown data source '{}'. Available: as2org, as2rel, rpki, pfx2as, all",
                    source
                );
                std::process::exit(1);
            }
        }
    };

    // Confirmation prompt
    if !skip_confirm && !output_format.is_json() {
        let source_names: Vec<&str> = sources_to_clear.iter().map(|s| s.name()).collect();
        eprintln!(
            "This will clear the following data sources: {}",
            source_names.join(", ")
        );
        eprint!("Are you sure? [y/N] ");

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() {
            let input = input.trim().to_lowercase();
            if input != "y" && input != "yes" {
                eprintln!("Aborted.");
                return;
            }
        } else {
            eprintln!("Aborted.");
            return;
        }
    }

    let mut results: Vec<(&str, Result<String, String>)> = Vec::new();

    for ds in &sources_to_clear {
        let result = do_clear(config, *ds);
        results.push((ds.name(), result));
    }

    // Output results
    if output_format.is_json() {
        let json_results: Vec<serde_json::Value> = results
            .iter()
            .map(|(name, result)| {
                serde_json::json!({
                    "source": name,
                    "success": result.is_ok(),
                    "message": match result {
                        Ok(msg) => msg.clone(),
                        Err(msg) => msg.clone(),
                    }
                })
            })
            .collect();

        match output_format {
            OutputFormat::JsonPretty => {
                if let Ok(json) = serde_json::to_string_pretty(&json_results) {
                    println!("{}", json);
                }
            }
            _ => {
                if let Ok(json) = serde_json::to_string(&json_results) {
                    println!("{}", json);
                }
            }
        }
    } else {
        for (name, result) in &results {
            match result {
                Ok(msg) => println!("✓ {}: {}", name, msg),
                Err(msg) => eprintln!("✗ {}: {}", name, msg),
            }
        }
    }
}

fn do_clear(config: &MonocleConfig, source: DataSource) -> Result<String, String> {
    match source {
        DataSource::As2org => {
            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            db.as2org()
                .clear()
                .map_err(|e| format!("Failed to clear as2org: {}", e))?;

            Ok("Cleared".to_string())
        }
        DataSource::As2rel => {
            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            db.as2rel()
                .clear()
                .map_err(|e| format!("Failed to clear as2rel: {}", e))?;

            Ok("Cleared".to_string())
        }
        DataSource::Rpki => {
            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            db.rpki()
                .clear()
                .map_err(|e| format!("Failed to clear rpki: {}", e))?;

            Ok("Cleared".to_string())
        }
        DataSource::Pfx2asCache => {
            let db = MonocleDatabase::open(&config.sqlite_path())
                .map_err(|e| format!("Failed to open database: {}", e))?;

            db.pfx2as()
                .clear()
                .map_err(|e| format!("Failed to clear pfx2as: {}", e))?;

            Ok("Cleared".to_string())
        }
    }
}

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
        println!("Available Data Sources:");
        println!();
        println!(
            "  {:<15} {:<45} {:<15} {}",
            "Name", "Description", "Status", "Last Updated"
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
        println!("  monocle database refresh <source>  Refresh a specific data source");
        println!("  monocle database refresh --all     Refresh all data sources");
        println!("  monocle database clear <source>    Clear a specific data source");
    }
}
