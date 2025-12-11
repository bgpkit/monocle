use clap::Args;
use monocle::database::MonocleDatabase;
use monocle::lens::as2org::{As2orgLens, As2orgOutputFormat, As2orgSearchArgs};
use monocle::MonocleConfig;

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

    /// Output to pretty table, default markdown table
    #[clap(short, long)]
    pub pretty: bool,

    /// Display a full table (with ord_id, org_size)
    #[clap(short = 'F', long)]
    pub full_table: bool,

    /// Export to pipe-separated values
    #[clap(short = 'P', long)]
    pub psv: bool,

    /// Show full country names instead of 2-letter code
    #[clap(short, long)]
    pub full_country: bool,
}

pub fn run(config: &MonocleConfig, args: WhoisArgs, json_output: bool) {
    let WhoisArgs {
        query,
        name_only,
        asn_only,
        country_only,
        update,
        pretty,
        full_table,
        full_country,
        psv,
    } = args;

    // Open the monocle database
    let data_dir = config.data_dir.as_str();
    let db_path = format!("{}/monocle-data.sqlite3", data_dir);
    let db = match MonocleDatabase::open(&db_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    // Create the lens
    let lens = As2orgLens::new(&db);

    // Handle update request
    if update {
        if !json_output {
            println!("Updating AS2org data...");
        }
        match lens.bootstrap() {
            Ok((as_count, org_count)) => {
                if !json_output {
                    println!(
                        "AS2org data updated: {} ASes, {} organizations",
                        as_count, org_count
                    );
                }
            }
            Err(e) => {
                eprintln!("Failed to update AS2org data: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Bootstrap if needed
    if lens.needs_bootstrap() {
        if !json_output {
            println!("Bootstrapping AS2org data now... (this may take about a minute)");
        }
        match lens.bootstrap() {
            Ok((as_count, org_count)) => {
                if !json_output {
                    println!(
                        "Bootstrapping complete: {} ASes, {} organizations",
                        as_count, org_count
                    );
                }
            }
            Err(e) => {
                eprintln!("Failed to bootstrap AS2org data: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Validate conflicting search type options
    let exclusive_options = [name_only, asn_only, country_only];
    if exclusive_options.iter().filter(|&&x| x).count() > 1 {
        eprintln!("ERROR: conflicting search type options - only one of --name-only, --asn-only, or --country-only can be specified");
        std::process::exit(1);
    }

    // Build search args
    let search_args = As2orgSearchArgs {
        query,
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

    // Perform search
    let results = match lens.search(&search_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Search error: {}", e);
            std::process::exit(1);
        }
    };

    // Determine output format
    let format = if json_output {
        As2orgOutputFormat::Json
    } else if psv {
        As2orgOutputFormat::Psv
    } else if pretty {
        As2orgOutputFormat::Pretty
    } else {
        As2orgOutputFormat::Markdown
    };

    // Format and print results
    let output = lens.format_results(&results, &format, full_table);
    println!("{}", output);
}
