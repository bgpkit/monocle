use std::collections::HashMap;
use std::path::Path;
use config::Config;

pub struct MonocleConfig {
    /// path to the directory to hold Monocle's data
    pub data_dir: String,
}

const EMPTY_CONFIG: &str = "# monocle configuration file\n";

impl MonocleConfig {
    /// function to create and initialize a new configuration
    pub fn new(path: &Option<String>) -> MonocleConfig {
        let mut builder = Config::builder();
        // by default use $HOME/.monocle.toml as the configuration file path
        let home_dir = dirs::home_dir().unwrap().to_str().unwrap().to_owned();
        // config dir
        let monocle_dir = format!("{}/.monocle", home_dir.as_str());


        // Add in toml configuration file
        match path {
            Some(p) => {
                let path = Path::new(p.as_str());
                if path.exists(){
                    builder = builder.add_source(config::File::with_name(path.to_str().unwrap()));
                } else {
                    std::fs::write(p.as_str(), EMPTY_CONFIG).expect("Unable to create config file");
                }
            }
            None => {
                std::fs::create_dir_all(monocle_dir.as_str()).unwrap();
                let p = format!("{}/monocle.toml", monocle_dir.as_str());
                if Path::new(p.as_str()).exists(){
                    builder = builder.add_source(config::File::with_name(p.as_str()));
                } else {
                    std::fs::write(p.as_str(), EMPTY_CONFIG).expect(format!("Unable to create config file {}", p.as_str()).as_str());
                }
            }
        }
        // Add in settings from the environment (with a prefix of APP)
        // Eg.. `MONOCLE_DEBUG=1 ./target/app` would set the `debug` key
        builder = builder.add_source(config::Environment::with_prefix("MONOCLE"));

        let settings = builder.build() .unwrap();
        let config = settings.try_deserialize::<HashMap<String, String>>()
            .unwrap();

        // check data directory config
        let data_dir = match config.get("data_dir") {
            Some(p) => {
                let path = Path::new(p);
                path.to_str().unwrap().to_string()
            },
            None => {
                let dir = format!("{}/.monocle/", dirs::home_dir().unwrap().to_str().unwrap());
                std::fs::create_dir_all(dir.as_str()).unwrap();
                dir
            }
        };

        MonocleConfig{ data_dir }
    }
}