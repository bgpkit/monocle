use clap::Args;
use monocle::database::{DuckDbConn, DuckDbSchemaManager, MonocleDatabase};
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
    databases: DatabaseInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<FileInfo>>,
}

#[derive(Debug, Serialize)]
struct DatabaseInfo {
    sqlite: SqliteDbInfo,
    duckdb: DuckDbInfo,
}

#[derive(Debug, Serialize)]
struct SqliteDbInfo {
    path: String,
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    as2org_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    as2rel_count: Option<u64>,
}

#[derive(Debug, Serialize)]
struct DuckDbInfo {
    path: String,
    exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_bytes: Option<u64>,
    schema_initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_version: Option<u32>,
}

#[derive(Debug, Serialize)]
struct FileInfo {
    name: String,
    path: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified: Option<String>,
}

pub fn run(config: &MonocleConfig, args: ConfigArgs, json_output: bool) {
    let ConfigArgs { verbose } = args;

    // Determine config file path
    let home_dir = dirs::home_dir()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|| "~".to_string());
    let config_file = format!("{}/.monocle/monocle.toml", home_dir);

    // Build paths for databases
    let data_dir = &config.data_dir;
    let sqlite_path = format!("{}monocle-data.sqlite3", data_dir);
    let duckdb_path = format!("{}monocle-data.duckdb", data_dir);

    // Get SQLite database info
    let sqlite_exists = Path::new(&sqlite_path).exists();
    let sqlite_size = if sqlite_exists {
        std::fs::metadata(&sqlite_path).ok().map(|m| m.len())
    } else {
        None
    };

    let (as2org_count, as2rel_count) = if sqlite_exists {
        match MonocleDatabase::open(&sqlite_path) {
            Ok(db) => {
                let as2org = db.as2org().as_count().ok();
                let as2rel = db.as2rel().count().ok();
                (as2org, as2rel)
            }
            Err(_) => (None, None),
        }
    } else {
        (None, None)
    };

    // Get DuckDB database info
    let duckdb_exists = Path::new(&duckdb_path).exists();
    let duckdb_size = if duckdb_exists {
        std::fs::metadata(&duckdb_path).ok().map(|m| m.len())
    } else {
        None
    };

    let (schema_initialized, schema_version) = if duckdb_exists {
        match DuckDbConn::open_path(&duckdb_path) {
            Ok(conn) => {
                let manager = DuckDbSchemaManager::new(&conn);
                match manager.check_status() {
                    Ok(status) => {
                        use monocle::database::DuckDbSchemaStatus;
                        match status {
                            DuckDbSchemaStatus::Current => {
                                (true, Some(monocle::database::DUCKDB_SCHEMA_VERSION))
                            }
                            DuckDbSchemaStatus::NeedsMigration { from, to: _ } => {
                                (true, Some(from))
                            }
                            DuckDbSchemaStatus::NotInitialized => (false, None),
                            DuckDbSchemaStatus::Incompatible {
                                database_version,
                                required_version: _,
                            } => (true, Some(database_version)),
                            DuckDbSchemaStatus::Corrupted => (false, None),
                        }
                    }
                    Err(_) => (false, None),
                }
            }
            Err(_) => (false, None),
        }
    } else {
        (false, None)
    };

    // Collect file info if verbose
    let files = if verbose {
        let mut file_list = Vec::new();
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
    } else {
        None
    };

    let config_info = ConfigInfo {
        config_file,
        data_dir: data_dir.clone(),
        databases: DatabaseInfo {
            sqlite: SqliteDbInfo {
                path: sqlite_path,
                exists: sqlite_exists,
                size_bytes: sqlite_size,
                as2org_count,
                as2rel_count,
            },
            duckdb: DuckDbInfo {
                path: duckdb_path,
                exists: duckdb_exists,
                size_bytes: duckdb_size,
                schema_initialized,
                schema_version,
            },
        },
        files,
    };

    if json_output {
        match serde_json::to_string_pretty(&config_info) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("Error serializing config info: {}", e),
        }
    } else {
        print_config_table(&config_info, verbose);
    }
}

fn print_config_table(info: &ConfigInfo, verbose: bool) {
    println!("Monocle Configuration");
    println!("=====================\n");

    println!("Paths:");
    println!("  Config file:  {}", info.config_file);
    println!("  Data dir:     {}", info.data_dir);
    println!();

    println!("SQLite Database (legacy/export):");
    println!("  Path:         {}", info.databases.sqlite.path);
    println!(
        "  Status:       {}",
        if info.databases.sqlite.exists {
            "exists"
        } else {
            "not created"
        }
    );
    if let Some(size) = info.databases.sqlite.size_bytes {
        println!("  Size:         {}", format_size(size));
    }
    if let Some(count) = info.databases.sqlite.as2org_count {
        println!("  AS2Org:       {} records", count);
    }
    if let Some(count) = info.databases.sqlite.as2rel_count {
        println!("  AS2Rel:       {} records", count);
    }
    println!();

    println!("DuckDB Database (primary):");
    println!("  Path:         {}", info.databases.duckdb.path);
    println!(
        "  Status:       {}",
        if info.databases.duckdb.exists {
            "exists"
        } else {
            "not created"
        }
    );
    if let Some(size) = info.databases.duckdb.size_bytes {
        println!("  Size:         {}", format_size(size));
    }
    println!(
        "  Schema:       {}",
        if info.databases.duckdb.schema_initialized {
            format!(
                "initialized (v{})",
                info.databases.duckdb.schema_version.unwrap_or(0)
            )
        } else {
            "not initialized".to_string()
        }
    );

    if verbose {
        if let Some(ref files) = info.files {
            println!();
            println!("Data Directory Files:");
            println!("  {:<30} {:>12}  {}", "Name", "Size", "Modified");
            println!("  {}", "-".repeat(70));
            for file in files {
                println!(
                    "  {:<30} {:>12}  {}",
                    file.name,
                    format_size(file.size_bytes),
                    file.modified.as_deref().unwrap_or("-")
                );
            }
        }
    }

    println!();
    println!("Tip: Use --verbose to see all files in the data directory");
    println!("     Use --json for machine-readable output");
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
