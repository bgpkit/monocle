use anyhow::{anyhow, Result};
use config::Config;
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
}
