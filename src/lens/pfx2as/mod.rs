//! Prefix-to-ASN mapping lens module
//!
//! This module provides the `Pfx2asLens` for prefix-to-ASN mapping operations.
//! The lens wraps `Pfx2asRepository` and provides:
//! - Prefix lookup (exact, longest, covering, covered)
//! - ASN-to-prefixes lookup
//! - Search with RPKI validation and AS name enrichment
//! - Cache management (refresh, needs_refresh)
//! - Output formatting
//!
//! # Example
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//! use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asSearchArgs};
//!
//! let db = MonocleDatabase::open()?;
//! let lens = Pfx2asLens::new(&db);
//!
//! // Ensure cache is populated
//! if lens.needs_refresh()? {
//!     lens.refresh(None)?;
//! }
//!
//! // Search by prefix
//! let args = Pfx2asSearchArgs::new("1.1.1.0/24");
//! let results = lens.search(&args)?;
//!
//! // Search by ASN
//! let args = Pfx2asSearchArgs::new("13335");
//! let results = lens.search(&args)?;
//!
//! // Search with options
//! let args = Pfx2asSearchArgs::new("8.8.0.0/16")
//!     .with_include_sub(true)
//!     .with_show_name(true);
//! let results = lens.search(&args)?;
//! ```

use crate::database::MonocleDatabase;
use crate::lens::rpki::RpkiLens;
use crate::lens::utils::{truncate_name, OutputFormat, DEFAULT_NAME_MAX_LEN};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tabled::Tabled;

// =============================================================================
// Types
// =============================================================================

/// A prefix-to-ASN mapping entry (from BGPKIT data source)
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct Pfx2asEntry {
    /// Origin ASN
    pub asn: u32,
    /// Number of observations/count
    pub count: u32,
    /// IP prefix
    pub prefix: String,
}

/// Result of a prefix-to-ASN lookup
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct Pfx2asResult {
    /// The queried prefix
    pub prefix: String,
    /// List of origin ASNs (comma-separated for display)
    pub asns: String,
    /// Match type (exact, longest, covering, covered)
    pub match_type: String,
}

/// Detailed result with structured ASN list
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asDetailedResult {
    /// The queried prefix
    pub prefix: String,
    /// The matched prefix (may differ for longest/covering queries)
    pub matched_prefix: String,
    /// List of origin ASNs
    pub origin_asns: Vec<u32>,
    /// Match type
    pub match_type: Pfx2asLookupMode,
}

/// Prefix record with validation status
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct Pfx2asPrefixRecord {
    /// IP prefix
    pub prefix: String,
    /// Origin ASN
    pub origin_asn: u32,
    /// RPKI validation status (if available)
    pub validation: String,
}

/// Search result with RPKI validation and optional AS name
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asSearchResult {
    /// IP prefix
    pub prefix: String,
    /// Origin ASN
    pub origin_asn: u32,
    /// AS name (if show_name is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_name: Option<String>,
    /// RPKI validation status
    pub rpki: String,
    /// Match type (for prefix queries: longest, super, sub)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_type: Option<String>,
}

/// Output format for Pfx2as lens results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Pfx2asOutputFormat {
    /// JSON format (default)
    #[default]
    Json,
    /// Pretty-printed JSON
    JsonPretty,
    /// Table format
    Table,
    /// Simple text format (ASNs only)
    Simple,
}

/// Lookup mode for prefix queries
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Pfx2asLookupMode {
    /// Exact match only
    Exact,
    /// Longest prefix match (default)
    #[default]
    Longest,
    /// Find all covering prefixes (supernets)
    Covering,
    /// Find all covered prefixes (subnets)
    Covered,
}

impl std::fmt::Display for Pfx2asLookupMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pfx2asLookupMode::Exact => write!(f, "exact"),
            Pfx2asLookupMode::Longest => write!(f, "longest"),
            Pfx2asLookupMode::Covering => write!(f, "covering"),
            Pfx2asLookupMode::Covered => write!(f, "covered"),
        }
    }
}

/// Query type for pfx2as searches
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pfx2asQueryType {
    /// Query by ASN
    Asn(u32),
    /// Query by prefix
    Prefix(String),
}

// =============================================================================
// Args
// =============================================================================

/// Arguments for Pfx2as lookup operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct Pfx2asLookupArgs {
    /// IP prefix to look up
    #[cfg_attr(feature = "cli", clap(value_name = "PREFIX"))]
    pub prefix: String,

    /// Lookup mode (exact, longest, covering, or covered)
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "longest"))]
    #[serde(default)]
    pub mode: Pfx2asLookupMode,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "json"))]
    #[serde(default)]
    pub format: Pfx2asOutputFormat,
}

impl Pfx2asLookupArgs {
    /// Create new args for a prefix lookup
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            mode: Pfx2asLookupMode::default(),
            format: Pfx2asOutputFormat::default(),
        }
    }

    /// Set lookup mode
    pub fn with_mode(mut self, mode: Pfx2asLookupMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set output format
    pub fn with_format(mut self, format: Pfx2asOutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Set exact match mode
    pub fn exact(mut self) -> Self {
        self.mode = Pfx2asLookupMode::Exact;
        self
    }

    /// Set longest prefix match mode
    pub fn longest(mut self) -> Self {
        self.mode = Pfx2asLookupMode::Longest;
        self
    }

    /// Set covering (supernets) mode
    pub fn covering(mut self) -> Self {
        self.mode = Pfx2asLookupMode::Covering;
        self
    }

    /// Set covered (subnets) mode
    pub fn covered(mut self) -> Self {
        self.mode = Pfx2asLookupMode::Covered;
        self
    }
}

/// Arguments for Pfx2as search operations (CLI-friendly)
///
/// This struct supports both prefix and ASN queries with options for
/// including sub/super prefixes, showing AS names, and RPKI validation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct Pfx2asSearchArgs {
    /// Query: an IP prefix (e.g., 1.1.1.0/24) or ASN (e.g., 13335, AS13335)
    #[cfg_attr(feature = "cli", clap(value_name = "QUERY"))]
    pub query: String,

    /// Include sub-prefixes (more specific) in results when querying by prefix
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub include_sub: bool,

    /// Include super-prefixes (less specific) in results when querying by prefix
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub include_super: bool,

    /// Show AS name for each origin ASN
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub show_name: bool,

    /// Show full AS name without truncation (default truncates to 20 chars)
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub show_full_name: bool,

    /// Limit the number of results (default: no limit)
    #[cfg_attr(feature = "cli", clap(long, short, value_name = "N"))]
    #[serde(default)]
    pub limit: Option<usize>,
}

impl Pfx2asSearchArgs {
    /// Create new search args with a query
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Default::default()
        }
    }

    /// Enable include_sub option
    pub fn with_include_sub(mut self, include_sub: bool) -> Self {
        self.include_sub = include_sub;
        self
    }

    /// Enable include_super option
    pub fn with_include_super(mut self, include_super: bool) -> Self {
        self.include_super = include_super;
        self
    }

    /// Enable show_name option
    pub fn with_show_name(mut self, show_name: bool) -> Self {
        self.show_name = show_name;
        self
    }

    /// Enable show_full_name option
    pub fn with_show_full_name(mut self, show_full_name: bool) -> Self {
        self.show_full_name = show_full_name;
        self
    }

    /// Set limit
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Validate arguments
    pub fn validate(&self) -> Result<(), String> {
        if self.query.is_empty() {
            return Err("Query cannot be empty".to_string());
        }
        Ok(())
    }
}

// =============================================================================
// Lens
// =============================================================================

/// Pfx2as lens for prefix-to-ASN mapping operations
///
/// This lens wraps `Pfx2asRepository` and provides:
/// - Prefix lookup with various modes (exact, longest, covering, covered)
/// - ASN-to-prefixes lookup
/// - Search with RPKI validation and AS name enrichment
/// - Cache management
/// - Output formatting
///
/// # Example
///
/// ```rust,ignore
/// use monocle::database::MonocleDatabase;
/// use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asSearchArgs};
///
/// let db = MonocleDatabase::open("~/.monocle/monocle-data.sqlite3")?;
/// let lens = Pfx2asLens::new(&db);
///
/// // Search by prefix with RPKI validation
/// let args = Pfx2asSearchArgs::new("1.1.1.0/24").with_show_name(true);
/// let results = lens.search(&args)?;
///
/// for result in &results {
///     println!("{} -> AS{} ({})", result.prefix, result.origin_asn, result.rpki);
/// }
/// ```
pub struct Pfx2asLens<'a> {
    /// Reference to the monocle database
    db: &'a MonocleDatabase,
}

impl<'a> Pfx2asLens<'a> {
    /// Create a new Pfx2as lens with database reference
    pub fn new(db: &'a MonocleDatabase) -> Self {
        Self { db }
    }

    // =========================================================================
    // Cache management
    // =========================================================================

    /// Check if the cache is empty
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.db.pfx2as().is_empty())
    }

    /// Check if the cache needs refresh (empty or expired)
    ///
    /// Uses the provided TTL to determine if the cache is stale.
    pub fn needs_refresh(&self, ttl: std::time::Duration) -> Result<bool> {
        Ok(self.db.pfx2as().needs_refresh(ttl))
    }

    /// Check why the cache needs refresh, if at all
    ///
    /// Returns `Some(RefreshReason)` if refresh is needed, `None` if data is current.
    /// Uses the provided TTL to determine if the cache is stale.
    pub fn refresh_reason(
        &self,
        ttl: std::time::Duration,
    ) -> Result<Option<crate::lens::utils::RefreshReason>> {
        use crate::lens::utils::RefreshReason;

        let pfx2as = self.db.pfx2as();

        // Check if empty first
        if pfx2as.is_empty() {
            return Ok(Some(RefreshReason::Empty));
        }

        // Check if outdated
        if pfx2as.needs_refresh(ttl) {
            return Ok(Some(RefreshReason::Outdated));
        }

        Ok(None)
    }

    /// Get cache metadata
    pub fn get_metadata(&self) -> Result<Option<crate::database::Pfx2asCacheDbMetadata>> {
        self.db.pfx2as().get_metadata()
    }

    /// Refresh the cache by loading data from the specified URL
    ///
    /// If no URL is provided, uses the default BGPKIT data source.
    /// Returns the number of records loaded.
    pub fn refresh(&self, url: Option<&str>) -> Result<usize> {
        use crate::database::Pfx2asDbRecord;

        let default_url = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";
        let url = url.unwrap_or(default_url);

        #[derive(serde::Deserialize)]
        struct Pfx2asEntry {
            prefix: String,
            asn: u32,
        }

        // Download and parse the data using oneio
        let entries: Vec<Pfx2asEntry> = oneio::read_json_struct(url)?;

        // Filter out invalid /0 prefixes and convert to database records
        let records: Vec<Pfx2asDbRecord> = entries
            .into_iter()
            .filter(|e| !e.prefix.ends_with("/0"))
            .map(|e| Pfx2asDbRecord {
                prefix: e.prefix,
                origin_asn: e.asn,
                validation: "unknown".to_string(),
            })
            .collect();

        let count = records.len();
        self.db.pfx2as().store(&records, url)?;

        Ok(count)
    }

    // =========================================================================
    // Query type detection
    // =========================================================================

    /// Detect whether a query is an ASN or a prefix
    pub fn detect_query_type(&self, query: &str) -> Pfx2asQueryType {
        let trimmed = query.trim();

        // Try to parse as ASN (with or without "AS" prefix)
        let asn_str = if trimmed.to_uppercase().starts_with("AS") {
            &trimmed[2..]
        } else {
            trimmed
        };

        if let Ok(asn) = asn_str.parse::<u32>() {
            // If it's a pure number or starts with AS, treat as ASN
            if trimmed.to_uppercase().starts_with("AS")
                || (!trimmed.contains('.') && !trimmed.contains(':'))
            {
                return Pfx2asQueryType::Asn(asn);
            }
        }

        // Otherwise treat as prefix
        Pfx2asQueryType::Prefix(trimmed.to_string())
    }

    // =========================================================================
    // Search operations (high-level, with RPKI and AS name enrichment)
    // =========================================================================

    /// Search for prefix-to-ASN mappings with RPKI validation and optional AS names
    ///
    /// This is the main entry point for pfx2as searches. It:
    /// - Auto-detects whether the query is an ASN or prefix
    /// - Performs the appropriate lookup
    /// - Enriches results with RPKI validation status
    /// - Optionally includes AS names
    pub fn search(&self, args: &Pfx2asSearchArgs) -> Result<Vec<Pfx2asSearchResult>> {
        // Validate args
        args.validate().map_err(|e| anyhow::anyhow!(e))?;

        let query_type = self.detect_query_type(&args.query);

        match query_type {
            Pfx2asQueryType::Asn(asn) => self.search_by_asn(asn, args),
            Pfx2asQueryType::Prefix(prefix) => self.search_by_prefix(&prefix, args),
        }
    }

    /// Search by ASN - returns all prefixes announced by the ASN
    pub fn search_by_asn(
        &self,
        asn: u32,
        args: &Pfx2asSearchArgs,
    ) -> Result<Vec<Pfx2asSearchResult>> {
        let records = self.get_prefixes_for_asn(asn)?;

        if records.is_empty() {
            return Ok(Vec::new());
        }

        // Apply limit
        let records: Vec<_> = if let Some(n) = args.limit {
            records.into_iter().take(n).collect()
        } else {
            records
        };

        // Get AS names if needed
        let show_name = args.show_name || args.show_full_name;
        let as_names = if show_name {
            self.get_as_names(&[asn])
        } else {
            HashMap::new()
        };

        // Get RPKI validation and build results
        let rpki_lens = RpkiLens::new(self.db);
        let mut results = Vec::new();

        for record in &records {
            let rpki_state = match rpki_lens.validate(&record.prefix, record.origin_asn) {
                Ok(result) => result.state.to_string(),
                Err(_) => "unknown".to_string(),
            };

            let as_name = if show_name {
                let name = as_names
                    .get(&record.origin_asn)
                    .cloned()
                    .unwrap_or_default();
                let display_name = if args.show_full_name {
                    name
                } else {
                    truncate_name(&name, DEFAULT_NAME_MAX_LEN)
                };
                Some(display_name)
            } else {
                None
            };

            results.push(Pfx2asSearchResult {
                prefix: record.prefix.clone(),
                origin_asn: record.origin_asn,
                as_name,
                rpki: rpki_state,
                match_type: None,
            });
        }

        Ok(results)
    }

    /// Search by prefix - returns origin ASNs with optional sub/super prefixes
    pub fn search_by_prefix(
        &self,
        prefix: &str,
        args: &Pfx2asSearchArgs,
    ) -> Result<Vec<Pfx2asSearchResult>> {
        // Collect results based on options
        // (prefix, asn, match_type)
        let mut all_results: Vec<(String, u32, String)> = Vec::new();

        // First, do longest match
        let longest_results = self.lookup_longest(prefix)?;
        for result in longest_results {
            for asn in &result.origin_asns {
                all_results.push((result.matched_prefix.clone(), *asn, "longest".to_string()));
            }
        }

        // Include super-prefixes (covering) if requested
        if args.include_super {
            let covering_results = self.lookup_covering(prefix)?;
            for result in covering_results {
                for asn in &result.origin_asns {
                    // Avoid duplicates from longest match
                    if !all_results
                        .iter()
                        .any(|(p, a, _)| p == &result.matched_prefix && a == asn)
                    {
                        all_results.push((
                            result.matched_prefix.clone(),
                            *asn,
                            "super".to_string(),
                        ));
                    }
                }
            }
        }

        // Include sub-prefixes (covered) if requested
        if args.include_sub {
            let covered_results = self.lookup_covered(prefix)?;
            for result in covered_results {
                for asn in &result.origin_asns {
                    // Avoid duplicates
                    if !all_results
                        .iter()
                        .any(|(p, a, _)| p == &result.matched_prefix && a == asn)
                    {
                        all_results.push((result.matched_prefix.clone(), *asn, "sub".to_string()));
                    }
                }
            }
        }

        if all_results.is_empty() {
            return Ok(Vec::new());
        }

        // Apply limit
        let all_results: Vec<_> = if let Some(n) = args.limit {
            all_results.into_iter().take(n).collect()
        } else {
            all_results
        };

        // Get unique ASNs for name lookup
        let unique_asns: Vec<u32> = all_results
            .iter()
            .map(|(_, asn, _)| *asn)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        // Get AS names if needed
        let show_name = args.show_name || args.show_full_name;
        let as_names = if show_name {
            self.get_as_names(&unique_asns)
        } else {
            HashMap::new()
        };

        // Get RPKI validation and build results
        let rpki_lens = RpkiLens::new(self.db);
        let mut results = Vec::new();

        for (pfx, asn, match_type) in &all_results {
            let rpki_state = match rpki_lens.validate(pfx, *asn) {
                Ok(result) => result.state.to_string(),
                Err(_) => "unknown".to_string(),
            };

            let as_name = if show_name {
                let name = as_names.get(asn).cloned().unwrap_or_default();
                let display_name = if args.show_full_name {
                    name
                } else {
                    truncate_name(&name, DEFAULT_NAME_MAX_LEN)
                };
                Some(display_name)
            } else {
                None
            };

            results.push(Pfx2asSearchResult {
                prefix: pfx.clone(),
                origin_asn: *asn,
                as_name,
                rpki: rpki_state,
                match_type: Some(match_type.clone()),
            });
        }

        Ok(results)
    }

    /// Get AS names for a list of ASNs
    fn get_as_names(&self, asns: &[u32]) -> HashMap<u32, String> {
        let mut names = HashMap::new();
        let asinfo = self.db.asinfo();

        for asn in asns {
            if let Ok(Some(record)) = asinfo.get_full(*asn) {
                names.insert(*asn, record.core.name);
            }
        }

        names
    }

    // =========================================================================
    // Lookup operations (low-level)
    // =========================================================================

    /// Look up a prefix based on the provided arguments
    pub fn lookup(&self, args: &Pfx2asLookupArgs) -> Result<Vec<Pfx2asDetailedResult>> {
        match args.mode {
            Pfx2asLookupMode::Exact => self.lookup_exact(&args.prefix),
            Pfx2asLookupMode::Longest => self.lookup_longest(&args.prefix),
            Pfx2asLookupMode::Covering => self.lookup_covering(&args.prefix),
            Pfx2asLookupMode::Covered => self.lookup_covered(&args.prefix),
        }
    }

    /// Exact prefix match
    pub fn lookup_exact(&self, prefix: &str) -> Result<Vec<Pfx2asDetailedResult>> {
        let asns = self.db.pfx2as().lookup_exact(prefix)?;

        if asns.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(vec![Pfx2asDetailedResult {
                prefix: prefix.to_string(),
                matched_prefix: prefix.to_string(),
                origin_asns: asns,
                match_type: Pfx2asLookupMode::Exact,
            }])
        }
    }

    /// Longest prefix match
    pub fn lookup_longest(&self, prefix: &str) -> Result<Vec<Pfx2asDetailedResult>> {
        let result = self.db.pfx2as().lookup_longest(prefix)?;

        if result.origin_asns.is_empty() {
            Ok(Vec::new())
        } else {
            Ok(vec![Pfx2asDetailedResult {
                prefix: prefix.to_string(),
                matched_prefix: result.prefix,
                origin_asns: result.origin_asns,
                match_type: Pfx2asLookupMode::Longest,
            }])
        }
    }

    /// Find all covering prefixes (supernets)
    pub fn lookup_covering(&self, prefix: &str) -> Result<Vec<Pfx2asDetailedResult>> {
        let results = self.db.pfx2as().lookup_covering(prefix)?;

        Ok(results
            .into_iter()
            .map(|r| Pfx2asDetailedResult {
                prefix: prefix.to_string(),
                matched_prefix: r.prefix,
                origin_asns: r.origin_asns,
                match_type: Pfx2asLookupMode::Covering,
            })
            .collect())
    }

    /// Find all covered prefixes (subnets)
    pub fn lookup_covered(&self, prefix: &str) -> Result<Vec<Pfx2asDetailedResult>> {
        let results = self.db.pfx2as().lookup_covered(prefix)?;

        Ok(results
            .into_iter()
            .map(|r| Pfx2asDetailedResult {
                prefix: prefix.to_string(),
                matched_prefix: r.prefix,
                origin_asns: r.origin_asns,
                match_type: Pfx2asLookupMode::Covered,
            })
            .collect())
    }

    /// Get all prefixes for an ASN
    pub fn get_prefixes_for_asn(&self, asn: u32) -> Result<Vec<Pfx2asPrefixRecord>> {
        let records = self.db.pfx2as().get_by_asn(asn)?;

        Ok(records
            .into_iter()
            .map(|r| Pfx2asPrefixRecord {
                prefix: r.prefix,
                origin_asn: r.origin_asn,
                validation: r.validation,
            })
            .collect())
    }

    /// Get record count
    pub fn record_count(&self) -> Result<usize> {
        Ok(self.db.pfx2as().record_count()? as usize)
    }

    /// Get prefix count
    pub fn prefix_count(&self) -> Result<usize> {
        Ok(self.db.pfx2as().prefix_count()? as usize)
    }

    // =========================================================================
    // Formatting
    // =========================================================================

    /// Format search results for display
    pub fn format_search_results(
        &self,
        results: &[Pfx2asSearchResult],
        format: &OutputFormat,
        show_name: bool,
    ) -> String {
        match format {
            OutputFormat::Json => serde_json::to_string(results).unwrap_or_default(),
            OutputFormat::JsonPretty => serde_json::to_string_pretty(results).unwrap_or_default(),
            OutputFormat::JsonLine => results
                .iter()
                .filter_map(|r| serde_json::to_string(r).ok())
                .collect::<Vec<_>>()
                .join("\n"),
            OutputFormat::Table | OutputFormat::Markdown => {
                use tabled::settings::Style;
                use tabled::Table;

                if show_name {
                    #[derive(Tabled)]
                    struct Row {
                        prefix: String,
                        origin_asn: u32,
                        as_name: String,
                        rpki: String,
                    }

                    let rows: Vec<Row> = results
                        .iter()
                        .map(|r| Row {
                            prefix: r.prefix.clone(),
                            origin_asn: r.origin_asn,
                            as_name: r.as_name.clone().unwrap_or_default(),
                            rpki: r.rpki.clone(),
                        })
                        .collect();

                    let mut table = Table::new(rows);
                    if matches!(format, OutputFormat::Markdown) {
                        table.with(Style::markdown())
                    } else {
                        table.with(Style::rounded())
                    }
                    .to_string()
                } else {
                    #[derive(Tabled)]
                    struct Row {
                        prefix: String,
                        origin_asn: u32,
                        rpki: String,
                    }

                    let rows: Vec<Row> = results
                        .iter()
                        .map(|r| Row {
                            prefix: r.prefix.clone(),
                            origin_asn: r.origin_asn,
                            rpki: r.rpki.clone(),
                        })
                        .collect();

                    let mut table = Table::new(rows);
                    if matches!(format, OutputFormat::Markdown) {
                        table.with(Style::markdown())
                    } else {
                        table.with(Style::rounded())
                    }
                    .to_string()
                }
            }
            OutputFormat::Psv => {
                let mut output = if show_name {
                    "prefix|origin_asn|as_name|rpki\n".to_string()
                } else {
                    "prefix|origin_asn|rpki\n".to_string()
                };

                for r in results {
                    if show_name {
                        output.push_str(&format!(
                            "{}|{}|{}|{}\n",
                            r.prefix,
                            r.origin_asn,
                            r.as_name.as_deref().unwrap_or(""),
                            r.rpki
                        ));
                    } else {
                        output.push_str(&format!("{}|{}|{}\n", r.prefix, r.origin_asn, r.rpki));
                    }
                }

                output.trim_end().to_string()
            }
        }
    }

    /// Format lookup results for display
    pub fn format_results(
        &self,
        results: &[Pfx2asDetailedResult],
        format: &Pfx2asOutputFormat,
    ) -> String {
        match format {
            Pfx2asOutputFormat::Json => serde_json::to_string(results).unwrap_or_default(),
            Pfx2asOutputFormat::JsonPretty => {
                serde_json::to_string_pretty(results).unwrap_or_default()
            }
            Pfx2asOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;

                let rows: Vec<Pfx2asResult> = results
                    .iter()
                    .map(|r| Pfx2asResult {
                        prefix: r.matched_prefix.clone(),
                        asns: r
                            .origin_asns
                            .iter()
                            .map(|a| a.to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                        match_type: r.match_type.to_string(),
                    })
                    .collect();

                Table::new(rows).with(Style::rounded()).to_string()
            }
            Pfx2asOutputFormat::Simple => results
                .iter()
                .map(|r| {
                    r.origin_asns
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Format prefix records for display
    pub fn format_prefixes(
        &self,
        prefixes: &[Pfx2asPrefixRecord],
        format: &Pfx2asOutputFormat,
    ) -> String {
        match format {
            Pfx2asOutputFormat::Json => serde_json::to_string(prefixes).unwrap_or_default(),
            Pfx2asOutputFormat::JsonPretty => {
                serde_json::to_string_pretty(prefixes).unwrap_or_default()
            }
            Pfx2asOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;
                Table::new(prefixes).with(Style::rounded()).to_string()
            }
            Pfx2asOutputFormat::Simple => prefixes
                .iter()
                .map(|p| p.prefix.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_args() {
        let args = Pfx2asLookupArgs::new("1.1.1.0/24")
            .exact()
            .with_format(Pfx2asOutputFormat::Table);

        assert_eq!(args.prefix, "1.1.1.0/24");
        assert!(matches!(args.mode, Pfx2asLookupMode::Exact));
        assert!(matches!(args.format, Pfx2asOutputFormat::Table));
    }

    #[test]
    fn test_lookup_modes() {
        let args = Pfx2asLookupArgs::new("10.0.0.0/8").covering();
        assert!(matches!(args.mode, Pfx2asLookupMode::Covering));

        let args = Pfx2asLookupArgs::new("10.0.0.0/8").covered();
        assert!(matches!(args.mode, Pfx2asLookupMode::Covered));

        let args = Pfx2asLookupArgs::new("10.0.0.0/8").longest();
        assert!(matches!(args.mode, Pfx2asLookupMode::Longest));
    }

    #[test]
    fn test_lookup_mode_display() {
        assert_eq!(Pfx2asLookupMode::Exact.to_string(), "exact");
        assert_eq!(Pfx2asLookupMode::Longest.to_string(), "longest");
        assert_eq!(Pfx2asLookupMode::Covering.to_string(), "covering");
        assert_eq!(Pfx2asLookupMode::Covered.to_string(), "covered");
    }

    #[test]
    fn test_detailed_result_serialization() {
        let result = Pfx2asDetailedResult {
            prefix: "1.1.1.0/24".to_string(),
            matched_prefix: "1.1.0.0/20".to_string(),
            origin_asns: vec![13335, 13336],
            match_type: Pfx2asLookupMode::Longest,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("1.1.1.0/24"));
        assert!(json.contains("13335"));
    }

    #[test]
    fn test_search_args() {
        let args = Pfx2asSearchArgs::new("1.1.1.0/24")
            .with_include_sub(true)
            .with_show_name(true)
            .with_limit(10);

        assert_eq!(args.query, "1.1.1.0/24");
        assert!(args.include_sub);
        assert!(args.show_name);
        assert_eq!(args.limit, Some(10));
    }

    #[test]
    fn test_search_args_validation() {
        let args = Pfx2asSearchArgs::new("");
        assert!(args.validate().is_err());

        let args = Pfx2asSearchArgs::new("1.1.1.0/24");
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_search_result_serialization() {
        let result = Pfx2asSearchResult {
            prefix: "1.1.1.0/24".to_string(),
            origin_asn: 13335,
            as_name: Some("CLOUDFLARENET".to_string()),
            rpki: "valid".to_string(),
            match_type: Some("longest".to_string()),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("1.1.1.0/24"));
        assert!(json.contains("13335"));
        assert!(json.contains("CLOUDFLARENET"));
        assert!(json.contains("valid"));
    }

    #[test]
    fn test_search_result_without_optional_fields() {
        let result = Pfx2asSearchResult {
            prefix: "1.1.1.0/24".to_string(),
            origin_asn: 13335,
            as_name: None,
            rpki: "valid".to_string(),
            match_type: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("1.1.1.0/24"));
        assert!(!json.contains("as_name")); // should be skipped
        assert!(!json.contains("match_type")); // should be skipped
    }
}
