use clap::Args;
use monocle::database::MonocleDatabase;
use monocle::lens::as2rel::{As2relLens, As2relOutputFormat, As2relSearchArgs};
use monocle::MonocleConfig;

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

    // Validate ASN count
    if asns.is_empty() || asns.len() > 2 {
        eprintln!("ERROR: Please provide one or two ASNs");
        std::process::exit(1);
    }

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
    let lens = As2relLens::new(&db);

    // Handle explicit updates
    if update || update_with.is_some() {
        if !json_output {
            println!("Updating AS2rel data...");
        }
        let result = match &update_with {
            Some(path) => lens.update_from(path),
            None => lens.update(),
        };
        match result {
            Ok(count) => {
                if !json_output {
                    println!("AS2rel data updated: {} relationships loaded", count);
                }
            }
            Err(e) => {
                eprintln!("Failed to update AS2rel data: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Check if data needs to be initialized or updated automatically
    if lens.needs_update() && !update && update_with.is_none() {
        if !json_output {
            println!("AS2rel data is empty or outdated, updating now...");
        }
        match lens.update() {
            Ok(count) => {
                if !json_output {
                    println!("AS2rel data updated: {} relationships loaded", count);
                }
            }
            Err(e) => {
                eprintln!("Failed to update AS2rel data: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Build search args
    let search_args = As2relSearchArgs {
        asns: asns.clone(),
        sort_by_asn,
        show_name,
        no_explain,
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
            eprintln!("Error searching for AS relationships: {}", e);
            std::process::exit(1);
        }
    };

    // Handle empty results
    if results.is_empty() {
        if json_output {
            println!("[]");
        } else if asns.len() == 1 {
            println!("No relationships found for ASN {}", asns[0]);
        } else {
            println!(
                "No relationship found between ASN {} and ASN {}",
                asns[0], asns[1]
            );
        }
        return;
    }

    // Determine output format
    let format = if json_output {
        As2relOutputFormat::Json
    } else if pretty {
        As2relOutputFormat::Pretty
    } else {
        As2relOutputFormat::Markdown
    };

    // Print explanation unless --no-explain is set or JSON output
    if !no_explain && !json_output {
        println!("{}", lens.get_explanation());
    }

    // Format and print results
    let output = lens.format_results(&results, &format, show_name);
    println!("{}", output);
}
