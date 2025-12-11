use chrono::NaiveDate;
use clap::Subcommand;
use monocle::lens::rpki::{
    RpkiAspaLookupArgs, RpkiDataSource, RpkiLens, RpkiListArgs, RpkiOutputFormat,
    RpkiRoaLookupArgs, RpkiSummaryArgs, RpkiValidationArgs, RpkiViewsCollectorOption,
};
use tabled::settings::object::Columns;
use tabled::settings::width::Width;
use tabled::settings::Style;
use tabled::Table;

#[derive(Subcommand)]
pub enum RpkiCommands {
    /// validate a prefix-asn pair with a RPKI validator (Cloudflare)
    Check {
        #[clap(short, long)]
        asn: u32,

        #[clap(short, long)]
        prefix: String,
    },

    /// list ROAs by ASN or prefix (Cloudflare real-time)
    List {
        /// prefix or ASN
        #[clap()]
        resource: String,
    },

    /// summarize RPKI status for a list of given ASNs (Cloudflare)
    Summary {
        #[clap()]
        asns: Vec<u32>,
    },

    /// list ROAs from RPKI data (current or historical via bgpkit-commons)
    Roas {
        /// Filter by origin ASN
        #[clap(long)]
        origin: Option<u32>,

        /// Filter by prefix
        #[clap(long)]
        prefix: Option<String>,

        /// Load historical data for this date (YYYY-MM-DD)
        #[clap(long)]
        date: Option<String>,

        /// Historical data source: ripe, rpkiviews (default: ripe)
        #[clap(long, default_value = "ripe")]
        source: String,

        /// RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost)
        #[clap(long, default_value = "soborost")]
        collector: String,
    },

    /// list ASPAs from RPKI data (current or historical via bgpkit-commons)
    Aspas {
        /// Filter by customer ASN
        #[clap(long)]
        customer: Option<u32>,

        /// Filter by provider ASN
        #[clap(long)]
        provider: Option<u32>,

        /// Load historical data for this date (YYYY-MM-DD)
        #[clap(long)]
        date: Option<String>,

        /// Historical data source: ripe, rpkiviews (default: ripe)
        #[clap(long, default_value = "ripe")]
        source: String,

        /// RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost)
        #[clap(long, default_value = "soborost")]
        collector: String,
    },
}

pub fn run(commands: RpkiCommands, json: bool) {
    match commands {
        RpkiCommands::Check { asn, prefix } => run_check(asn, prefix, json),
        RpkiCommands::List { resource } => run_list(resource, json),
        RpkiCommands::Summary { asns } => run_summary(asns, json),
        RpkiCommands::Roas {
            origin,
            prefix,
            date,
            source,
            collector,
        } => run_roas(origin, prefix, date, source, collector, json),
        RpkiCommands::Aspas {
            customer,
            provider,
            date,
            source,
            collector,
        } => run_aspas(customer, provider, date, source, collector, json),
    }
}

fn run_check(asn: u32, prefix: String, json: bool) {
    let lens = RpkiLens::new();
    let args = RpkiValidationArgs::new(asn, &prefix);

    let (validity, roas) = match lens.validate(&args) {
        Ok((v1, v2)) => (v1, v2),
        Err(e) => {
            eprintln!("ERROR: unable to check RPKI validity: {}", e);
            return;
        }
    };

    let format = if json {
        RpkiOutputFormat::Json
    } else {
        RpkiOutputFormat::Table
    };

    let output = lens.format_validation(&validity, &roas, &format);
    println!("{}", output);
}

fn run_list(resource: String, json: bool) {
    let lens = RpkiLens::new();
    let args = RpkiListArgs {
        resource: resource.clone(),
        format: if json {
            RpkiOutputFormat::Json
        } else {
            RpkiOutputFormat::Table
        },
    };

    let roas = match lens.list_roas(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return;
        }
    };

    if roas.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("no matching ROAs found for {}", resource);
        }
        return;
    }

    let output = lens.format_roa_items(&roas, &args.format);
    println!("{}", output);
}

fn run_summary(asns: Vec<u32>, json: bool) {
    let lens = RpkiLens::new();
    let format = if json {
        RpkiOutputFormat::Json
    } else {
        RpkiOutputFormat::Table
    };

    let mut results = Vec::new();
    for asn in asns {
        let args = RpkiSummaryArgs::new(asn);
        match lens.summarize(&args) {
            Ok(summary) => results.push(summary),
            Err(e) => {
                if !json {
                    eprintln!("Failed to summarize ASN {}: {}", asn, e);
                }
            }
        }
    }

    if results.is_empty() {
        if json {
            println!("[]");
        }
        return;
    }

    match format {
        RpkiOutputFormat::Json => match serde_json::to_string_pretty(&results) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        _ => {
            println!("{}", Table::new(&results).with(Style::markdown()));
        }
    }
}

fn parse_data_source(source: &str) -> RpkiDataSource {
    match source.to_lowercase().as_str() {
        "ripe" => RpkiDataSource::Ripe,
        "rpkiviews" => RpkiDataSource::RpkiViews,
        _ => RpkiDataSource::Cloudflare,
    }
}

fn parse_collector(collector: &str) -> Option<RpkiViewsCollectorOption> {
    match collector.to_lowercase().as_str() {
        "soborost" => Some(RpkiViewsCollectorOption::Soborost),
        "massars" => Some(RpkiViewsCollectorOption::Massars),
        "attn" => Some(RpkiViewsCollectorOption::Attn),
        "kerfuffle" => Some(RpkiViewsCollectorOption::Kerfuffle),
        _ => None,
    }
}

fn run_roas(
    origin: Option<u32>,
    prefix: Option<String>,
    date: Option<String>,
    source: String,
    collector: String,
    json: bool,
) {
    // Parse date if provided
    let parsed_date = match &date {
        Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
            Ok(date) => Some(date),
            Err(e) => {
                eprintln!("ERROR: Invalid date format '{}': {}. Use YYYY-MM-DD", d, e);
                return;
            }
        },
        None => None,
    };

    let mut lens = RpkiLens::new();
    let args = RpkiRoaLookupArgs {
        prefix,
        asn: origin,
        date: parsed_date,
        source: parse_data_source(&source),
        collector: parse_collector(&collector),
        format: if json {
            RpkiOutputFormat::Json
        } else {
            RpkiOutputFormat::Table
        },
    };

    let roas = match lens.get_roas(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: Failed to get ROAs: {}", e);
            return;
        }
    };

    if roas.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No ROAs found matching the criteria");
        }
        return;
    }

    if !json {
        println!(
            "Found {} ROAs{}",
            roas.len(),
            match &date {
                Some(d) => format!(" (historical data from {})", d),
                None => " (current data)".to_string(),
            }
        );
    }

    let output = lens.format_roas(&roas, &args.format);
    println!("{}", output);
}

fn run_aspas(
    customer: Option<u32>,
    provider: Option<u32>,
    date: Option<String>,
    source: String,
    collector: String,
    json: bool,
) {
    // Parse date if provided
    let parsed_date = match &date {
        Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
            Ok(date) => Some(date),
            Err(e) => {
                eprintln!("ERROR: Invalid date format '{}': {}. Use YYYY-MM-DD", d, e);
                return;
            }
        },
        None => None,
    };

    let mut lens = RpkiLens::new();
    let args = RpkiAspaLookupArgs {
        customer_asn: customer,
        provider_asn: provider,
        date: parsed_date,
        source: parse_data_source(&source),
        collector: parse_collector(&collector),
        format: if json {
            RpkiOutputFormat::Json
        } else {
            RpkiOutputFormat::Table
        },
    };

    let aspas = match lens.get_aspas(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: Failed to get ASPAs: {}", e);
            return;
        }
    };

    if aspas.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No ASPAs found matching the criteria");
        }
        return;
    }

    if !json {
        println!(
            "Found {} ASPAs{}",
            aspas.len(),
            match &date {
                Some(d) => format!(" (historical data from {})", d),
                None => " (current data)".to_string(),
            }
        );
    }

    match args.format {
        RpkiOutputFormat::Json => match serde_json::to_string_pretty(&aspas) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        _ => {
            println!(
                "{}",
                Table::new(
                    aspas
                        .iter()
                        .map(monocle::lens::rpki::RpkiAspaTableEntry::from)
                )
                .with(Style::markdown())
                .modify(Columns::last(), Width::wrap(60).keep_words(true))
            );
        }
    }
}
