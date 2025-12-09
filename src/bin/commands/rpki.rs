use chrono::NaiveDate;
use clap::Subcommand;
use ipnet::IpNet;
use monocle::{
    get_aspas, get_roas, list_by_asn, list_by_prefix, load_rpki_data, summarize_asn, validate,
    AspaTableEntry, RoaTableItem, SummaryTableItem,
};
use serde_json::json;
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
    let (validity, roas) = match validate(asn, prefix.as_str()) {
        Ok((v1, v2)) => (v1, v2),
        Err(e) => {
            eprintln!("ERROR: unable to check RPKI validity: {}", e);
            return;
        }
    };
    if json {
        let roa_items: Vec<RoaTableItem> = roas.into_iter().map(RoaTableItem::from).collect();
        let output = json!({
            "validation": validity,
            "covering_roas": roa_items
        });
        println!("{}", output);
    } else {
        println!("RPKI validation result:");
        println!("{}", Table::new(vec![validity]).with(Style::markdown()));
        println!();
        println!("Covering prefixes:");
        println!(
            "{}",
            Table::new(
                roas.into_iter()
                    .map(RoaTableItem::from)
                    .collect::<Vec<RoaTableItem>>()
            )
            .with(Style::markdown())
        );
    }
}

fn run_list(resource: String, json: bool) {
    let resources = match resource.parse::<u32>() {
        Ok(asn) => match list_by_asn(asn) {
            Ok(resources) => resources,
            Err(e) => {
                eprintln!("Failed to list ROAs for ASN {}: {}", asn, e);
                return;
            }
        },
        Err(_) => match resource.parse::<IpNet>() {
            Ok(prefix) => match list_by_prefix(&prefix) {
                Ok(resources) => resources,
                Err(e) => {
                    eprintln!("Failed to list ROAs for prefix {}: {}", prefix, e);
                    return;
                }
            },
            Err(_) => {
                eprintln!(
                    "ERROR: list resource not an AS number or a prefix: {}",
                    resource
                );
                return;
            }
        },
    };

    let roas: Vec<RoaTableItem> = resources
        .into_iter()
        .flat_map(Into::<Vec<RoaTableItem>>::into)
        .collect();
    if json {
        match serde_json::to_string(&roas) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        }
    } else if roas.is_empty() {
        println!("no matching ROAS found for {}", resource);
    } else {
        println!("{}", Table::new(roas).with(Style::markdown()));
    }
}

fn run_summary(asns: Vec<u32>, json: bool) {
    let res: Vec<SummaryTableItem> = asns
        .into_iter()
        .filter_map(|v| match summarize_asn(v) {
            Ok(summary) => Some(summary),
            Err(e) => {
                eprintln!("Failed to summarize ASN {}: {}", v, e);
                None
            }
        })
        .collect();

    if json {
        match serde_json::to_string(&res) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        }
    } else {
        println!("{}", Table::new(res).with(Style::markdown()));
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

    // Load RPKI data
    let commons = match load_rpki_data(parsed_date, Some(source.as_str()), Some(collector.as_str()))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: Failed to load RPKI data: {}", e);
            return;
        }
    };

    // Get ROAs with filters
    let roas = match get_roas(&commons, prefix.as_deref(), origin) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: Failed to get ROAs: {}", e);
            return;
        }
    };

    if json {
        match serde_json::to_string(&roas) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        }
    } else if roas.is_empty() {
        println!("No ROAs found matching the criteria");
    } else {
        println!(
            "Found {} ROAs{}",
            roas.len(),
            match &date {
                Some(d) => format!(" (historical data from {})", d),
                None => " (current data)".to_string(),
            }
        );
        println!("{}", Table::new(roas).with(Style::markdown()));
    }
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

    // Load RPKI data
    let commons = match load_rpki_data(parsed_date, Some(source.as_str()), Some(collector.as_str()))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: Failed to load RPKI data: {}", e);
            return;
        }
    };

    // Get ASPAs with filters
    let aspas = match get_aspas(&commons, customer, provider) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: Failed to get ASPAs: {}", e);
            return;
        }
    };

    if json {
        match serde_json::to_string(&aspas) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        }
    } else if aspas.is_empty() {
        println!("No ASPAs found matching the criteria");
    } else {
        println!(
            "Found {} ASPAs{}",
            aspas.len(),
            match &date {
                Some(d) => format!(" (historical data from {})", d),
                None => " (current data)".to_string(),
            }
        );
        let table_entries: Vec<AspaTableEntry> = aspas.iter().map(AspaTableEntry::from).collect();
        println!(
            "{}",
            Table::new(table_entries)
                .with(Style::markdown())
                .modify(Columns::last(), Width::wrap(60).keep_words(true))
        );
    }
}
