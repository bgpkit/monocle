use clap::Args;
use ipnet::IpNet;
use itertools::Itertools;
use monocle::lens::pfx2as::Pfx2asLens;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Arguments for the Pfx2as command
#[derive(Args)]
pub struct Pfx2asArgs {
    /// Prefix-to-AS mapping data file location
    #[clap(
        long,
        default_value = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2"
    )]
    pub data_file_path: String,

    /// IP prefixes or prefix files (one prefix per line)
    #[clap(required = true)]
    pub input: Vec<String>,

    /// Only matching exact prefixes. By default, it does longest-prefix matching.
    #[clap(short, long)]
    pub exact_match: bool,
}

pub fn run(args: Pfx2asArgs, json: bool) {
    let Pfx2asArgs {
        data_file_path,
        input,
        exact_match,
    } = args;

    let pfx2as = match Pfx2asLens::new(Some(data_file_path)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: unable to open data file: {}", e);
            std::process::exit(1);
        }
    };

    // collect all prefixes to look up
    let mut prefixes: Vec<IpNet> = vec![];
    for i in input {
        match i.parse::<IpNet>() {
            Ok(p) => prefixes.push(p),
            Err(_) => {
                // it might be a data file
                if let Ok(lines) = oneio::read_lines(i.as_str()) {
                    for line in lines.map_while(Result::ok) {
                        if line.starts_with('#') {
                            continue;
                        }
                        let trimmed = line.trim().split(',').next().unwrap_or(line.as_str());
                        if let Ok(p) = trimmed.parse::<IpNet>() {
                            prefixes.push(p);
                        }
                    }
                }
            }
        }
    }

    // map prefix to origins. one prefix may be mapped to multiple origins
    prefixes.sort();
    let mut prefix_origins_map: HashMap<IpNet, HashSet<u32>> = HashMap::new();
    for p in prefixes {
        let origins = match exact_match {
            true => pfx2as.lookup_exact(p),
            false => pfx2as.lookup_longest(p),
        };
        prefix_origins_map.entry(p).or_default().extend(origins);
    }

    // display
    if json {
        // map prefix_origin_pairs to a vector of JSON objects each with a
        // "prefix" and "origin" field
        let data = prefix_origins_map
            .iter()
            .map(|(p, o)| {
                json!({"prefix": p.to_string(), "origins": o.iter().cloned().collect::<Vec<u32>>()})
            })
            .collect::<Vec<Value>>();
        if let Err(e) = serde_json::to_writer_pretty(std::io::stdout(), &data) {
            eprintln!("Error writing JSON to stdout: {}", e);
        }
    } else {
        for (prefix, origins) in prefix_origins_map {
            let mut origins_vec = origins.iter().cloned().collect::<Vec<u32>>();
            origins_vec.sort();
            println!("{},{}", prefix, origins.iter().join(","));
        }
    }
}
