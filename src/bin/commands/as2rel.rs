use clap::Args;
use monocle::database::MonocleDatabase;
use monocle::lens::as2rel::{As2relLens, As2relSearchArgs};
use monocle::lens::utils::{truncate_name, OutputFormat, DEFAULT_NAME_MAX_LEN};
use monocle::MonocleConfig;
use serde::Serialize;
use serde_json::json;
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the As2rel command
#[derive(Args)]
pub struct As2relArgs {
    /// One or more ASNs to query relationships for
    ///
    /// - Single ASN: shows all relationships for that ASN
    /// - Two ASNs: shows the relationship between them
    /// - Multiple ASNs: shows relationships for all pairs (asn1 < asn2)
    #[clap(required = true)]
    pub asns: Vec<u32>,

    /// Force update the local as2rel database
    #[clap(short, long)]
    pub update: bool,

    /// Update with a custom data file (local path or URL)
    #[clap(long)]
    pub update_with: Option<String>,

    /// Hide the explanation text
    #[clap(long)]
    pub no_explain: bool,

    /// Sort by ASN2 ascending instead of connected percentage descending
    #[clap(long)]
    pub sort_by_asn: bool,

    /// Show organization name for ASN2 (from asinfo database)
    #[clap(long)]
    pub show_name: bool,

    /// Show full organization name without truncation (default truncates to 20 chars)
    #[clap(long)]
    pub show_full_name: bool,

    /// Minimum visibility percentage (0-100) to include in results
    ///
    /// Filters out relationships seen by fewer than this percentage of peers.
    #[clap(long, value_name = "PERCENT")]
    pub min_visibility: Option<f32>,

    /// Only show ASNs that are single-homed to the queried ASN
    ///
    /// An ASN is single-homed if it has exactly one upstream provider.
    /// This finds ASNs where the queried ASN is their ONLY upstream.
    ///
    /// Only applicable when querying a single ASN.
    #[clap(long)]
    pub single_homed: bool,

    /// Only show relationships where the queried ASN is an upstream (provider)
    ///
    /// Shows the downstream customers of the queried ASN.
    /// Only applicable when querying a single ASN.
    #[clap(long, conflicts_with_all = ["is_downstream", "is_peer"])]
    pub is_upstream: bool,

    /// Only show relationships where the queried ASN is a downstream (customer)
    ///
    /// Shows the upstream providers of the queried ASN.
    /// Only applicable when querying a single ASN.
    #[clap(long, conflicts_with_all = ["is_upstream", "is_peer"])]
    pub is_downstream: bool,

    /// Only show peer relationships
    ///
    /// Only applicable when querying a single ASN.
    #[clap(long, conflicts_with_all = ["is_upstream", "is_downstream"])]
    pub is_peer: bool,
}

pub fn run(
    config: &MonocleConfig,
    args: As2relArgs,
    output_format: OutputFormat,
    no_refresh: bool,
) {
    let As2relArgs {
        asns,
        update,
        update_with,
        no_explain,
        sort_by_asn,
        show_name,
        show_full_name,
        min_visibility,
        single_homed,
        is_upstream,
        is_downstream,
        is_peer,
    } = args;

    // show_full_name implies show_name
    let show_name = show_name || show_full_name;

    // Validate ASN count
    if asns.is_empty() {
        eprintln!("ERROR: Please provide at least one ASN");
        std::process::exit(1);
    }

    // Validate single-ASN-only flags
    if asns.len() != 1 {
        if single_homed {
            eprintln!("ERROR: --single-homed can only be used with a single ASN");
            std::process::exit(1);
        }
        if is_upstream || is_downstream || is_peer {
            eprintln!(
                "ERROR: --is-upstream, --is-downstream, and --is-peer can only be used with a single ASN"
            );
            std::process::exit(1);
        }
    }

    // Validate min_visibility range
    if let Some(min_vis) = min_visibility {
        if !(0.0..=100.0).contains(&min_vis) {
            eprintln!("ERROR: --min-visibility must be between 0 and 100");
            std::process::exit(1);
        }
    }

    let sqlite_path = config.sqlite_path();

    // Handle explicit updates
    if update || update_with.is_some() {
        if no_refresh {
            eprintln!("[monocle] Warning: --update ignored because --no-refresh is set");
        } else {
            eprintln!("[monocle] Updating AS2rel data...");

            let db = match MonocleDatabase::open(&sqlite_path) {
                Ok(db) => db,
                Err(e) => {
                    eprintln!("Failed to open database: {}", e);
                    std::process::exit(1);
                }
            };

            let lens = As2relLens::new(&db);
            let result = match &update_with {
                Some(path) => lens.update_from(path),
                None => lens.update(),
            };

            match result {
                Ok(count) => {
                    eprintln!(
                        "[monocle] AS2rel data updated: {} relationships loaded",
                        count
                    );
                }
                Err(e) => {
                    eprintln!("[monocle] Failed to update AS2rel data: {}", e);
                    std::process::exit(1);
                }
            }

            // Continue with query using the same connection
            run_query(
                &db,
                &asns,
                sort_by_asn,
                show_name,
                show_full_name,
                no_explain,
                output_format,
                min_visibility,
                single_homed,
                is_upstream,
                is_downstream,
                is_peer,
            );
            return;
        }
    }

    // Open the database
    let db = match MonocleDatabase::open(&sqlite_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    let lens = As2relLens::new(&db);

    // Check if data needs to be initialized or updated automatically
    if let Some(reason) = lens.update_reason() {
        if no_refresh {
            eprintln!(
                "[monocle] Warning: AS2rel {} Results may be incomplete.",
                reason
            );
            eprintln!("[monocle]          Run without --no-refresh or use 'monocle config db-refresh --as2rel' to load data.");
        } else {
            eprintln!("[monocle] AS2rel {}, updating now...", reason);

            match lens.update() {
                Ok(count) => {
                    eprintln!(
                        "[monocle] AS2rel data updated: {} relationships loaded",
                        count
                    );
                }
                Err(e) => {
                    eprintln!("[monocle] Failed to update AS2rel data: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    // Run query
    run_query(
        &db,
        &asns,
        sort_by_asn,
        show_name,
        show_full_name,
        no_explain,
        output_format,
        min_visibility,
        single_homed,
        is_upstream,
        is_downstream,
        is_peer,
    );
}

#[derive(Debug, Clone, Serialize, tabled::Tabled)]
struct As2relResult {
    asn1: u32,
    asn2: u32,
    connected: String,
    peer: String,
    as1_upstream: String,
    as2_upstream: String,
}

#[derive(Debug, Clone, Serialize, tabled::Tabled)]
struct As2relResultWithName {
    asn1: u32,
    asn2: u32,
    asn2_name: String,
    connected: String,
    peer: String,
    as1_upstream: String,
    as2_upstream: String,
}

#[allow(clippy::too_many_arguments)]
fn run_query(
    db: &MonocleDatabase,
    asns: &[u32],
    sort_by_asn: bool,
    show_name: bool,
    show_full_name: bool,
    no_explain: bool,
    output_format: OutputFormat,
    min_visibility: Option<f32>,
    single_homed: bool,
    is_upstream: bool,
    is_downstream: bool,
    is_peer: bool,
) {
    let lens = As2relLens::new(db);

    // Build search args
    let search_args = As2relSearchArgs {
        asns: asns.to_vec(),
        sort_by_asn,
        show_name,
        no_explain,
        min_visibility,
        single_homed,
        is_upstream,
        is_downstream,
        is_peer,
    };

    // Validate
    if let Err(e) = search_args.validate() {
        eprintln!("ERROR: {}", e);
        std::process::exit(1);
    }

    // Perform search
    let results = match lens.search(&search_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error searching for AS relationships: {}", e);
            std::process::exit(1);
        }
    };

    // Handle empty results
    if results.is_empty() {
        if output_format.is_json() {
            println!("[]");
        } else if single_homed {
            println!(
                "No single-homed ASNs found for AS{} (with the current filters)",
                asns[0]
            );
        } else if asns.len() == 1 {
            let filter_msg = if is_upstream {
                " with --is-upstream filter"
            } else if is_downstream {
                " with --is-downstream filter"
            } else if is_peer {
                " with --is-peer filter"
            } else {
                ""
            };
            println!("No relationships found for ASN {}{}", asns[0], filter_msg);
        } else if asns.len() == 2 {
            println!(
                "No relationship found between ASN {} and ASN {}",
                asns[0], asns[1]
            );
        } else {
            println!("No relationships found among the provided ASNs");
        }
        return;
    }

    // Print explanation to stderr unless --no-explain is set or JSON output
    if !no_explain && !output_format.is_json() {
        if single_homed {
            eprintln!("{}", lens.get_single_homed_explanation(asns[0]));
        } else {
            eprintln!("{}", lens.get_explanation());
        }
    }

    // Truncate names for table output unless show_full_name is set
    let truncate_names = !show_full_name && output_format.is_table();
    let max_peers = lens.get_max_peers_count();

    // Format and print results based on output format
    match output_format {
        OutputFormat::Table => {
            if show_name {
                let display: Vec<As2relResultWithName> = results
                    .iter()
                    .map(|r| As2relResultWithName {
                        asn1: r.asn1,
                        asn2: r.asn2,
                        asn2_name: format_name(&r.asn2_name, truncate_names),
                        connected: r.connected.clone(),
                        peer: r.peer.clone(),
                        as1_upstream: r.as1_upstream.clone(),
                        as2_upstream: r.as2_upstream.clone(),
                    })
                    .collect();
                println!("{}", Table::new(display).with(Style::rounded()));
            } else {
                let display: Vec<As2relResult> = results
                    .iter()
                    .map(|r| As2relResult {
                        asn1: r.asn1,
                        asn2: r.asn2,
                        connected: r.connected.clone(),
                        peer: r.peer.clone(),
                        as1_upstream: r.as1_upstream.clone(),
                        as2_upstream: r.as2_upstream.clone(),
                    })
                    .collect();
                println!("{}", Table::new(display).with(Style::rounded()));
            }
        }
        OutputFormat::Markdown => {
            if show_name {
                let display: Vec<As2relResultWithName> = results
                    .iter()
                    .map(|r| As2relResultWithName {
                        asn1: r.asn1,
                        asn2: r.asn2,
                        asn2_name: format_name(&r.asn2_name, truncate_names),
                        connected: r.connected.clone(),
                        peer: r.peer.clone(),
                        as1_upstream: r.as1_upstream.clone(),
                        as2_upstream: r.as2_upstream.clone(),
                    })
                    .collect();
                println!("{}", Table::new(display).with(Style::markdown()));
            } else {
                let display: Vec<As2relResult> = results
                    .iter()
                    .map(|r| As2relResult {
                        asn1: r.asn1,
                        asn2: r.asn2,
                        connected: r.connected.clone(),
                        peer: r.peer.clone(),
                        as1_upstream: r.as1_upstream.clone(),
                        as2_upstream: r.as2_upstream.clone(),
                    })
                    .collect();
                println!("{}", Table::new(display).with(Style::markdown()));
            }
        }
        OutputFormat::Json => {
            let output = build_json_output(&results, show_name, max_peers);
            match serde_json::to_string(&output) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonPretty => {
            let output = build_json_output(&results, show_name, max_peers);
            match serde_json::to_string_pretty(&output) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonLine => {
            for r in &results {
                let obj = if show_name {
                    json!({
                        "asn1": r.asn1,
                        "asn2": r.asn2,
                        "asn2_name": r.asn2_name.as_deref().unwrap_or(""),
                        "connected": &r.connected,
                        "peer": &r.peer,
                        "as1_upstream": &r.as1_upstream,
                        "as2_upstream": &r.as2_upstream,
                    })
                } else {
                    json!({
                        "asn1": r.asn1,
                        "asn2": r.asn2,
                        "connected": &r.connected,
                        "peer": &r.peer,
                        "as1_upstream": &r.as1_upstream,
                        "as2_upstream": &r.as2_upstream,
                    })
                };
                match serde_json::to_string(&obj) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
                }
            }
        }
        OutputFormat::Psv => {
            if show_name {
                println!("asn1|asn2|asn2_name|connected|peer|as1_upstream|as2_upstream");
                for r in &results {
                    println!(
                        "{}|{}|{}|{}|{}|{}|{}",
                        r.asn1,
                        r.asn2,
                        r.asn2_name.as_deref().unwrap_or(""),
                        r.connected,
                        r.peer,
                        r.as1_upstream,
                        r.as2_upstream
                    );
                }
            } else {
                println!("asn1|asn2|connected|peer|as1_upstream|as2_upstream");
                for r in &results {
                    println!(
                        "{}|{}|{}|{}|{}|{}",
                        r.asn1, r.asn2, r.connected, r.peer, r.as1_upstream, r.as2_upstream
                    );
                }
            }
        }
    }
}

fn format_name(name: &Option<String>, truncate: bool) -> String {
    let name = name.as_deref().unwrap_or("");
    if truncate {
        truncate_name(name, DEFAULT_NAME_MAX_LEN)
    } else {
        name.to_string()
    }
}

fn build_json_output(
    results: &[monocle::lens::as2rel::As2relSearchResult],
    show_name: bool,
    max_peers: u32,
) -> serde_json::Value {
    let json_results: Vec<_> = results
        .iter()
        .map(|r| {
            if show_name {
                json!({
                    "asn1": r.asn1,
                    "asn2": r.asn2,
                    "asn2_name": r.asn2_name.as_deref().unwrap_or(""),
                    "connected": &r.connected,
                    "peer": &r.peer,
                    "as1_upstream": &r.as1_upstream,
                    "as2_upstream": &r.as2_upstream,
                })
            } else {
                json!({
                    "asn1": r.asn1,
                    "asn2": r.asn2,
                    "connected": &r.connected,
                    "peer": &r.peer,
                    "as1_upstream": &r.as1_upstream,
                    "as2_upstream": &r.as2_upstream,
                })
            }
        })
        .collect();

    json!({
        "max_peers_count": max_peers,
        "results": json_results,
    })
}
