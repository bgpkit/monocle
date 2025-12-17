use clap::Args;
use monocle::lens::country::{CountryEntry, CountryLens, CountryLookupArgs};
use monocle::lens::utils::OutputFormat;
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the Country command
#[derive(Args)]
pub struct CountryArgs {
    /// Search query: country code (e.g., "US") or partial name (e.g., "united")
    #[clap(value_name = "QUERY")]
    pub query: Option<String>,

    /// List all countries
    #[clap(short, long)]
    pub all: bool,

    /// Output as simple text (code: name)
    #[clap(short, long)]
    pub simple: bool,
}

pub fn run(args: CountryArgs, output_format: OutputFormat) {
    let CountryArgs { query, all, simple } = args;

    // Build lookup args
    let lookup_args = CountryLookupArgs {
        query,
        all,
        format: monocle::lens::country::CountryOutputFormat::Table, // Not used, we handle format ourselves
    };

    // Validate args
    if let Err(e) = lookup_args.validate() {
        eprintln!("ERROR: {}", e);
        std::process::exit(1);
    }

    let lens = CountryLens::new();

    match lens.search(&lookup_args) {
        Ok(results) => {
            // Simple mode overrides output format
            if simple {
                for entry in &results {
                    println!("{}: {}", entry.code, entry.name);
                }
                return;
            }

            format_output(&results, output_format);
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
    }
}

fn format_output(results: &[CountryEntry], output_format: OutputFormat) {
    if results.is_empty() {
        if output_format.is_json() {
            println!("[]");
        } else {
            println!("No countries found");
        }
        return;
    }

    match output_format {
        OutputFormat::Table => {
            println!("{}", Table::new(results).with(Style::rounded()));
        }
        OutputFormat::Markdown => {
            println!("{}", Table::new(results).with(Style::markdown()));
        }
        OutputFormat::Json => match serde_json::to_string(results) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(results) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonLine => {
            for entry in results {
                match serde_json::to_string(entry) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("code|name");
            for entry in results {
                println!("{}|{}", entry.code, entry.name);
            }
        }
    }
}
