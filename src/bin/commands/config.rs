use clap::Args;
use monocle::database::{cache_size, MonocleDatabase, SchemaManager, SchemaStatus, SCHEMA_VERSION};
use monocle::lens::utils::OutputFormat;
use monocle::MonocleConfig;
use serde::Serialize;
use std::path::Path;

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
    database: DatabaseInfo,
    cache: CacheInfo,
    cache_settings: CacheSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileInfo>>,
}

#[derive(Debug, Serialize)]
struct DatabaseInfo {
    path: String,
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    schema_initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    as2org_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    as2rel_count: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CacheInfo {
    directory: String,
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CacheSettings {
    rpki_ttl_secs: u64,
    pfx2as_ttl_secs: u64,
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

    // Determine config file path
    let home_dir = dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|| "~".to_string());
    let config_file = format!("{}/.monocle/monocle.toml", home_dir);

    // Build paths
    let data_dir = &config.data_dir;
    let sqlite_path = config.sqlite_path();
    let cache_dir = format!("{}/cache", data_dir.trim_end_matches('/'));

    // Get SQLite database info
    let sqlite_exists = Path::new(&sqlite_path).exists();
    let sqlite_size = if sqlite_exists {
        std::fs::metadata(&sqlite_path).ok().map(|m| m.len())
    } else {
        None
    };

    let (schema_initialized, schema_version, as2org_count, as2rel_count) = if sqlite_exists {
        match MonocleDatabase::open(&sqlite_path) {
            Ok(db) => {
                let conn = db.connection();
                let manager = SchemaManager::new(conn);
                let (initialized, version) = match manager.check_status() {
                    Ok(status) => match status {
                        SchemaStatus::Current => (true, Some(SCHEMA_VERSION)),
                        SchemaStatus::NeedsMigration { from, to: _ } => (true, Some(from)),
                        SchemaStatus::NotInitialized => (false, None),
                        SchemaStatus::Incompatible {
                            database_version,
                            required_version: _,
                        } => (true, Some(database_version)),
                        SchemaStatus::Corrupted => (false, None),
                    },
                    Err(_) => (false, None),
                };

                // Get record counts if schema is initialized
                let (as2org, as2rel) = if initialized {
                    let as2org = db.as2org().as_count().ok();
                    let as2rel = db.as2rel().count().ok();
                    (as2org, as2rel)
                } else {
                    (None, None)
                };

                (initialized, version, as2org, as2rel)
            }
            Err(_) => (false, None, None, None),
        }
    } else {
        (false, None, None, None)
    };

    // Get cache info
    let cache_exists = Path::new(&cache_dir).exists();
    let cache_size_bytes = if cache_exists {
        cache_size(data_dir).ok()
    } else {
        None
    };

    // Collect file info if verbose
    let files = if verbose {
        let mut file_list = Vec::new();

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
        if cache_exists {
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
    } else {
        None
    };

    let config_info = ConfigInfo {
        config_file,
        data_dir: data_dir.clone(),
        database: DatabaseInfo {
            path: sqlite_path,
            exists: sqlite_exists,
            size_bytes: sqlite_size,
            schema_initialized,
            schema_version,
            as2org_count,
            as2rel_count,
        },
        cache: CacheInfo {
            directory: cache_dir,
            exists: cache_exists,
            size_bytes: cache_size_bytes,
        },
        cache_settings: CacheSettings {
            rpki_ttl_secs: config.rpki_cache_ttl_secs,
            pfx2as_ttl_secs: config.pfx2as_cache_ttl_secs,
        },
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
        println!("  AS2Rel:         {} records", count);
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
    eprintln!("  Use --output json for machine-readable output");
    eprintln!("  Edit ~/.monocle/monocle.toml to customize settings");
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
