use clap::Args;
use ipnet::IpNet;
use itertools::Itertools;
use monocle::database::{MonocleDatabase, Pfx2asDbRecord};
use monocle::lens::pfx2as::Pfx2asEntry;
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
    /// IP prefixes or prefix files (one prefix per line)
    #[clap(required = true)]
    pub input: Vec<String>,

    /// Only matching exact prefixes. By default, it does longest-prefix matching.
    #[clap(short, long)]
    pub exact_match: bool,

    /// Force refresh the cache even if it's fresh
    #[clap(short, long)]
    pub refresh: bool,

    /// Show covering prefixes (supernets) instead of longest match
    #[clap(long)]
    pub covering: bool,

    /// Show covered prefixes (subnets) instead of longest match
    #[clap(long)]
    pub covered: bool,
}

pub fn run(config: &MonocleConfig, args: Pfx2asArgs, output_format: OutputFormat) {
    let Pfx2asArgs {
        input,
        exact_match,
        refresh,
        covering,
        covered,
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

    // Determine lookup mode
    let mode = if exact_match {
        LookupMode::Exact
    } else if covering {
        LookupMode::Covering
    } else if covered {
        LookupMode::Covered
    } else {
        LookupMode::Longest
    };

    // Open the database
    let db = match MonocleDatabase::open_in_dir(&config.data_dir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("ERROR: Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    let repo = db.pfx2as();

    // Check if we need to refresh the cache
    let ttl_std = config.pfx2as_cache_ttl();
    let ttl = chrono::Duration::from_std(ttl_std).unwrap_or(chrono::Duration::hours(24));
    let needs_refresh = refresh || repo.needs_refresh(ttl);

    if needs_refresh {
        let url = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";
        eprintln!("Loading pfx2as data from {}...", url);

        // Load data from source
        let entries: Vec<Pfx2asEntry> = match oneio::read_json_struct(url) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("ERROR: unable to load data file: {}", e);
                std::process::exit(1);
            }
        };

        eprintln!("Storing {} entries in database...", entries.len());

        // Convert to database records
        let records: Vec<Pfx2asDbRecord> = entries
            .into_iter()
            .map(|e| Pfx2asDbRecord {
                prefix: e.prefix,
                origin_asn: e.asn,
            })
            .collect();

        // Store in SQLite
        if let Err(e) = repo.store(&records, url) {
            eprintln!("ERROR: Failed to store data: {}", e);
            std::process::exit(1);
        }

        eprintln!("Database updated");
    }

    // Check if database has data
    if repo.is_empty() {
        eprintln!("ERROR: No pfx2as data in database. Run with --refresh to populate.");
        std::process::exit(1);
    }

    // Perform lookups based on mode
    match mode {
        LookupMode::Exact => {
            let mut prefix_origins_map: HashMap<IpNet, HashSet<u32>> = HashMap::new();
            for p in &prefixes {
                let prefix_str = p.to_string();
                match repo.lookup_exact(&prefix_str) {
                    Ok(asns) => {
                        prefix_origins_map.entry(*p).or_default().extend(asns);
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to lookup {}: {}", p, e);
                    }
                }
            }
            display_simple_results(&prefix_origins_map, output_format);
        }
        LookupMode::Longest => {
            let mut prefix_origins_map: HashMap<IpNet, HashSet<u32>> = HashMap::new();
            for p in &prefixes {
                let prefix_str = p.to_string();
                match repo.lookup_longest(&prefix_str) {
                    Ok(result) => {
                        prefix_origins_map
                            .entry(*p)
                            .or_default()
                            .extend(result.origin_asns);
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to lookup {}: {}", p, e);
                    }
                }
            }
            display_simple_results(&prefix_origins_map, output_format);
        }
        LookupMode::Covering => {
            let mut results: Vec<CoveringResult> = Vec::new();
            for p in &prefixes {
                let prefix_str = p.to_string();
                match repo.lookup_covering(&prefix_str) {
                    Ok(covering_results) => {
                        for r in covering_results {
                            results.push(CoveringResult {
                                query: p.to_string(),
                                prefix: r.prefix,
                                origins: r.origin_asns,
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to lookup {}: {}", p, e);
                    }
                }
            }
            display_covering_results(&results, output_format);
        }
        LookupMode::Covered => {
            let mut results: Vec<CoveringResult> = Vec::new();
            for p in &prefixes {
                let prefix_str = p.to_string();
                match repo.lookup_covered(&prefix_str) {
                    Ok(covered_results) => {
                        for r in covered_results {
                            results.push(CoveringResult {
                                query: p.to_string(),
                                prefix: r.prefix,
                                origins: r.origin_asns,
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to lookup {}: {}", p, e);
                    }
                }
            }
            display_covering_results(&results, output_format);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum LookupMode {
    Exact,
    Longest,
    Covering,
    Covered,
}

#[derive(Debug, Clone, Serialize, Deserialize, tabled::Tabled)]
struct Pfx2asResult {
    prefix: String,
    origins: String,
}

#[derive(Debug, Clone)]
struct CoveringResult {
    query: String,
    prefix: String,
    origins: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, tabled::Tabled)]
struct CoveringResultDisplay {
    query: String,
    prefix: String,
    origins: String,
}

/// Display simple results (exact/longest match)
fn display_simple_results(
    prefix_origins_map: &HashMap<IpNet, HashSet<u32>>,
    output_format: OutputFormat,
) {
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

/// Display covering/covered results
fn display_covering_results(results: &[CoveringResult], output_format: OutputFormat) {
    let sorted_results: Vec<_> = results
        .iter()
        .sorted_by(|a, b| a.query.cmp(&b.query).then(a.prefix.cmp(&b.prefix)))
        .collect();

    match output_format {
        OutputFormat::Table => {
            let display: Vec<CoveringResultDisplay> = sorted_results
                .iter()
                .map(|r| CoveringResultDisplay {
                    query: r.query.clone(),
                    prefix: r.prefix.clone(),
                    origins: r.origins.iter().sorted().join(","),
                })
                .collect();
            println!("{}", Table::new(display).with(Style::rounded()));
        }
        OutputFormat::Markdown => {
            let display: Vec<CoveringResultDisplay> = sorted_results
                .iter()
                .map(|r| CoveringResultDisplay {
                    query: r.query.clone(),
                    prefix: r.prefix.clone(),
                    origins: r.origins.iter().sorted().join(","),
                })
                .collect();
            println!("{}", Table::new(display).with(Style::markdown()));
        }
        OutputFormat::Json => {
            let data: Vec<Value> = sorted_results
                .iter()
                .map(|r| {
                    json!({
                        "query": r.query,
                        "prefix": r.prefix,
                        "origins": r.origins.iter().sorted().collect::<Vec<_>>()
                    })
                })
                .collect();
            match serde_json::to_string(&data) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonPretty => {
            let data: Vec<Value> = sorted_results
                .iter()
                .map(|r| {
                    json!({
                        "query": r.query,
                        "prefix": r.prefix,
                        "origins": r.origins.iter().sorted().collect::<Vec<_>>()
                    })
                })
                .collect();
            match serde_json::to_string_pretty(&data) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonLine => {
            for r in &sorted_results {
                let obj = json!({
                    "query": r.query,
                    "prefix": r.prefix,
                    "origins": r.origins.iter().sorted().collect::<Vec<_>>()
                });
                match serde_json::to_string(&obj) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("query|prefix|origins");
            for r in &sorted_results {
                println!(
                    "{}|{}|{}",
                    r.query,
                    r.prefix,
                    r.origins.iter().sorted().join(",")
                );
            }
        }
    }
}
