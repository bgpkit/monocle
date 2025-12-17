//! Inspect command - unified AS and prefix information lookup
//!
//! This command consolidates functionality from the former `whois`, `pfx2as`, and `as2rel` commands.

use clap::Args;
use monocle::database::MonocleDatabase;
use monocle::lens::inspect::{
    InspectDataSection, InspectDisplayConfig, InspectLens, InspectQueryOptions, InspectResult,
};
use monocle::lens::utils::OutputFormat;
use monocle::MonocleConfig;
use std::collections::HashSet;

/// Result of parsing --show options
struct ShowParseResult {
    /// Parsed sections
    sections: HashSet<InspectDataSection>,
    /// Invalid section names encountered
    invalid_sections: Vec<String>,
}

/// Arguments for the Inspect command
#[derive(Args)]
pub struct InspectArgs {
    /// One or more queries: ASN (13335, AS13335), prefix (1.1.1.0/24), IP (1.1.1.1), or name (cloudflare)
    #[clap(required_unless_present_any = ["country", "update"])]
    pub query: Vec<String>,

    // === Query Type Options ===
    /// Force treat queries as ASNs
    #[clap(short = 'a', long, conflicts_with_all = ["prefix", "name"])]
    pub asn: bool,

    /// Force treat queries as prefixes
    #[clap(short = 'p', long, conflicts_with_all = ["asn", "name"])]
    pub prefix: bool,

    /// Force treat queries as name search
    #[clap(short = 'n', long, conflicts_with_all = ["asn", "prefix"])]
    pub name: bool,

    /// Search by country code (e.g., US, DE)
    #[clap(short = 'c', long, conflicts_with_all = ["asn", "prefix", "name"])]
    pub country: Option<String>,

    // === Data Selection Options ===
    /// Select data sections to display (can be repeated). Overrides defaults.
    /// Available: basic (default), prefixes, connectivity, rpki, all
    #[clap(long = "show", value_name = "SECTION")]
    pub show: Vec<String>,

    // === Output Limit Options ===
    /// Show all data sections with no limits
    #[clap(long)]
    pub full: bool,

    /// Show all RPKI ROAs (default: top 10)
    #[clap(long)]
    pub full_roas: bool,

    /// Show all prefixes (default: top 10)
    #[clap(long)]
    pub full_prefixes: bool,

    /// Show all neighbors (default: top 5 per category)
    #[clap(long)]
    pub full_connectivity: bool,

    /// Limit search results (default: 20)
    #[clap(long, value_name = "N")]
    pub limit: Option<usize>,

    // === Data Options ===
    /// Force refresh the asinfo database
    #[clap(short = 'u', long)]
    pub update: bool,
}

pub fn run(config: &MonocleConfig, args: InspectArgs, output_format: OutputFormat) {
    let sqlite_path = config.sqlite_path();

    // Open the database
    let db = match MonocleDatabase::open(&sqlite_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    let lens = InspectLens::new(&db);

    // Handle explicit update request (force refresh all)
    if args.update {
        eprintln!("Updating all data sources...");
        match lens.ensure_data_available() {
            Ok(summary) => {
                for msg in summary.format_messages() {
                    eprintln!("{}", msg);
                }
                if !summary.any_refreshed {
                    eprintln!("All data sources are up to date.");
                }
            }
            Err(e) => {
                eprintln!("Failed to update data: {}", e);
                std::process::exit(1);
            }
        }

        // If no query provided after update, just exit
        if args.query.is_empty() && args.country.is_none() {
            return;
        }
    }

    // Ensure all required data is available (auto-refresh if empty or expired)
    match lens.ensure_data_available() {
        Ok(summary) => {
            // Print messages about any data that was refreshed
            for msg in summary.format_messages() {
                eprintln!("{}", msg);
            }
        }
        Err(e) => {
            eprintln!("Warning: Could not verify data availability: {}", e);
            // Continue anyway - some data sources may still work
        }
    }

    // Build query options
    let (options, select_result) = build_query_options(&args);

    // Execute query
    let result = if let Some(ref country) = args.country {
        // Country search
        match lens.query_by_country(country, &options) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Query failed: {}", e);
                std::process::exit(1);
            }
        }
    } else if args.asn {
        // Force ASN query
        match lens.query_as_asn(&args.query, &options) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Query failed: {}", e);
                std::process::exit(1);
            }
        }
    } else if args.prefix {
        // Force prefix query
        match lens.query_as_prefix(&args.query, &options) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Query failed: {}", e);
                std::process::exit(1);
            }
        }
    } else if args.name {
        // Force name query
        match lens.query_as_name(&args.query, &options) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Query failed: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Auto-detect query types
        match lens.query(&args.query, &options) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Query failed: {}", e);
                std::process::exit(1);
            }
        }
    };

    // Format and output results
    output_results(&lens, &result, output_format, &select_result);
}

/// Parse --show options and return parsed result
fn parse_show_options(args: &InspectArgs) -> ShowParseResult {
    let mut sections = HashSet::new();
    let mut invalid_sections = Vec::new();

    for s in &args.show {
        let s_lower = s.to_lowercase();
        match s_lower.as_str() {
            "all" => {
                sections.extend(InspectDataSection::all());
            }
            _ => {
                if let Some(section) = InspectDataSection::from_str(&s_lower) {
                    sections.insert(section);
                } else {
                    invalid_sections.push(s.clone());
                }
            }
        }
    }

    ShowParseResult {
        sections,
        invalid_sections,
    }
}

/// Build query options from CLI arguments
fn build_query_options(args: &InspectArgs) -> (InspectQueryOptions, ShowParseResult) {
    let mut options = if args.full {
        InspectQueryOptions::full()
    } else {
        InspectQueryOptions::default()
    };

    // Parse --show options
    let show_result = parse_show_options(args);

    // Validate show options - exit with error if any invalid sections
    if !show_result.invalid_sections.is_empty() {
        eprintln!(
            "Error: Unknown section(s): {}",
            show_result.invalid_sections.join(", ")
        );
        eprintln!(
            "Available sections: {}, all",
            InspectDataSection::all_names().join(", ")
        );
        std::process::exit(1);
    }

    if !show_result.sections.is_empty() {
        options.select = Some(show_result.sections.clone());
    }

    // Apply individual expansion flags
    if args.full_roas {
        options.max_roas = 0;
    }

    if args.full_prefixes {
        options.max_prefixes = 0;
    }

    if args.full_connectivity {
        options.max_neighbors = 0;
    }

    if let Some(limit) = args.limit {
        options.max_search_results = limit;
    }

    (options, show_result)
}

/// Output results in the appropriate format
fn output_results(
    lens: &InspectLens,
    result: &InspectResult,
    format: OutputFormat,
    _show_result: &ShowParseResult,
) {
    // Determine display config
    let config = InspectDisplayConfig::auto();

    match format {
        OutputFormat::Json => {
            println!("{}", lens.format_json(result, false));
        }
        OutputFormat::JsonPretty => {
            println!("{}", lens.format_json(result, true));
        }
        OutputFormat::JsonLine => {
            // Output each query result as a separate JSON line
            for query_result in &result.queries {
                if let Ok(json) = serde_json::to_string(query_result) {
                    println!("{}", json);
                }
            }
        }
        OutputFormat::Markdown => {
            let config = config.with_markdown(true);
            println!("{}", lens.format_table(result, &config));
        }
        OutputFormat::Psv => {
            eprintln!("PSV format is not supported for inspect command. Use --format json or --format table.");
            std::process::exit(1);
        }
        OutputFormat::Table => {
            println!("{}", lens.format_table(result, &config));
        }
    }
}
