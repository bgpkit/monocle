use clap::Args;
use monocle::lens::country::{CountryLens, CountryLookupArgs, CountryOutputFormat};

/// Arguments for the Country command
#[derive(Args)]
pub struct CountryArgs {
    /// Search query: country code (e.g., "US") or partial name (e.g., "united")
    #[clap(value_name = "QUERY")]
    pub query: Option<String>,

    /// List all countries
    #[clap(short, long)]
    pub all: bool,

    /// Output as JSON
    #[clap(long)]
    pub json: bool,

    /// Output as simple text (code: name)
    #[clap(short, long)]
    pub simple: bool,
}

pub fn run(args: CountryArgs) {
    let CountryArgs {
        query,
        all,
        json,
        simple,
    } = args;

    // Determine output format
    let format = if json {
        CountryOutputFormat::Json
    } else if simple {
        CountryOutputFormat::Simple
    } else {
        CountryOutputFormat::Table
    };

    // Build lookup args
    let lookup_args = CountryLookupArgs {
        query,
        all,
        format: format.clone(),
    };

    // Validate args
    if let Err(e) = lookup_args.validate() {
        eprintln!("ERROR: {}", e);
        std::process::exit(1);
    }

    let lens = CountryLens::new();

    match lens.search(&lookup_args) {
        Ok(results) => {
            let output = lens.format_results(&results, &format);
            println!("{}", output);
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
