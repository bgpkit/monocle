use clap::Args;
use monocle::lens::time::{TimeBgpTime, TimeLens, TimeParseArgs};
use monocle::lens::utils::OutputFormat;
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the Time command
#[derive(Args)]
pub struct TimeArgs {
    /// Time stamp or time string to convert
    #[clap()]
    pub time: Vec<String>,

    /// Simple output, only print the converted time (RFC3339 format)
    #[clap(short, long)]
    pub simple: bool,
}

pub fn run(args: TimeArgs, output_format: OutputFormat) {
    let TimeArgs { time, simple } = args;

    let lens = TimeLens::new();
    let parse_args = TimeParseArgs::new(time);

    // Simple mode overrides output format
    if simple {
        match lens.parse_to_rfc3339(&parse_args.times) {
            Ok(results) => {
                for r in results {
                    println!("{}", r);
                }
            }
            Err(e) => eprintln!("ERROR: {}", e),
        }
        return;
    }

    // Parse times
    let results = match lens.parse(&parse_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return;
        }
    };

    // Format and print based on output format
    format_output(&results, output_format);
}

fn format_output(results: &[TimeBgpTime], output_format: OutputFormat) {
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
            for r in results {
                match serde_json::to_string(r) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("unix|rfc3339|human");
            for r in results {
                println!("{}|{}|{}", r.unix, r.rfc3339, r.human);
            }
        }
    }
}
