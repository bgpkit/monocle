use clap::Args;
use monocle::{As2rel, As2relSearchResult, As2relSortOrder, MonocleConfig};
use serde_json::json;
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the As2rel command
#[derive(Args)]
pub struct As2relArgs {
    /// One or two ASNs to query relationships for
    #[clap(required = true)]
    pub asns: Vec<u32>,

    /// Force update the local as2rel database
    #[clap(short, long)]
    pub update: bool,

    /// Update with a custom data file (local path or URL)
    #[clap(long)]
    pub update_with: Option<String>,

    /// Output to pretty table, default markdown table
    #[clap(short, long)]
    pub pretty: bool,

    /// Hide the explanation text
    #[clap(long)]
    pub no_explain: bool,

    /// Sort by ASN2 ascending instead of connected percentage descending
    #[clap(long)]
    pub sort_by_asn: bool,

    /// Show organization name for ASN2 (from as2org database)
    #[clap(long)]
    pub show_name: bool,
}

pub fn run(config: &MonocleConfig, args: As2relArgs, json_output: bool) {
    let As2relArgs {
        asns,
        update,
        update_with,
        pretty,
        no_explain,
        sort_by_asn,
        show_name,
    } = args;

    if asns.is_empty() || asns.len() > 2 {
        eprintln!("ERROR: Please provide one or two ASNs");
        std::process::exit(1);
    }

    let data_dir = config.data_dir.as_str();
    let db_path = format!("{data_dir}/monocle-data.sqlite3");
    let as2rel = match As2rel::new(&Some(db_path.clone())) {
        Ok(as2rel) => as2rel,
        Err(e) => {
            eprintln!("Failed to create AS2rel database: {}", e);
            std::process::exit(1);
        }
    };

    // Handle updates
    if update || update_with.is_some() {
        println!("Updating AS2rel data...");
        let result = match &update_with {
            Some(path) => as2rel.update_with(path),
            None => as2rel.update(),
        };
        if let Err(e) = result {
            eprintln!("Failed to update AS2rel data: {}", e);
            std::process::exit(1);
        }
        println!("AS2rel data updated successfully");
    }

    // Check if data needs to be initialized or updated
    if as2rel.should_update() && !update && update_with.is_none() {
        println!("AS2rel data is empty or outdated, updating now...");
        if let Err(e) = as2rel.update() {
            eprintln!("Failed to update AS2rel data: {}", e);
            std::process::exit(1);
        }
        println!("AS2rel data updated successfully");
    }

    // Query relationships (use JOIN-based lookup if names are requested)
    let mut results: Vec<As2relSearchResult> = match asns.len() {
        1 => {
            let asn = asns[0];
            let search_result = if show_name {
                as2rel.search_asn_with_names(asn)
            } else {
                as2rel.search_asn(asn)
            };
            match search_result {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error searching for ASN {}: {}", asn, e);
                    std::process::exit(1);
                }
            }
        }
        2 => {
            let asn1 = asns[0];
            let asn2 = asns[1];
            let search_result = if show_name {
                as2rel.search_pair_with_names(asn1, asn2)
            } else {
                as2rel.search_pair(asn1, asn2)
            };
            match search_result {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Error searching for ASN pair {} - {}: {}", asn1, asn2, e);
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("ERROR: Please provide one or two ASNs");
            std::process::exit(1);
        }
    };

    if results.is_empty() {
        if asns.len() == 1 {
            println!("No relationships found for ASN {}", asns[0]);
        } else {
            println!(
                "No relationship found between ASN {} and ASN {}",
                asns[0], asns[1]
            );
        }
        return;
    }

    // Sort results
    let sort_order = if sort_by_asn {
        As2relSortOrder::Asn2Asc
    } else {
        As2relSortOrder::ConnectedDesc
    };
    As2rel::sort_results(&mut results, sort_order);

    // Output results
    if json_output {
        let max_peers = as2rel.get_max_peers_count();
        let json_results: Vec<_> = results
            .iter()
            .map(|r| {
                if show_name {
                    json!({
                        "asn1": r.asn1,
                        "asn2": r.asn2,
                        "asn2_name": r.asn2_name.as_deref().unwrap_or(""),
                        "connected": &r.connected,
                        "peer": &r.peer,
                        "as1_upstream": &r.as1_upstream,
                        "as2_upstream": &r.as2_upstream,
                    })
                } else {
                    json!({
                        "asn1": r.asn1,
                        "asn2": r.asn2,
                        "connected": &r.connected,
                        "peer": &r.peer,
                        "as1_upstream": &r.as1_upstream,
                        "as2_upstream": &r.as2_upstream,
                    })
                }
            })
            .collect();
        let output = json!({
            "max_peers_count": max_peers,
            "results": json_results,
        });
        match serde_json::to_string_pretty(&output) {
            Ok(s) => println!("{}", s),
            Err(e) => eprintln!("Error serializing JSON: {}", e),
        }
    } else {
        // Print explanation unless --no-explain is set
        if !no_explain {
            println!("{}", as2rel.get_explanation());
        }

        if show_name {
            let results_with_name: Vec<_> = results.into_iter().map(|r| r.with_name()).collect();
            let mut table = Table::new(&results_with_name);
            if pretty {
                println!("{}", table.with(Style::rounded()));
            } else {
                println!("{}", table.with(Style::markdown()));
            }
        } else {
            let mut table = Table::new(&results);
            if pretty {
                println!("{}", table.with(Style::rounded()));
            } else {
                println!("{}", table.with(Style::markdown()));
            }
        }
    }
}
