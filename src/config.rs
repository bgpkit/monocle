use std::collections::HashMap;
use std::path::Path;
use config::Config;

pub struct MonocleConfig {
    pub config: HashMap<String, String>,
}

const EMPTY_CONFIG: &str = "# monocle configuration file\n";

impl MonocleConfig {
    pub fn load(path: &Option<String>) -> MonocleConfig {
        let mut builder = Config::builder();

        // Add in toml configuration file
        match path {
            Some(p) => {
                if Path::new(p.as_str()).exists(){
                    builder = builder.add_source(config::File::with_name(p.as_str()));
                } else {
                    std::fs::write(p.as_str(), EMPTY_CONFIG).expect("Unable to create config file");
                }
            }
            None => {
                // by default use $HOME/.monocle.toml as the configuration file path
                let p = format!("{}/.monocle.toml", dirs::home_dir().unwrap().to_str().unwrap());
                if Path::new(p.as_str()).exists(){
                    builder = builder.add_source(config::File::with_name(p.as_str()));
                } else {
                    std::fs::write(p.as_str(), EMPTY_CONFIG).expect("Unable to create config file");
                }
            }
        }
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `MONOCLE_DEBUG=1 ./target/app` would set the `debug` key
        builder = builder.add_source(config::Environment::with_prefix("MONOCLE"));

        let settings = builder.build() .unwrap();
        let config = settings.try_deserialize::<HashMap<String, String>>()
            .unwrap();
        MonocleConfig{ config }
    }
}