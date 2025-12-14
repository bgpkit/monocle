use clap::Args;
use monocle::config::{
    format_size, get_cache_info, get_cache_settings, get_sqlite_info, CacheInfo, CacheSettings,
    SqliteDatabaseInfo,
};
use monocle::lens::utils::OutputFormat;
use monocle::MonocleConfig;
use serde::Serialize;

/// Arguments for the Config command
#[derive(Args)]
pub struct ConfigArgs {
    /// Show detailed information about all data files
    #[clap(short, long)]
    pub verbose: bool,
}

#[derive(Debug, Serialize)]
struct ConfigInfo {
    config_file: String,
    data_dir: String,
    database: SqliteDatabaseInfo,
    cache: CacheInfo,
    cache_settings: CacheSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileInfo>>,
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
    let ConfigArgs { verbose } = args;

    // Get config file path
    let config_file = MonocleConfig::config_file_path();

    // Get database and cache info using shared functions
    let database_info = get_sqlite_info(config);
    let cache_info = get_cache_info(config);
    let cache_settings = get_cache_settings(config);

    // Collect file info if verbose
    let files = if verbose {
        collect_file_info(config, &cache_info)
    } else {
        None
    };

    let config_info = ConfigInfo {
        config_file,
        data_dir: config.data_dir.clone(),
        database: database_info,
        cache: cache_info,
        cache_settings,
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
        _ => {
            // Table, Markdown, and PSV all use the same human-readable format
            print_config_table(&config_info, verbose);
        }
    }
}

fn collect_file_info(config: &MonocleConfig, cache_info: &CacheInfo) -> Option<Vec<FileInfo>> {
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

    // List cache directory files
    if cache_info.exists {
        let cache_dir = &cache_info.directory;
        for subdir in &["rpki", "pfx2as"] {
            let subdir_path = format!("{}/{}", cache_dir, subdir);
            if let Ok(entries) = std::fs::read_dir(&subdir_path) {
                for entry in entries.flatten() {
                    if let Ok(metadata) = entry.metadata() {
                        if metadata.is_file() {
                            let modified = metadata.modified().ok().map(|t| {
                                let datetime: chrono::DateTime<chrono::Utc> = t.into();
                                datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
                            });

                            file_list.push(FileInfo {
                                name: format!(
                                    "cache/{}/{}",
                                    subdir,
                                    entry.file_name().to_string_lossy()
                                ),
                                path: entry.path().to_string_lossy().to_string(),
                                size_bytes: metadata.len(),
                                modified,
                            });
                        }
                    }
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

    println!("SQLite Database:");
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
    if let Some(count) = info.database.as2org_count {
        println!("  AS2Org:         {} records", count);
    }
    if let Some(count) = info.database.as2rel_count {
        if let Some(ref updated) = info.database.as2rel_last_updated {
            println!("  AS2Rel:         {} records (updated: {})", count, updated);
        } else {
            println!("  AS2Rel:         {} records", count);
        }
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
    println!();

    println!("File Cache:");
    println!("  Directory:      {}", info.cache.directory);
    println!(
        "  Status:         {}",
        if info.cache.exists {
            "exists"
        } else {
            "not created"
        }
    );
    if let Some(size) = info.cache.size_bytes {
        println!("  Size:           {}", format_size(size));
    }
    if let Some(count) = info.cache.pfx2as_cache_count {
        println!("  Pfx2as files:   {}", count);
    }
    println!();

    println!("Cache Settings:");
    println!(
        "  RPKI TTL:       {} seconds ({} hours)",
        info.cache_settings.rpki_ttl_secs,
        info.cache_settings.rpki_ttl_secs / 3600
    );
    println!(
        "  Pfx2as TTL:     {} seconds ({} hours)",
        info.cache_settings.pfx2as_ttl_secs,
        info.cache_settings.pfx2as_ttl_secs / 3600
    );

    if verbose {
        if let Some(ref files) = info.files {
            println!();
            println!("Data Directory Files:");
            println!("  {:<40} {:>12}  {}", "Name", "Size", "Modified");
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
    eprintln!("  Use 'monocle database' for database management commands");
}
