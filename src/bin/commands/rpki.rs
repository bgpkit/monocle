use chrono::NaiveDate;
use clap::Subcommand;
use monocle::database::{MonocleDatabase, RpkiRoaRecord};
use monocle::lens::rpki::{
    RpkiAspaLookupArgs, RpkiAspaTableEntry, RpkiDataSource, RpkiLens, RpkiRoaEntry,
    RpkiRoaLookupArgs, RpkiViewsCollectorOption,
};
use monocle::lens::utils::OutputFormat;
use std::collections::HashSet;
use tabled::settings::object::Columns;
use tabled::settings::width::Width;
use tabled::settings::Style;
use tabled::Table;

#[derive(Subcommand)]
pub enum RpkiCommands {
    /// validate a prefix-asn pair using cached RPKI data
    Validate {
        /// Two resources: one prefix and one ASN (order does not matter)
        #[clap(num_args = 2)]
        resources: Vec<String>,

        /// Force refresh the RPKI cache before validation
        #[clap(long, short)]
        refresh: bool,
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

        /// Force refresh the RPKI cache (only applies to current data)
        #[clap(long, short)]
        refresh: bool,
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

        /// Force refresh the RPKI cache (only applies to current data)
        #[clap(long, short)]
        refresh: bool,
    },
}

pub fn run(commands: RpkiCommands, output_format: OutputFormat, data_dir: &str) {
    match commands {
        RpkiCommands::Validate { resources, refresh } => {
            run_validate(resources, refresh, output_format, data_dir)
        }
        RpkiCommands::Roas {
            resources,
            date,
            source,
            collector,
            refresh,
        } => run_roas(
            resources,
            date,
            source,
            collector,
            refresh,
            output_format,
            data_dir,
        ),
        RpkiCommands::Aspas {
            customer,
            provider,
            date,
            source,
            collector,
            refresh,
        } => run_aspas(
            customer,
            provider,
            date,
            source,
            collector,
            refresh,
            output_format,
            data_dir,
        ),
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

/// Ensure RPKI cache is populated (refresh if needed or forced)
fn ensure_rpki_cache(
    lens: &RpkiLens,
    force_refresh: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let needs_refresh = force_refresh
        || lens
            .needs_refresh()
            .map_err(|e| format!("Failed to check cache: {}", e))?;

    if needs_refresh {
        eprintln!("Refreshing RPKI cache from Cloudflare...");

        let (roa_count, aspa_count) = lens
            .refresh()
            .map_err(|e| format!("Failed to refresh cache: {}", e))?;

        eprintln!("Cached {} ROAs and {} ASPAs", roa_count, aspa_count);
    }

    Ok(())
}

fn run_validate(
    resources: Vec<String>,
    refresh: bool,
    output_format: OutputFormat,
    data_dir: &str,
) {
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

    // Open database and create lens
    let db = match MonocleDatabase::open_in_dir(data_dir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("ERROR: Failed to open database: {}", e);
            return;
        }
    };

    let lens = RpkiLens::new(&db);
    if let Err(e) = ensure_rpki_cache(&lens, refresh) {
        eprintln!("ERROR: Failed to refresh RPKI cache: {}", e);
        return;
    }

    // Display data source
    if let Ok(Some(meta)) = lens.get_metadata() {
        eprintln!(
            "Data source: CLOUDFLARE (cached at {}, {} ROAs)",
            meta.updated_at.format("%Y-%m-%d %H:%M:%S UTC"),
            meta.roa_count
        );
    }

    // Perform validation using lens (policy logic now in lens layer)
    let result = match lens.validate(&prefix, asn) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: Validation failed: {}", e);
            return;
        }
    };

    match output_format {
        OutputFormat::Table => {
            #[derive(tabled::Tabled)]
            struct ValidationRow {
                prefix: String,
                asn: u32,
                state: String,
                reason: String,
            }

            let row = ValidationRow {
                prefix: result.prefix.clone(),
                asn: result.asn,
                state: result.state.to_string(),
                reason: result.reason.clone(),
            };

            let mut output = Table::new(vec![row]).with(Style::rounded()).to_string();

            if !result.covering_roas.is_empty() {
                output.push_str("\n\nCovering ROAs:\n");
                output.push_str(
                    &Table::new(&result.covering_roas)
                        .with(Style::rounded())
                        .to_string(),
                );
            }
            println!("{}", output);
        }
        OutputFormat::Markdown => {
            #[derive(tabled::Tabled)]
            struct ValidationRow {
                prefix: String,
                asn: u32,
                state: String,
                reason: String,
            }

            let row = ValidationRow {
                prefix: result.prefix.clone(),
                asn: result.asn,
                state: result.state.to_string(),
                reason: result.reason.clone(),
            };

            let mut output = Table::new(vec![row]).with(Style::markdown()).to_string();

            if !result.covering_roas.is_empty() {
                output.push_str("\n\nCovering ROAs:\n");
                output.push_str(
                    &Table::new(&result.covering_roas)
                        .with(Style::markdown())
                        .to_string(),
                );
            }
            println!("{}", output);
        }
        OutputFormat::Json => match serde_json::to_string(&result) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonPretty => match serde_json::to_string_pretty(&result) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::JsonLine => match serde_json::to_string(&result) {
            Ok(json) => println!("{}", json),
            Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
        },
        OutputFormat::Psv => {
            println!("prefix|asn|state|reason");
            println!(
                "{}|{}|{}|{}",
                result.prefix, result.asn, result.state, result.reason
            );
            if !result.covering_roas.is_empty() {
                eprintln!("\nCovering ROAs:");
                println!("prefix|origin_asn|max_length|ta");
                for roa in &result.covering_roas {
                    println!(
                        "{}|{}|{}|{}",
                        roa.prefix, roa.origin_asn, roa.max_length, roa.ta
                    );
                }
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
    refresh: bool,
    output_format: OutputFormat,
    data_dir: &str,
) {
    // Parse date if provided
    let (parsed_date, date_str) = match &date {
        Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
            Ok(parsed) => (parsed, d.clone()),
            Err(e) => {
                eprintln!("ERROR: Invalid date format '{}': {}. Use YYYY-MM-DD", d, e);
                return;
            }
        },
        None => {
            // For current data (no date), use SQLite cache
            run_roas_from_cache(resources, refresh, output_format, data_dir);
            return;
        }
    };

    // For historical data, use the lens directly
    // Display data source
    let source_display = format!(
        "Data source: {} (historical data from {})",
        source.to_uppercase(),
        date_str
    );
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

    // For historical data, we need a database reference but won't use its cache
    let db = match MonocleDatabase::open_in_dir(data_dir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("ERROR: Failed to open database: {}", e);
            return;
        }
    };

    let mut lens = RpkiLens::new(&db);

    // If no resources specified, get all ROAs
    if asns.is_empty() && prefixes.is_empty() {
        let args = RpkiRoaLookupArgs::new()
            .with_date(parsed_date)
            .with_source(parse_data_source(&source));

        let args = if let Some(c) = parse_collector(&collector) {
            RpkiRoaLookupArgs {
                collector: Some(c),
                ..args
            }
        } else {
            args
        };

        let roas = match lens.get_roas(&args) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ERROR: Failed to get ROAs: {}", e);
                return;
            }
        };

        output_roas_entries(roas, output_format);
        return;
    }

    // Collect all ROAs matching any of the resources (union)
    let mut all_roas = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();

    // Query for each ASN
    for asn in &asns {
        let args = RpkiRoaLookupArgs::new()
            .with_asn(*asn)
            .with_date(parsed_date)
            .with_source(parse_data_source(&source));

        let args = if let Some(c) = parse_collector(&collector) {
            RpkiRoaLookupArgs {
                collector: Some(c),
                ..args
            }
        } else {
            args
        };

        match lens.get_roas(&args) {
            Ok(roas) => {
                for roa in roas {
                    let key = format!("{}|{}|{}", roa.origin_asn, roa.prefix, roa.max_length);
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
        let args = RpkiRoaLookupArgs::new()
            .with_prefix(prefix)
            .with_date(parsed_date)
            .with_source(parse_data_source(&source));

        let args = if let Some(c) = parse_collector(&collector) {
            RpkiRoaLookupArgs {
                collector: Some(c),
                ..args
            }
        } else {
            args
        };

        match lens.get_roas(&args) {
            Ok(roas) => {
                for roa in roas {
                    let key = format!("{}|{}|{}", roa.origin_asn, roa.prefix, roa.max_length);
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

    output_roas_entries(all_roas, output_format);
}

fn run_roas_from_cache(
    resources: Vec<String>,
    refresh: bool,
    output_format: OutputFormat,
    data_dir: &str,
) {
    // Open database and create lens
    let db = match MonocleDatabase::open_in_dir(data_dir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("ERROR: Failed to open database: {}", e);
            return;
        }
    };

    let lens = RpkiLens::new(&db);
    if let Err(e) = ensure_rpki_cache(&lens, refresh) {
        eprintln!("ERROR: Failed to refresh RPKI cache: {}", e);
        return;
    }

    // Display data source
    if let Ok(Some(meta)) = lens.get_metadata() {
        eprintln!(
            "Data source: CLOUDFLARE (cached at {}, {} ROAs)",
            meta.updated_at.format("%Y-%m-%d %H:%M:%S UTC"),
            meta.roa_count
        );
    }

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

    // If no resources specified, get all ROAs
    if asns.is_empty() && prefixes.is_empty() {
        let roas = match db.rpki().get_all_roas() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("ERROR: Failed to get ROAs: {}", e);
                return;
            }
        };

        output_roas_records(roas, output_format);
        return;
    }

    // Collect all ROAs matching any of the resources (union)
    let mut all_roas = Vec::new();
    let mut seen_keys: HashSet<String> = HashSet::new();

    // Query for each ASN
    for asn in &asns {
        match lens.get_roas_by_asn(*asn) {
            Ok(roas) => {
                for roa in roas {
                    let key = format!("{}|{}|{}", roa.origin_asn, roa.prefix, roa.max_length);
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

    // Query for each prefix (get covering ROAs)
    for prefix in &prefixes {
        match lens.get_covering_roas(prefix) {
            Ok(roas) => {
                for roa in roas {
                    let key = format!("{}|{}|{}", roa.origin_asn, roa.prefix, roa.max_length);
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

    output_roas_records_from_lens(all_roas, output_format);
}

fn output_roas_entries(roas: Vec<RpkiRoaEntry>, output_format: OutputFormat) {
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

fn output_roas_records_from_lens(
    roas: Vec<monocle::lens::rpki::RpkiRoaRecord>,
    output_format: OutputFormat,
) {
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

fn output_roas_records(roas: Vec<RpkiRoaRecord>, output_format: OutputFormat) {
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

#[allow(clippy::too_many_arguments)]
fn run_aspas(
    customer: Option<u32>,
    provider: Option<u32>,
    date: Option<String>,
    source: String,
    collector: String,
    refresh: bool,
    output_format: OutputFormat,
    data_dir: &str,
) {
    // Parse date if provided
    let (parsed_date, date_str) = match &date {
        Some(d) => match NaiveDate::parse_from_str(d, "%Y-%m-%d") {
            Ok(parsed) => (parsed, d.clone()),
            Err(e) => {
                eprintln!("ERROR: Invalid date format '{}': {}. Use YYYY-MM-DD", d, e);
                return;
            }
        },
        None => {
            // For current data (no date), use SQLite cache
            run_aspas_from_cache(customer, provider, refresh, output_format, data_dir);
            return;
        }
    };

    // For historical data, use the lens directly
    // Display data source
    let source_display = format!(
        "Data source: {} (historical data from {})",
        source.to_uppercase(),
        date_str
    );
    eprintln!("{}", source_display);

    // For historical data, we need a database reference but won't use its cache
    let db = match MonocleDatabase::open_in_dir(data_dir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("ERROR: Failed to open database: {}", e);
            return;
        }
    };

    let mut lens = RpkiLens::new(&db);
    let mut args = RpkiAspaLookupArgs::new();

    if let Some(c) = customer {
        args = args.with_customer(c);
    }
    if let Some(p) = provider {
        args = args.with_provider(p);
    }

    // Set date and source
    args = RpkiAspaLookupArgs {
        date: Some(parsed_date),
        source: parse_data_source(&source),
        collector: parse_collector(&collector),
        ..args
    };

    let aspas = match lens.get_aspas(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: Failed to get ASPAs: {}", e);
            return;
        }
    };

    output_aspas_entries(aspas, output_format);
}

fn run_aspas_from_cache(
    customer: Option<u32>,
    provider: Option<u32>,
    refresh: bool,
    output_format: OutputFormat,
    data_dir: &str,
) {
    // Open database and create lens
    let db = match MonocleDatabase::open_in_dir(data_dir) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("ERROR: Failed to open database: {}", e);
            return;
        }
    };

    let lens = RpkiLens::new(&db);
    if let Err(e) = ensure_rpki_cache(&lens, refresh) {
        eprintln!("ERROR: Failed to refresh RPKI cache: {}", e);
        return;
    }

    // Display data source
    if let Ok(Some(meta)) = lens.get_metadata() {
        eprintln!(
            "Data source: CLOUDFLARE (cached at {}, {} ASPAs)",
            meta.updated_at.format("%Y-%m-%d %H:%M:%S UTC"),
            meta.aspa_count
        );
    }

    // Use lens methods for cache queries
    let args = RpkiAspaLookupArgs {
        customer_asn: customer,
        provider_asn: provider,
        date: None,
        source: RpkiDataSource::Cloudflare,
        collector: None,
        format: monocle::lens::rpki::RpkiOutputFormat::Table,
    };

    let mut lens_mut = RpkiLens::new(&db);
    let aspas = match lens_mut.get_aspas(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ERROR: Failed to get ASPAs: {}", e);
            return;
        }
    };

    output_aspas_entries(aspas, output_format);
}

fn output_aspas_entries(
    aspas: Vec<monocle::lens::rpki::RpkiAspaEntry>,
    output_format: OutputFormat,
) {
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
                let providers = aspa
                    .providers
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                println!("{}|{}", aspa.customer_asn, providers);
            }
        }
    }
}
