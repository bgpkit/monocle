use chrono::NaiveDate;
use clap::Subcommand;
use monocle::lens::rpki::{
    RpkiAspaLookupArgs, RpkiAspaTableEntry, RpkiDataSource, RpkiLens, RpkiListArgs,
    RpkiRoaLookupArgs, RpkiSummaryArgs, RpkiValidationArgs, RpkiViewsCollectorOption,
};
use monocle::lens::utils::OutputFormat;
use tabled::settings::object::Columns;
use tabled::settings::width::Width;
use tabled::settings::Style;
use tabled::Table;

#[derive(Subcommand)]
pub enum RpkiCommands {
    /// validate a prefix-asn pair with a RPKI validator (Cloudflare GraphQL)
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

pub fn run(commands: RpkiCommands, output_format: OutputFormat) {
    match commands {
        RpkiCommands::Check { asn, prefix } => run_check(asn, prefix, output_format),
        RpkiCommands::List { resource } => run_list(resource, output_format),
        RpkiCommands::Summary { asns } => run_summary(asns, output_format),
        RpkiCommands::Roas {
            origin,
            prefix,
            date,
            source,
            collector,
        } => run_roas(origin, prefix, date, source, collector, output_format),
        RpkiCommands::Aspas {
            customer,
            provider,
            date,
            source,
            collector,
        } => run_aspas(customer, provider, date, source, collector, output_format),
    }
}

fn run_check(asn: u32, prefix: String, output_format: OutputFormat) {
    let lens = RpkiLens::new();
    let args = RpkiValidationArgs::new(asn, &prefix);

    let (validity, roas) = match lens.validate(&args) {
        Ok((v1, v2)) => (v1, v2),
        Err(e) => {
            eprintln!("ERROR: unable to check RPKI validity: {}", e);
            return;
        }
    };

    match output_format {
        OutputFormat::Table => {
            let mut output = Table::new(vec![&validity])
                .with(Style::rounded())
                .to_string();

            if !roas.is_empty() {
                let covering_items: Vec<monocle::lens::rpki::RpkiRoaTableItem> =
                    roas.iter().cloned().map(|r| r.into()).collect();
                output.push_str("\n\nCovering ROAs:\n");
                output.push_str(
                    &Table::new(covering_items)
                        .with(Style::rounded())
                        .to_string(),
                );
            }
            println!("{}", output);
        }
        OutputFormat::Markdown => {
            let mut output = Table::new(vec![&validity])
                .with(Style::markdown())
                .to_string();

            if !roas.is_empty() {
                let covering_items: Vec<monocle::lens::rpki::RpkiRoaTableItem> =
                    roas.iter().cloned().map(|r| r.into()).collect();
                output.push_str("\n\nCovering ROAs:\n");
                output.push_str(
                    &Table::new(covering_items)
                        .with(Style::markdown())
                        .to_string(),
                );
            }
            println!("{}", output);
        }
        OutputFormat::Json => {
            let result = serde_json::json!({
                "validity": validity,
                "covering_roas": roas,
            });
            match serde_json::to_string(&result) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonPretty => {
            let result = serde_json::json!({
                "validity": validity,
                "covering_roas": roas,
            });
            match serde_json::to_string_pretty(&result) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonLine => {
            let result = serde_json::json!({
                "validity": validity,
                "covering_roas": roas,
            });
            match serde_json::to_string(&result) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::Psv => {
            // RpkiValidity fields are private, serialize to JSON and extract
            let json_val = serde_json::to_value(&validity).unwrap_or_default();
            let asn = json_val.get("asn").and_then(|v| v.as_u64()).unwrap_or(0);
            let prefix = json_val
                .get("prefix")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let validity_str = json_val
                .get("validity")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let description = json_val
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            println!("asn|prefix|validity|description");
            println!("{}|{}|{}|{}", asn, prefix, validity_str, description);
            if !roas.is_empty() {
                eprintln!("\nCovering ROAs:");
                println!("asn|prefix|max_length|trust_anchor");
                for roa in &roas {
                    let roa_json = serde_json::to_value(roa).unwrap_or_default();
                    println!(
                        "{}|{}|{}|{}",
                        roa_json.get("asn").and_then(|v| v.as_u64()).unwrap_or(0),
                        roa_json
                            .get("prefix")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        roa_json
                            .get("max_length")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        roa_json
                            .get("trust_anchor")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                    );
                }
            }
        }
    }
}

fn run_list(resource: String, output_format: OutputFormat) {
    let lens = RpkiLens::new();
    let args = RpkiListArgs {
        resource: resource.clone(),
        format: monocle::lens::rpki::RpkiOutputFormat::Table, // Not used, we handle format ourselves
    };

    let roas = match lens.list_roas(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return;
        }
    };

    if roas.is_empty() {
        if output_format.is_json() {
            println!("[]");
        } else {
            println!("no matching ROAs found for {}", resource);
        }
        return;
    }

    match output_format {
        OutputFormat::Table => {
            println!("{}", Table::new(&roas).with(Style::rounded()));
        }
        OutputFormat::Markdown => {
            println!("{}", Table::new(&roas).with(Style::markdown()));
        }
        OutputFormat::Json => match serde_json::to_string(&roas) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(&roas) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonLine => {
            for roa in &roas {
                match serde_json::to_string(roa) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("asn|prefix|max_length|name");
            for roa in &roas {
                let roa_json = serde_json::to_value(roa).unwrap_or_default();
                println!(
                    "{}|{}|{}|{}",
                    roa_json.get("asn").and_then(|v| v.as_u64()).unwrap_or(0),
                    roa_json
                        .get("prefix")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    roa_json
                        .get("max_length")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    roa_json.get("name").and_then(|v| v.as_str()).unwrap_or("")
                );
            }
        }
    }
}

fn run_summary(asns: Vec<u32>, output_format: OutputFormat) {
    let lens = RpkiLens::new();

    let mut results = Vec::new();
    for asn in asns {
        let args = RpkiSummaryArgs::new(asn);
        match lens.summarize(&args) {
            Ok(summary) => results.push(summary),
            Err(e) => {
                eprintln!("Failed to summarize ASN {}: {}", asn, e);
            }
        }
    }

    if results.is_empty() {
        if output_format.is_json() {
            println!("[]");
        }
        return;
    }

    match output_format {
        OutputFormat::Table => {
            println!("{}", Table::new(&results).with(Style::rounded()));
        }
        OutputFormat::Markdown => {
            println!("{}", Table::new(&results).with(Style::markdown()));
        }
        OutputFormat::Json => match serde_json::to_string(&results) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(&results) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonLine => {
            for result in &results {
                match serde_json::to_string(result) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("asn|roas_count|ipv4_prefixes|ipv6_prefixes");
            for r in &results {
                let r_json = serde_json::to_value(r).unwrap_or_default();
                println!(
                    "{}|{}|{}|{}",
                    r_json.get("asn").and_then(|v| v.as_u64()).unwrap_or(0),
                    r_json
                        .get("roas_count")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    r_json
                        .get("ipv4_prefixes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    r_json
                        .get("ipv6_prefixes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                );
            }
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
    output_format: OutputFormat,
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
        format: monocle::lens::rpki::RpkiOutputFormat::Table, // Not used
    };

    let roas = match lens.get_roas(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: Failed to get ROAs: {}", e);
            return;
        }
    };

    if roas.is_empty() {
        if output_format.is_json() {
            println!("[]");
        } else {
            println!("No ROAs found matching the criteria");
        }
        return;
    }

    eprintln!(
        "Found {} ROAs{}",
        roas.len(),
        match &date {
            Some(d) => format!(" (historical data from {})", d),
            None => " (current data)".to_string(),
        }
    );

    match output_format {
        OutputFormat::Table => {
            println!("{}", Table::new(&roas).with(Style::rounded()));
        }
        OutputFormat::Markdown => {
            println!("{}", Table::new(&roas).with(Style::markdown()));
        }
        OutputFormat::Json => match serde_json::to_string(&roas) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(&roas) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonLine => {
            for roa in &roas {
                match serde_json::to_string(roa) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("prefix|asn|max_length|not_before|not_after");
            for roa in &roas {
                let roa_json = serde_json::to_value(roa).unwrap_or_default();
                println!(
                    "{}|{}|{}|{}|{}",
                    roa_json
                        .get("prefix")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    roa_json.get("asn").and_then(|v| v.as_u64()).unwrap_or(0),
                    roa_json
                        .get("max_length")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    roa_json
                        .get("not_before")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    roa_json
                        .get("not_after")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                );
            }
        }
    }
}

fn run_aspas(
    customer: Option<u32>,
    provider: Option<u32>,
    date: Option<String>,
    source: String,
    collector: String,
    output_format: OutputFormat,
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
        format: monocle::lens::rpki::RpkiOutputFormat::Table, // Not used
    };

    let aspas = match lens.get_aspas(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: Failed to get ASPAs: {}", e);
            return;
        }
    };

    if aspas.is_empty() {
        if output_format.is_json() {
            println!("[]");
        } else {
            println!("No ASPAs found matching the criteria");
        }
        return;
    }

    eprintln!(
        "Found {} ASPAs{}",
        aspas.len(),
        match &date {
            Some(d) => format!(" (historical data from {})", d),
            None => " (current data)".to_string(),
        }
    );

    match output_format {
        OutputFormat::Table => {
            let table_entries: Vec<RpkiAspaTableEntry> = aspas.iter().map(|a| a.into()).collect();
            println!(
                "{}",
                Table::new(table_entries)
                    .with(Style::rounded())
                    .modify(Columns::last(), Width::wrap(60).keep_words(true))
            );
        }
        OutputFormat::Markdown => {
            let table_entries: Vec<RpkiAspaTableEntry> = aspas.iter().map(|a| a.into()).collect();
            println!(
                "{}",
                Table::new(table_entries)
                    .with(Style::markdown())
                    .modify(Columns::last(), Width::wrap(60).keep_words(true))
            );
        }
        OutputFormat::Json => match serde_json::to_string(&aspas) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(&aspas) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonLine => {
            for aspa in &aspas {
                match serde_json::to_string(aspa) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            println!("customer_asn|providers");
            for aspa in &aspas {
                let aspa_json = serde_json::to_value(aspa).unwrap_or_default();
                let customer = aspa_json
                    .get("customer_asn")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let providers = aspa_json
                    .get("providers")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|p| p.as_u64())
                            .map(|p| p.to_string())
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                println!("{}|{}", customer, providers);
            }
        }
    }
}
