//! Prefix-to-ASN (pfx2as) command
//!
//! This command provides prefix-to-ASN mapping lookups.
//! All business logic is delegated to `Pfx2asLens`.

use clap::Args;
use monocle::database::MonocleDatabase;
use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asSearchArgs};
use monocle::lens::rpki::RpkiLens;
use monocle::lens::utils::OutputFormat;
use monocle::MonocleConfig;

/// Arguments for the Pfx2as command
#[derive(Args)]
pub struct Pfx2asArgs {
    /// Query: an IP prefix (e.g., 1.1.1.0/24) or ASN (e.g., 13335, AS13335)
    #[clap(required = true)]
    pub query: String,

    /// Force update the local pfx2as database
    #[clap(short, long)]
    pub update: bool,

    /// Include sub-prefixes (more specific) in results when querying by prefix
    #[clap(long)]
    pub include_sub: bool,

    /// Include super-prefixes (less specific) in results when querying by prefix
    #[clap(long)]
    pub include_super: bool,

    /// Show AS name for each origin ASN
    #[clap(long)]
    pub show_name: bool,

    /// Show full AS name without truncation (default truncates to 20 chars)
    #[clap(long)]
    pub show_full_name: bool,

    /// Limit the number of results (default: no limit)
    #[clap(long, short, value_name = "N")]
    pub limit: Option<usize>,
}

impl From<&Pfx2asArgs> for Pfx2asSearchArgs {
    fn from(args: &Pfx2asArgs) -> Self {
        let mut search_args = Pfx2asSearchArgs::new(&args.query)
            .with_include_sub(args.include_sub)
            .with_include_super(args.include_super)
            .with_show_name(args.show_name)
            .with_show_full_name(args.show_full_name);

        if let Some(limit) = args.limit {
            search_args = search_args.with_limit(limit);
        }

        search_args
    }
}

pub fn run(config: &MonocleConfig, args: Pfx2asArgs, output_format: OutputFormat, no_update: bool) {
    let sqlite_path = config.sqlite_path();

    // Open the database
    let db = match MonocleDatabase::open(&sqlite_path) {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    let lens = Pfx2asLens::new(&db);

    // Handle explicit updates
    if args.update {
        if no_update {
            eprintln!("[monocle] Warning: --update ignored because --no-update is set");
        } else {
            eprintln!("[monocle] Updating pfx2as data...");

            match lens.refresh(None) {
                Ok(count) => {
                    eprintln!("[monocle] Pfx2as data updated: {} records loaded", count);
                }
                Err(e) => {
                    eprintln!("[monocle] Failed to update pfx2as data: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    // Check if pfx2as data needs refresh
    if !no_update {
        match lens.refresh_reason(config.pfx2as_cache_ttl()) {
            Ok(Some(reason)) => {
                eprintln!("[monocle] Pfx2as {}, updating now...", reason);
                match lens.refresh(None) {
                    Ok(count) => {
                        eprintln!("[monocle] Pfx2as data updated: {} records loaded", count);
                    }
                    Err(e) => {
                        eprintln!("[monocle] Failed to update pfx2as data: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!(
                    "[monocle] Warning: Could not check pfx2as data status: {}",
                    e
                );
            }
        }

        // Also ensure RPKI data is available for validation
        let rpki_lens = RpkiLens::new(&db);
        if let Ok(Some(reason)) = rpki_lens.refresh_reason(config.rpki_cache_ttl()) {
            eprintln!("[monocle] RPKI {}, updating for validation...", reason);
            match rpki_lens.refresh() {
                Ok((roa_count, aspa_count)) => {
                    eprintln!(
                        "[monocle] RPKI data updated: {} ROAs, {} ASPAs",
                        roa_count, aspa_count
                    );
                }
                Err(e) => {
                    eprintln!("[monocle] Warning: Failed to update RPKI data: {}", e);
                }
            }
        }
    }

    // Convert CLI args to lens search args
    let search_args = Pfx2asSearchArgs::from(&args);
    let show_name = args.show_name || args.show_full_name;

    // Perform search
    let results = match lens.search(&search_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error searching pfx2as data: {}", e);
            std::process::exit(1);
        }
    };

    // Handle empty results
    if results.is_empty() {
        if output_format.is_json() {
            println!("[]");
        } else {
            println!("No results found for query: {}", args.query);
        }
        return;
    }

    // Format and output results
    println!(
        "{}",
        lens.format_search_results(&results, &output_format, show_name)
    );
}
