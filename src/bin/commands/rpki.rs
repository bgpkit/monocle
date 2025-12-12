use chrono::NaiveDate;
use clap::Subcommand;
use monocle::lens::rpki::{
    RpkiAspaLookupArgs, RpkiAspaTableEntry, RpkiDataSource, RpkiLens, RpkiRoaEntry,
    RpkiRoaLookupArgs, RpkiSummaryArgs, RpkiValidationArgs, RpkiViewsCollectorOption,
};
use monocle::lens::utils::OutputFormat;
use std::collections::HashSet;
#[allow(unused_imports)]
use tabled::settings::object::Columns;
#[allow(unused_imports)]
use tabled::settings::width::Width;
use tabled::settings::Style;
use tabled::Table;

#[derive(Subcommand)]
pub enum RpkiCommands {
    /// validate a prefix-asn pair with a RPKI validator (Cloudflare GraphQL)
    Validate {
        /// Two resources: one prefix and one ASN (order does not matter)
        #[clap(num_args = 2)]
        resources: Vec<String>,
    },

    /// summarize RPKI status for a list of given ASNs (Cloudflare)
    Summary {
        #[clap()]
        asns: Vec<u32>,
    },

    /// list ROAs from RPKI data (current or historical via bgpkit-commons)
    Roas {
        /// Filter by resources (prefixes or ASNs, auto-detected)
        #[clap()]
        resources: Vec<String>,

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
        RpkiCommands::Validate { resources } => run_validate(resources, output_format),
        RpkiCommands::Summary { asns } => run_summary(asns, output_format),
        RpkiCommands::Roas {
            resources,
            date,
            source,
            collector,
        } => run_roas(resources, date, source, collector, output_format),
        RpkiCommands::Aspas {
            customer,
            provider,
            date,
            source,
            collector,
        } => run_aspas(customer, provider, date, source, collector, output_format),
    }
}

/// Parse a resource string into either an ASN (u32) or a prefix (String)
fn parse_resource(resource: &str) -> Result<ResourceType, String> {
    let trimmed = resource.trim();

    // Try to parse as ASN (with or without "AS" prefix)
    let asn_str = if trimmed.to_uppercase().starts_with("AS") {
        &trimmed[2..]
    } else {
        trimmed
    };

    if let Ok(asn) = asn_str.parse::<u32>() {
        return Ok(ResourceType::Asn(asn));
    }

    // Try to parse as prefix (contains '/' or ':' for IPv6 or '.' for IPv4)
    if trimmed.contains('/') || trimmed.contains(':') || trimmed.contains('.') {
        // Basic validation - should contain a slash for CIDR notation or be an IP
        return Ok(ResourceType::Prefix(trimmed.to_string()));
    }

    Err(format!(
        "Could not parse '{}' as either an ASN or a prefix",
        resource
    ))
}

#[derive(Debug, Clone)]
enum ResourceType {
    Asn(u32),
    Prefix(String),
}

fn run_validate(resources: Vec<String>, output_format: OutputFormat) {
    eprintln!("Data source: Cloudflare RPKI GraphQL API");

    if resources.len() != 2 {
        eprintln!(
            "ERROR: validate command requires exactly two resources (one prefix and one ASN)"
        );
        return;
    }

    let mut asn: Option<u32> = None;
    let mut prefix: Option<String> = None;

    for resource in &resources {
        match parse_resource(resource) {
            Ok(ResourceType::Asn(a)) => {
                if asn.is_some() {
                    eprintln!("ERROR: Two ASNs provided. Please provide one prefix and one ASN.");
                    return;
                }
                asn = Some(a);
            }
            Ok(ResourceType::Prefix(p)) => {
                if prefix.is_some() {
                    eprintln!(
                        "ERROR: Two prefixes provided. Please provide one prefix and one ASN."
                    );
                    return;
                }
                prefix = Some(p);
            }
            Err(e) => {
                eprintln!("ERROR: {}", e);
                return;
            }
        }
    }

    let asn = match asn {
        Some(a) => a,
        None => {
            eprintln!("ERROR: No ASN provided. Please provide one prefix and one ASN.");
            return;
        }
    };

    let prefix = match prefix {
        Some(p) => p,
        None => {
            eprintln!("ERROR: No prefix provided. Please provide one prefix and one ASN.");
            return;
        }
    };

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

fn run_summary(asns: Vec<u32>, output_format: OutputFormat) {
    eprintln!("Data source: Cloudflare RPKI GraphQL API");

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
    resources: Vec<String>,
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

    // Display data source - current data always uses Cloudflare
    let source_display = match date {
        Some(ref d) => format!(
            "Data source: {} (historical data from {})",
            source.to_uppercase(),
            d
        ),
        None => "Data source: CLOUDFLARE (current data)".to_string(),
    };
    eprintln!("{}", source_display);

    // Parse resources into ASNs and prefixes
    let mut asns: Vec<u32> = Vec::new();
    let mut prefixes: Vec<String> = Vec::new();

    for resource in &resources {
        match parse_resource(resource) {
            Ok(ResourceType::Asn(a)) => asns.push(a),
            Ok(ResourceType::Prefix(p)) => prefixes.push(p),
            Err(e) => {
                eprintln!("ERROR: {}", e);
                return;
            }
        }
    }

    let mut lens = RpkiLens::new();

    // If no resources specified, get all ROAs
    if asns.is_empty() && prefixes.is_empty() {
        let args = RpkiRoaLookupArgs {
            prefix: None,
            asn: None,
            date: parsed_date,
            source: parse_data_source(&source),
            collector: parse_collector(&collector),
            format: monocle::lens::rpki::RpkiOutputFormat::Table,
        };

        let roas = match lens.get_roas(&args) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ERROR: Failed to get ROAs: {}", e);
                return;
            }
        };

        output_roas(roas, output_format);
        return;
    }

    // Collect all ROAs matching any of the resources (union)
    let mut all_roas = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();

    // Query for each ASN
    for asn in &asns {
        let args = RpkiRoaLookupArgs {
            prefix: None,
            asn: Some(*asn),
            date: parsed_date,
            source: parse_data_source(&source),
            collector: parse_collector(&collector),
            format: monocle::lens::rpki::RpkiOutputFormat::Table,
        };

        match lens.get_roas(&args) {
            Ok(roas) => {
                for roa in roas {
                    let roa_json = serde_json::to_value(&roa).unwrap_or_default();
                    let key = format!(
                        "{}|{}|{}",
                        roa_json.get("asn").and_then(|v| v.as_u64()).unwrap_or(0),
                        roa_json
                            .get("prefix")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        roa_json
                            .get("max_length")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0)
                    );
                    if seen_keys.insert(key) {
                        all_roas.push(roa);
                    }
                }
            }
            Err(e) => {
                eprintln!("WARNING: Failed to get ROAs for ASN {}: {}", asn, e);
            }
        }
    }

    // Query for each prefix
    for prefix in &prefixes {
        let args = RpkiRoaLookupArgs {
            prefix: Some(prefix.clone()),
            asn: None,
            date: parsed_date,
            source: parse_data_source(&source),
            collector: parse_collector(&collector),
            format: monocle::lens::rpki::RpkiOutputFormat::Table,
        };

        match lens.get_roas(&args) {
            Ok(roas) => {
                for roa in roas {
                    let roa_json = serde_json::to_value(&roa).unwrap_or_default();
                    let key = format!(
                        "{}|{}|{}",
                        roa_json.get("asn").and_then(|v| v.as_u64()).unwrap_or(0),
                        roa_json
                            .get("prefix")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        roa_json
                            .get("max_length")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0)
                    );
                    if seen_keys.insert(key) {
                        all_roas.push(roa);
                    }
                }
            }
            Err(e) => {
                eprintln!("WARNING: Failed to get ROAs for prefix {}: {}", prefix, e);
            }
        }
    }

    output_roas(all_roas, output_format);
}

fn output_roas(roas: Vec<RpkiRoaEntry>, output_format: OutputFormat) {
    if roas.is_empty() {
        if output_format.is_json() {
            println!("[]");
        } else {
            println!("No ROAs found matching the criteria");
        }
        return;
    }

    eprintln!("Found {} ROAs", roas.len());

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
            println!("prefix|origin_asn|max_length|ta");
            for roa in &roas {
                println!(
                    "{}|{}|{}|{}",
                    roa.prefix, roa.origin_asn, roa.max_length, roa.ta
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

    // Display data source - current data always uses Cloudflare
    let source_display = match &date {
        Some(d) => format!(
            "Data source: {} (historical data from {})",
            source.to_uppercase(),
            d
        ),
        None => "Data source: CLOUDFLARE (current data)".to_string(),
    };
    eprintln!("{}", source_display);

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

    eprintln!("Found {} ASPAs", aspas.len());

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
            println!("{}", Table::new(table_entries).with(Style::markdown()));
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
