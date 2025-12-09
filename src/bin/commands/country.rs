use clap::Args;
use monocle::{CountryEntry, CountryLookup};
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the Country command
#[derive(Args)]
pub struct CountryArgs {
    /// Search query, e.g. "US" or "United States"
    pub queries: Vec<String>,
}

pub fn run(args: CountryArgs) {
    let CountryArgs { queries } = args;

    let lookup = CountryLookup::new();
    let res: Vec<CountryEntry> = queries
        .into_iter()
        .flat_map(|query| lookup.lookup(query.as_str()))
        .collect();
    println!("{}", Table::new(res).with(Style::rounded()));
}
