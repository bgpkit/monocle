use std::collections::HashMap;
use config::Config;

pub struct MonocleConfig {
    pub config: HashMap<String, String>,
}

impl MonocleConfig {
    pub fn load(path: &Option<String>) -> MonocleConfig {
        let mut builder = Config::builder();

        // Add in toml configuration file
        match path {
            Some(p) => {
                builder = builder.add_source(config::File::with_name(p.as_str()));
            }
            None => {
                // by default use $HOME/.monocle.toml as the configuration file path
                let home = format!("{}/.monocle.toml", dirs::home_dir().unwrap().to_str().unwrap());
                builder = builder.add_source(config::File::with_name(home.as_str()));
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