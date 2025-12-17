use anyhow::{anyhow, Result};
use config::Config;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

pub struct MonocleConfig {
    /// Path to the directory to hold Monocle's data
    pub data_dir: String,

    /// TTL for RPKI cache in seconds (default: 1 hour)
    pub rpki_cache_ttl_secs: u64,

    /// TTL for Pfx2as cache in seconds (default: 24 hours)
    pub pfx2as_cache_ttl_secs: u64,
}

const EMPTY_CONFIG: &str = r#"### monocle configuration file

### directory for cached data used by monocle
# data_dir = "~/.monocle"

### cache TTL settings (in seconds)
# rpki_cache_ttl_secs = 3600        # 1 hour
# pfx2as_cache_ttl_secs = 86400     # 24 hours
"#;

impl Default for MonocleConfig {
    fn default() -> Self {
        let home_dir = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());

        Self {
            data_dir: format!("{}/.monocle", home_dir),
            rpki_cache_ttl_secs: 3600,    // 1 hour
            pfx2as_cache_ttl_secs: 86400, // 24 hours
        }
    }
}

impl MonocleConfig {
    /// Function to create and initialize a new configuration
    pub fn new(path: &Option<String>) -> Result<MonocleConfig> {
        let mut builder = Config::builder();

        // By default use $HOME/.monocle.toml as the configuration file path
        let home_dir = dirs::home_dir()
            .ok_or_else(|| anyhow!("Could not find home directory"))?
            .to_str()
            .ok_or_else(|| anyhow!("Could not convert home directory path to string"))?
            .to_owned();

        // Config dir
        let monocle_dir = format!("{}/.monocle", home_dir.as_str());

        // Add in toml configuration file
        match path {
            Some(p) => {
                let path = Path::new(p.as_str());
                if path.exists() {
                    let path_str = path
                        .to_str()
                        .ok_or_else(|| anyhow!("Could not convert path to string"))?;
                    builder = builder.add_source(config::File::with_name(path_str));
                } else {
                    std::fs::write(p.as_str(), EMPTY_CONFIG)
                        .map_err(|e| anyhow!("Unable to create config file: {}", e))?;
                }
            }
            None => {
                std::fs::create_dir_all(monocle_dir.as_str())
                    .map_err(|e| anyhow!("Unable to create monocle directory: {}", e))?;
                let p = format!("{}/monocle.toml", monocle_dir.as_str());
                if Path::new(p.as_str()).exists() {
                    builder = builder.add_source(config::File::with_name(p.as_str()));
                } else {
                    std::fs::write(p.as_str(), EMPTY_CONFIG).map_err(|e| {
                        anyhow!("Unable to create config file {}: {}", p.as_str(), e)
                    })?;
                }
            }
        }

        // Add in settings from the environment (with a prefix of MONOCLE)
        // E.g., `MONOCLE_DATA_DIR=~/.monocle ./monocle` would set the data directory
        builder = builder.add_source(config::Environment::with_prefix("MONOCLE"));

        let settings = builder
            .build()
            .map_err(|e| anyhow!("Failed to build configuration: {}", e))?;

        let config = settings
            .try_deserialize::<HashMap<String, String>>()
            .map_err(|e| anyhow!("Failed to deserialize configuration: {}", e))?;

        // Parse data directory
        let data_dir = match config.get("data_dir") {
            Some(p) => {
                let path = Path::new(p);
                path.to_str()
                    .ok_or_else(|| anyhow!("Could not convert data_dir path to string"))?
                    .to_string()
            }
            None => {
                let home =
                    dirs::home_dir().ok_or_else(|| anyhow!("Could not find home directory"))?;
                let home_str = home
                    .to_str()
                    .ok_or_else(|| anyhow!("Could not convert home directory path to string"))?;
                let dir = format!("{}/.monocle/", home_str);
                std::fs::create_dir_all(dir.as_str())
                    .map_err(|e| anyhow!("Unable to create data directory: {}", e))?;
                dir
            }
        };

        // Parse RPKI cache TTL (default: 1 hour)
        let rpki_cache_ttl_secs = config
            .get("rpki_cache_ttl_secs")
            .and_then(|s| s.parse().ok())
            .unwrap_or(3600);

        // Parse Pfx2as cache TTL (default: 24 hours)
        let pfx2as_cache_ttl_secs = config
            .get("pfx2as_cache_ttl_secs")
            .and_then(|s| s.parse().ok())
            .unwrap_or(86400);

        Ok(MonocleConfig {
            data_dir,
            rpki_cache_ttl_secs,
            pfx2as_cache_ttl_secs,
        })
    }

    /// Get the path to the SQLite database file
    pub fn sqlite_path(&self) -> String {
        let data_dir = self.data_dir.trim_end_matches('/');
        format!("{}/monocle-data.sqlite3", data_dir)
    }

    /// Get RPKI cache TTL as Duration
    pub fn rpki_cache_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.rpki_cache_ttl_secs)
    }

    /// Get Pfx2as cache TTL as Duration
    pub fn pfx2as_cache_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.pfx2as_cache_ttl_secs)
    }

    /// Display configuration summary
    pub fn summary(&self) -> String {
        let mut lines = vec![
            format!("Data Directory:     {}", self.data_dir),
            format!("SQLite Path:        {}", self.sqlite_path()),
            format!("RPKI Cache TTL:     {} seconds", self.rpki_cache_ttl_secs),
            format!("Pfx2as Cache TTL:   {} seconds", self.pfx2as_cache_ttl_secs),
        ];

        // Check if cache directories exist and show status
        let cache_dir = format!("{}/cache", self.data_dir.trim_end_matches('/'));
        if std::path::Path::new(&cache_dir).exists() {
            lines.push(format!("Cache Directory:    {}", cache_dir));
        }

        lines.join("\n")
    }

    /// Get the config file path
    pub fn config_file_path() -> String {
        let home_dir = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|| "~".to_string());
        format!("{}/.monocle/monocle.toml", home_dir)
    }

    /// Get the cache directory path
    pub fn cache_dir(&self) -> String {
        format!("{}/cache", self.data_dir.trim_end_matches('/'))
    }
}

// =============================================================================
// Shared Database Info Types (used by both config and database commands)
// =============================================================================

/// Information about an individual data source
#[derive(Debug, Serialize, Clone)]
pub struct DataSourceInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    pub status: DataSourceStatus,
}

/// Status of a data source
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DataSourceStatus {
    /// Data is loaded and available
    Ready,
    /// Data source is empty, needs refresh
    Empty,
    /// Data source is not initialized
    NotInitialized,
}

impl std::fmt::Display for DataSourceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataSourceStatus::Ready => write!(f, "ready"),
            DataSourceStatus::Empty => write!(f, "empty"),
            DataSourceStatus::NotInitialized => write!(f, "not initialized"),
        }
    }
}

/// Information about the SQLite database
#[derive(Debug, Serialize, Clone)]
pub struct SqliteDatabaseInfo {
    pub path: String,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub schema_initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asinfo_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asinfo_last_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as2rel_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as2rel_last_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpki_roa_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpki_aspa_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpki_last_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pfx2as_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pfx2as_last_updated: Option<String>,
}

/// Information about the file-based cache
#[derive(Debug, Serialize, Clone)]
pub struct CacheInfo {
    pub directory: String,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// Cache settings
#[derive(Debug, Serialize, Clone)]
pub struct CacheSettings {
    pub rpki_ttl_secs: u64,
    pub pfx2as_ttl_secs: u64,
}

/// Available data sources that can be refreshed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataSource {
    Asinfo,
    As2rel,
    Rpki,
    Pfx2as,
}

impl DataSource {
    pub fn all() -> Vec<DataSource> {
        vec![
            DataSource::Asinfo,
            DataSource::As2rel,
            DataSource::Rpki,
            DataSource::Pfx2as,
        ]
    }

    /// Get database sources only (excluding caches)
    pub fn database_sources() -> Vec<DataSource> {
        vec![DataSource::Asinfo, DataSource::As2rel, DataSource::Rpki]
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<DataSource> {
        match s.to_lowercase().as_str() {
            "asinfo" => Some(DataSource::Asinfo),
            "as2rel" => Some(DataSource::As2rel),
            "rpki" => Some(DataSource::Rpki),
            "pfx2as" => Some(DataSource::Pfx2as),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            DataSource::Asinfo => "asinfo",
            DataSource::As2rel => "as2rel",
            DataSource::Rpki => "rpki",
            DataSource::Pfx2as => "pfx2as",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            DataSource::Asinfo => "AS information data (from BGPKIT)",
            DataSource::As2rel => "AS-level relationship data (from BGPKIT)",
            DataSource::Rpki => "RPKI ROAs and ASPAs (from Cloudflare)",
            DataSource::Pfx2as => "Prefix-to-AS mappings (from BGPKIT)",
        }
    }
}

impl std::fmt::Display for DataSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Get SQLite database information
pub fn get_sqlite_info(config: &MonocleConfig) -> SqliteDatabaseInfo {
    use crate::database::{MonocleDatabase, SchemaManager, SchemaStatus, SCHEMA_VERSION};

    let sqlite_path = config.sqlite_path();
    let sqlite_exists = Path::new(&sqlite_path).exists();
    let sqlite_size = if sqlite_exists {
        std::fs::metadata(&sqlite_path).ok().map(|m| m.len())
    } else {
        None
    };

    let (
        schema_initialized,
        schema_version,
        asinfo_count,
        asinfo_last_updated,
        as2rel_count,
        as2rel_last_updated,
        rpki_roa_count,
        rpki_aspa_count,
        rpki_last_updated,
        pfx2as_count,
        pfx2as_last_updated,
    ) = if sqlite_exists {
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

                // Get record counts and metadata if schema is initialized
                let (
                    asinfo,
                    asinfo_updated,
                    as2rel,
                    as2rel_updated,
                    rpki_roa,
                    rpki_aspa,
                    rpki_updated,
                    pfx2as,
                    pfx2as_updated,
                ) = if initialized {
                    // Get ASInfo counts
                    let asinfo = Some(db.asinfo().core_count() as u64);
                    let asinfo_meta = db.asinfo().get_metadata().ok().flatten();
                    let asinfo_updated = asinfo_meta.map(|m| {
                        let datetime =
                            chrono::DateTime::from_timestamp(m.last_updated, 0).unwrap_or_default();
                        datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
                    });

                    let as2rel = db.as2rel().count().ok();
                    let as2rel_meta = db.as2rel().get_meta().ok().flatten();
                    let as2rel_updated = as2rel_meta.map(|m| {
                        let datetime = chrono::DateTime::from_timestamp(m.last_updated as i64, 0)
                            .unwrap_or_default();
                        datetime.format("%Y-%m-%d %H:%M:%S UTC").to_string()
                    });

                    // Get RPKI counts
                    let rpki_roa = db.rpki().roa_count().ok();
                    let rpki_aspa = db.rpki().aspa_count().ok();
                    let rpki_meta = db.rpki().get_metadata().ok().flatten();
                    let rpki_updated =
                        rpki_meta.map(|m| m.updated_at.format("%Y-%m-%d %H:%M:%S UTC").to_string());

                    // Get Pfx2as counts
                    let pfx2as = db.pfx2as().record_count().ok();
                    let pfx2as_meta = db.pfx2as().get_metadata().ok().flatten();
                    let pfx2as_updated = pfx2as_meta
                        .map(|m| m.updated_at.format("%Y-%m-%d %H:%M:%S UTC").to_string());

                    (
                        asinfo,
                        asinfo_updated,
                        as2rel,
                        as2rel_updated,
                        rpki_roa,
                        rpki_aspa,
                        rpki_updated,
                        pfx2as,
                        pfx2as_updated,
                    )
                } else {
                    (None, None, None, None, None, None, None, None, None)
                };

                (
                    initialized,
                    version,
                    asinfo,
                    asinfo_updated,
                    as2rel,
                    as2rel_updated,
                    rpki_roa,
                    rpki_aspa,
                    rpki_updated,
                    pfx2as,
                    pfx2as_updated,
                )
            }
            Err(_) => (
                false, None, None, None, None, None, None, None, None, None, None,
            ),
        }
    } else {
        (
            false, None, None, None, None, None, None, None, None, None, None,
        )
    };

    SqliteDatabaseInfo {
        path: sqlite_path,
        exists: sqlite_exists,
        size_bytes: sqlite_size,
        schema_initialized,
        schema_version,
        asinfo_count,
        asinfo_last_updated,
        as2rel_count,
        as2rel_last_updated,
        rpki_roa_count,
        rpki_aspa_count,
        rpki_last_updated,
        pfx2as_count,
        pfx2as_last_updated,
    }
}

/// Get cache information
pub fn get_cache_info(config: &MonocleConfig) -> CacheInfo {
    use crate::database::cache_size;

    let cache_dir = config.cache_dir();
    let cache_exists = Path::new(&cache_dir).exists();
    let cache_size_bytes = if cache_exists {
        cache_size(&config.data_dir).ok()
    } else {
        None
    };

    CacheInfo {
        directory: cache_dir,
        exists: cache_exists,
        size_bytes: cache_size_bytes,
    }
}

/// Get cache settings
pub fn get_cache_settings(config: &MonocleConfig) -> CacheSettings {
    CacheSettings {
        rpki_ttl_secs: config.rpki_cache_ttl_secs,
        pfx2as_ttl_secs: config.pfx2as_cache_ttl_secs,
    }
}

/// Get detailed information about all data sources
pub fn get_data_source_info(config: &MonocleConfig) -> Vec<DataSourceInfo> {
    let sqlite_info = get_sqlite_info(config);

    let mut sources = Vec::new();

    // ASInfo
    let asinfo_status = match sqlite_info.asinfo_count {
        Some(count) if count > 0 => DataSourceStatus::Ready,
        Some(_) => DataSourceStatus::Empty,
        None => DataSourceStatus::NotInitialized,
    };
    sources.push(DataSourceInfo {
        name: DataSource::Asinfo.name().to_string(),
        description: DataSource::Asinfo.description().to_string(),
        record_count: sqlite_info.asinfo_count,
        last_updated: sqlite_info.asinfo_last_updated.clone(),
        status: asinfo_status,
    });

    // AS2Rel
    let as2rel_status = match sqlite_info.as2rel_count {
        Some(count) if count > 0 => DataSourceStatus::Ready,
        Some(_) => DataSourceStatus::Empty,
        None => DataSourceStatus::NotInitialized,
    };
    sources.push(DataSourceInfo {
        name: DataSource::As2rel.name().to_string(),
        description: DataSource::As2rel.description().to_string(),
        record_count: sqlite_info.as2rel_count,
        last_updated: sqlite_info.as2rel_last_updated.clone(),
        status: as2rel_status,
    });

    // RPKI (combined ROA + ASPA count for record_count, but we'll show details separately)
    let rpki_total = match (sqlite_info.rpki_roa_count, sqlite_info.rpki_aspa_count) {
        (Some(roa), Some(aspa)) => Some(roa + aspa),
        (Some(roa), None) => Some(roa),
        (None, Some(aspa)) => Some(aspa),
        (None, None) => None,
    };
    let rpki_status = match rpki_total {
        Some(count) if count > 0 => DataSourceStatus::Ready,
        Some(_) => DataSourceStatus::Empty,
        None => DataSourceStatus::NotInitialized,
    };
    sources.push(DataSourceInfo {
        name: DataSource::Rpki.name().to_string(),
        description: DataSource::Rpki.description().to_string(),
        record_count: rpki_total,
        last_updated: sqlite_info.rpki_last_updated.clone(),
        status: rpki_status,
    });

    // Pfx2as
    let pfx2as_status = match sqlite_info.pfx2as_count {
        Some(count) if count > 0 => DataSourceStatus::Ready,
        Some(_) => DataSourceStatus::Empty,
        None => DataSourceStatus::NotInitialized,
    };
    sources.push(DataSourceInfo {
        name: DataSource::Pfx2as.name().to_string(),
        description: DataSource::Pfx2as.description().to_string(),
        record_count: sqlite_info.pfx2as_count,
        last_updated: sqlite_info.pfx2as_last_updated.clone(),
        status: pfx2as_status,
    });

    sources
}

/// Format bytes as human-readable size
pub fn format_size(bytes: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MonocleConfig::default();
        assert_eq!(config.rpki_cache_ttl_secs, 3600);
        assert_eq!(config.pfx2as_cache_ttl_secs, 86400);
    }

    #[test]
    fn test_paths() {
        let config = MonocleConfig {
            data_dir: "/test/dir".to_string(),
            rpki_cache_ttl_secs: 3600,
            pfx2as_cache_ttl_secs: 86400,
        };

        assert_eq!(config.sqlite_path(), "/test/dir/monocle-data.sqlite3");
        assert_eq!(config.cache_dir(), "/test/dir/cache");
    }

    #[test]
    fn test_ttl_durations() {
        let config = MonocleConfig {
            data_dir: "/test".to_string(),
            rpki_cache_ttl_secs: 7200,
            pfx2as_cache_ttl_secs: 3600,
        };

        assert_eq!(
            config.rpki_cache_ttl(),
            std::time::Duration::from_secs(7200)
        );
        assert_eq!(
            config.pfx2as_cache_ttl(),
            std::time::Duration::from_secs(3600)
        );
    }

    #[test]
    fn test_data_source_from_str() {
        assert_eq!(DataSource::from_str("asinfo"), Some(DataSource::Asinfo));
        assert_eq!(DataSource::from_str("AS2REL"), Some(DataSource::As2rel));
        assert_eq!(DataSource::from_str("rpki"), Some(DataSource::Rpki));
        assert_eq!(DataSource::from_str("pfx2as"), Some(DataSource::Pfx2as));
        assert_eq!(DataSource::from_str("unknown"), None);
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(500), "500 B");
        assert_eq!(format_size(1024), "1.00 KB");
        assert_eq!(format_size(1536), "1.50 KB");
        assert_eq!(format_size(1048576), "1.00 MB");
        assert_eq!(format_size(1073741824), "1.00 GB");
    }

    #[test]
    fn test_data_source_status_display() {
        assert_eq!(format!("{}", DataSourceStatus::Ready), "ready");
        assert_eq!(format!("{}", DataSourceStatus::Empty), "empty");
        assert_eq!(
            format!("{}", DataSourceStatus::NotInitialized),
            "not initialized"
        );
    }
}
