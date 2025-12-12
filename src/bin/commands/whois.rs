use clap::Args;
use monocle::database::MonocleDatabase;
use monocle::lens::as2org::{As2orgLens, As2orgSearchArgs};
use monocle::lens::utils::OutputFormat;
use monocle::MonocleConfig;
use serde::Serialize;
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the Whois command
#[derive(Args)]
pub struct WhoisArgs {
    /// Search query, an ASN (e.g. "400644") or a name (e.g. "bgpkit")
    pub query: Vec<String>,

    /// Search AS and Org name only
    #[clap(short, long)]
    pub name_only: bool,

    /// Search by ASN only
    #[clap(short, long)]
    pub asn_only: bool,

    /// Search by country only
    #[clap(short = 'C', long)]
    pub country_only: bool,

    /// Refresh the local as2org database
    #[clap(short, long)]
    pub update: bool,

    /// Display a full table (with org_id, org_size)
    #[clap(short = 'F', long)]
    pub full_table: bool,

    /// Show full country names instead of 2-letter code
    #[clap(short, long)]
    pub full_country: bool,

    /// Show full names without truncation (default truncates to 20 chars)
    #[clap(long)]
    pub show_full_name: bool,
}

pub fn run(config: &MonocleConfig, args: WhoisArgs, output_format: OutputFormat) {
    let WhoisArgs {
        query,
        name_only,
        asn_only,
        country_only,
        update,
        full_table,
        full_country,
        show_full_name,
    } = args;

    let sqlite_path = config.sqlite_path();

    // Handle update request
    if update {
        eprintln!("Updating AS2org data...");
        let db = match MonocleDatabase::open(&sqlite_path) {
            Ok(db) => db,
            Err(e) => {
                eprintln!("Failed to open database: {}", e);
                std::process::exit(1);
            }
        };

        let lens = As2orgLens::new(&db);
        match lens.bootstrap() {
            Ok(count) => {
                eprintln!("AS2org data updated: {} entries", count);
            }
            Err(e) => {
                eprintln!("Failed to update AS2org data: {}", e);
                std::process::exit(1);
            }
        }

        // If no query provided after update, just exit
        if query.is_empty() {
            return;
        }

        // Continue with query using the same connection
        run_query(
            &db,
            &query,
            name_only,
            asn_only,
            country_only,
            full_country,
            full_table,
            show_full_name,
            output_format,
        );
        return;
    }

    // Open the database
    let db = match MonocleDatabase::open(&sqlite_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    let lens = As2orgLens::new(&db);

    // Bootstrap if needed
    if lens.needs_bootstrap() {
        eprintln!("Bootstrapping AS2org data now... (this may take a few seconds)");

        match lens.bootstrap() {
            Ok(count) => {
                eprintln!("Bootstrapping complete: {} entries", count);
            }
            Err(e) => {
                eprintln!("Failed to bootstrap AS2org data: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Run query
    run_query(
        &db,
        &query,
        name_only,
        asn_only,
        country_only,
        full_country,
        full_table,
        show_full_name,
        output_format,
    );
}

#[derive(Debug, Clone, Serialize, tabled::Tabled)]
struct WhoisResultConcise {
    asn: u32,
    as_name: String,
    org_name: String,
    org_country: String,
}

fn run_query(
    db: &MonocleDatabase,
    query: &[String],
    name_only: bool,
    asn_only: bool,
    country_only: bool,
    full_country: bool,
    full_table: bool,
    show_full_name: bool,
    output_format: OutputFormat,
) {
    // Validate conflicting search type options
    let exclusive_options = [name_only, asn_only, country_only];
    if exclusive_options.iter().filter(|&&x| x).count() > 1 {
        eprintln!("ERROR: conflicting search type options - only one of --name-only, --asn-only, or --country-only can be specified");
        std::process::exit(1);
    }

    // Build search args
    let search_args = As2orgSearchArgs {
        query: query.to_vec(),
        name_only,
        asn_only,
        country_only,
        full_country,
        full_table,
    };

    // Validate
    if let Err(e) = search_args.validate() {
        eprintln!("ERROR: {}", e);
        std::process::exit(1);
    }

    let lens = As2orgLens::new(db);

    // Perform search
    let results = match lens.search(&search_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Search error: {}", e);
            std::process::exit(1);
        }
    };

    // Truncate names for table output unless show_full_name is set
    let truncate_names = !show_full_name && output_format.is_table();

    // Format and print results based on output format
    match output_format {
        OutputFormat::Table => {
            if full_table {
                let display: Vec<_> = results
                    .iter()
                    .map(|r| r.to_truncated(truncate_names))
                    .collect();
                println!("{}", Table::new(display).with(Style::rounded()));
            } else {
                let display: Vec<WhoisResultConcise> = results
                    .iter()
                    .map(|r| {
                        let concise = r.to_concise_truncated(truncate_names);
                        WhoisResultConcise {
                            asn: concise.asn,
                            as_name: concise.as_name,
                            org_name: concise.org_name,
                            org_country: concise.org_country,
                        }
                    })
                    .collect();
                println!("{}", Table::new(display).with(Style::rounded()));
            }
        }
        OutputFormat::Markdown => {
            if full_table {
                let display: Vec<_> = results
                    .iter()
                    .map(|r| r.to_truncated(truncate_names))
                    .collect();
                println!("{}", Table::new(display).with(Style::markdown()));
            } else {
                let display: Vec<WhoisResultConcise> = results
                    .iter()
                    .map(|r| {
                        let concise = r.to_concise_truncated(truncate_names);
                        WhoisResultConcise {
                            asn: concise.asn,
                            as_name: concise.as_name,
                            org_name: concise.org_name,
                            org_country: concise.org_country,
                        }
                    })
                    .collect();
                println!("{}", Table::new(display).with(Style::markdown()));
            }
        }
        OutputFormat::Json => {
            let output = if full_table {
                serde_json::to_string(&results)
            } else {
                let concise: Vec<WhoisResultConcise> = results
                    .iter()
                    .map(|r| {
                        let c = r.to_concise_truncated(false);
                        WhoisResultConcise {
                            asn: c.asn,
                            as_name: c.as_name,
                            org_name: c.org_name,
                            org_country: c.org_country,
                        }
                    })
                    .collect();
                serde_json::to_string(&concise)
            };
            match output {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonPretty => {
            let output = if full_table {
                serde_json::to_string_pretty(&results)
            } else {
                let concise: Vec<WhoisResultConcise> = results
                    .iter()
                    .map(|r| {
                        let c = r.to_concise_truncated(false);
                        WhoisResultConcise {
                            asn: c.asn,
                            as_name: c.as_name,
                            org_name: c.org_name,
                            org_country: c.org_country,
                        }
                    })
                    .collect();
                serde_json::to_string_pretty(&concise)
            };
            match output {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonLine => {
            if full_table {
                for r in &results {
                    match serde_json::to_string(r) {
                        Ok(json) => println!("{}", json),
                        Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                    }
                }
            } else {
                for r in &results {
                    let c = r.to_concise_truncated(false);
                    let concise = WhoisResultConcise {
                        asn: c.asn,
                        as_name: c.as_name,
                        org_name: c.org_name,
                        org_country: c.org_country,
                    };
                    match serde_json::to_string(&concise) {
                        Ok(json) => println!("{}", json),
                        Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                    }
                }
            }
        }
        OutputFormat::Psv => {
            if full_table {
                println!("asn|as_name|org_name|org_id|org_country|org_size");
                for r in &results {
                    println!(
                        "{}|{}|{}|{}|{}|{}",
                        r.asn, r.as_name, r.org_name, r.org_id, r.org_country, r.org_size
                    );
                }
            } else {
                println!("asn|as_name|org_name|org_country");
                for r in &results {
                    println!("{}|{}|{}|{}", r.asn, r.as_name, r.org_name, r.org_country);
                }
            }
        }
    }
}
