use anyhow::{anyhow, Result};
use config::Config;
use std::collections::HashMap;
use std::path::Path;

pub struct MonocleConfig {
    /// path to the directory to hold Monocle's data
    pub data_dir: String,
}

const EMPTY_CONFIG: &str = r#"### monocle configuration file

### directory for cached data used by monocle
# data_dir="~/.monocle"
"#;

impl MonocleConfig {
    /// function to create and initialize a new configuration
    pub fn new(path: &Option<String>) -> Result<MonocleConfig> {
        let mut builder = Config::builder();
        // by default use $HOME/.monocle.toml as the configuration file path
        let home_dir = dirs::home_dir()
            .ok_or_else(|| anyhow!("Could not find home directory"))?
            .to_str()
            .ok_or_else(|| anyhow!("Could not convert home directory path to string"))?
            .to_owned();
        // config dir
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
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `MONOCLE_DEBUG=1 ./target/app` would set the `debug` key
        builder = builder.add_source(config::Environment::with_prefix("MONOCLE"));

        let settings = builder
            .build()
            .map_err(|e| anyhow!("Failed to build configuration: {}", e))?;
        let config = settings
            .try_deserialize::<HashMap<String, String>>()
            .map_err(|e| anyhow!("Failed to deserialize configuration: {}", e))?;

        // check data directory config
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

        Ok(MonocleConfig { data_dir })
    }
}
