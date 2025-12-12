use clap::Args;
use ipnet::IpNet;
use itertools::Itertools;
use monocle::database::{Pfx2asFileCache, Pfx2asRecord};
use monocle::lens::pfx2as::Pfx2asLens;
use monocle::lens::utils::OutputFormat;
use monocle::MonocleConfig;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tabled::settings::Style;
use tabled::Table;

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

    /// Force refresh the cache even if it's fresh
    #[clap(short, long)]
    pub refresh: bool,

    /// Skip cache and use direct in-memory lookup (legacy behavior)
    #[clap(long)]
    pub no_cache: bool,
}

pub fn run(config: &MonocleConfig, args: Pfx2asArgs, output_format: OutputFormat) {
    let Pfx2asArgs {
        data_file_path,
        input,
        exact_match,
        refresh,
        no_cache,
    } = args;

    // Collect all prefixes to look up
    let mut prefixes: Vec<IpNet> = vec![];
    for i in &input {
        match i.parse::<IpNet>() {
            Ok(p) => prefixes.push(p),
            Err(_) => {
                // It might be a data file
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

    if prefixes.is_empty() {
        eprintln!("ERROR: No valid prefixes provided");
        std::process::exit(1);
    }

    prefixes.sort();

    // If no_cache is set, use the legacy in-memory approach
    if no_cache {
        run_legacy(&data_file_path, &prefixes, exact_match, output_format);
        return;
    }

    // Use file-based cache
    run_with_cache(
        config,
        &data_file_path,
        &prefixes,
        exact_match,
        refresh,
        output_format,
    );
}

/// Run with file-based caching and in-memory trie
fn run_with_cache(
    config: &MonocleConfig,
    data_source: &str,
    prefixes: &[IpNet],
    exact_match: bool,
    refresh: bool,
    output_format: OutputFormat,
) {
    let data_dir = config.data_dir.as_str();

    // Initialize the file cache
    let cache = match Pfx2asFileCache::new(data_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to initialize cache: {}", e);
            // Fall back to legacy mode
            run_legacy(data_source, prefixes, exact_match, output_format);
            return;
        }
    };

    // Check if we need to refresh the cache
    let ttl = config.pfx2as_cache_ttl();
    let needs_refresh = refresh || !cache.is_fresh(data_source, ttl);

    if needs_refresh {
        eprintln!("Loading pfx2as data from {}...", data_source);

        // Load data from source
        let entries: Vec<monocle::lens::pfx2as::Pfx2asEntry> =
            match oneio::read_json_struct(data_source) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("ERROR: unable to load data file: {}", e);
                    std::process::exit(1);
                }
            };

        eprintln!("Caching {} entries...", entries.len());

        // Convert to cache records (aggregate by prefix)
        let mut prefix_map: HashMap<String, HashSet<u32>> = HashMap::new();
        for entry in entries {
            prefix_map
                .entry(entry.prefix.clone())
                .or_default()
                .insert(entry.asn);
        }

        let records: Vec<Pfx2asRecord> = prefix_map
            .into_iter()
            .map(|(prefix, asns)| Pfx2asRecord {
                prefix,
                origin_asns: asns.into_iter().collect(),
            })
            .collect();

        // Store in cache
        if let Err(e) = cache.store(data_source, records) {
            eprintln!("Warning: Failed to cache data: {}", e);
            // Fall back to legacy mode
            run_legacy(data_source, prefixes, exact_match, output_format);
            return;
        }

        eprintln!("Cache updated");
    }

    // Load cached data and perform lookups using in-memory trie
    let cached_data = match cache.load(data_source) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("Warning: Failed to load cached data: {}", e);
            // Fall back to legacy mode
            run_legacy(data_source, prefixes, exact_match, output_format);
            return;
        }
    };

    // Build a Pfx2asLens from cached data
    let pfx2as = match Pfx2asLens::from_records(cached_data.records) {
        Ok(lens) => lens,
        Err(e) => {
            eprintln!("Warning: Failed to build lookup trie: {}", e);
            // Fall back to legacy mode
            run_legacy(data_source, prefixes, exact_match, output_format);
            return;
        }
    };

    // Perform lookups
    let mut prefix_origins_map: HashMap<IpNet, HashSet<u32>> = HashMap::new();
    for p in prefixes {
        let origins = if exact_match {
            pfx2as.lookup_exact(*p)
        } else {
            pfx2as.lookup_longest(*p)
        };
        prefix_origins_map.entry(*p).or_default().extend(origins);
    }

    display_results(&prefix_origins_map, output_format);
}

/// Run with legacy in-memory trie (no caching)
fn run_legacy(
    data_source: &str,
    prefixes: &[IpNet],
    exact_match: bool,
    output_format: OutputFormat,
) {
    let pfx2as = match Pfx2asLens::new(Some(data_source.to_string())) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: unable to open data file: {}", e);
            std::process::exit(1);
        }
    };

    let mut prefix_origins_map: HashMap<IpNet, HashSet<u32>> = HashMap::new();
    for p in prefixes {
        let origins = if exact_match {
            pfx2as.lookup_exact(*p)
        } else {
            pfx2as.lookup_longest(*p)
        };
        prefix_origins_map.entry(*p).or_default().extend(origins);
    }

    display_results(&prefix_origins_map, output_format);
}

#[derive(Debug, Clone, Serialize, Deserialize, tabled::Tabled)]
struct Pfx2asResult {
    prefix: String,
    origins: String,
}

/// Display results in the appropriate format
fn display_results(prefix_origins_map: &HashMap<IpNet, HashSet<u32>>, output_format: OutputFormat) {
    let sorted_results: Vec<_> = prefix_origins_map
        .iter()
        .sorted_by_key(|(p, _)| *p)
        .map(|(p, o)| {
            let mut origins_vec: Vec<u32> = o.iter().cloned().collect();
            origins_vec.sort();
            (p.to_string(), origins_vec)
        })
        .collect();

    match output_format {
        OutputFormat::Table => {
            let display: Vec<Pfx2asResult> = sorted_results
                .iter()
                .map(|(p, o)| Pfx2asResult {
                    prefix: p.clone(),
                    origins: o.iter().join(","),
                })
                .collect();
            println!("{}", Table::new(display).with(Style::rounded()));
        }
        OutputFormat::Markdown => {
            let display: Vec<Pfx2asResult> = sorted_results
                .iter()
                .map(|(p, o)| Pfx2asResult {
                    prefix: p.clone(),
                    origins: o.iter().join(","),
                })
                .collect();
            println!("{}", Table::new(display).with(Style::markdown()));
        }
        OutputFormat::Json => {
            let data: Vec<Value> = sorted_results
                .iter()
                .map(|(p, o)| json!({"prefix": p, "origins": o}))
                .collect();
            match serde_json::to_string(&data) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonPretty => {
            let data: Vec<Value> = sorted_results
                .iter()
                .map(|(p, o)| json!({"prefix": p, "origins": o}))
                .collect();
            match serde_json::to_string_pretty(&data) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonLine => {
            for (p, o) in &sorted_results {
                let obj = json!({"prefix": p, "origins": o});
                match serde_json::to_string(&obj) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("prefix|origins");
            for (p, o) in &sorted_results {
                println!("{}|{}", p, o.iter().join(","));
            }
        }
    }
}
