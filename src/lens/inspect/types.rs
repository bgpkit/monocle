//! Types for the inspect lens module
//!
//! This module defines all the types used by the inspect lens for unified
//! AS and prefix information queries.

use crate::database::{AsinfoCoreRecord, AsinfoFullRecord, RpkiRoaRecord};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// Re-export connectivity types from the database module
pub use crate::database::{AsConnectivitySummary, ConnectivityEntry, ConnectivityGroup};

// =============================================================================
// Query Type Detection
// =============================================================================

/// Query type detected from input
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectQueryType {
    Asn,
    Prefix,
    Name,
}

impl std::fmt::Display for InspectQueryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InspectQueryType::Asn => write!(f, "asn"),
            InspectQueryType::Prefix => write!(f, "prefix"),
            InspectQueryType::Name => write!(f, "name"),
        }
    }
}

// =============================================================================
// Data Section Selection
// =============================================================================

/// Available data sections that can be selected via --show
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectDataSection {
    /// Basic AS information (name, country, org, peeringdb, hegemony, population)
    Basic,
    /// Announced prefixes (from pfx2as)
    Prefixes,
    /// AS connectivity (from as2rel)
    Connectivity,
    /// RPKI information (ROAs and ASPA)
    Rpki,
}

impl InspectDataSection {
    /// Get all available sections
    pub fn all() -> Vec<Self> {
        vec![Self::Basic, Self::Prefixes, Self::Connectivity, Self::Rpki]
    }

    /// Default sections for ASN queries (basic only)
    pub fn default_for_asn() -> Vec<Self> {
        vec![Self::Basic]
    }

    /// Default sections for prefix queries
    pub fn default_for_prefix() -> Vec<Self> {
        vec![Self::Basic, Self::Rpki]
    }

    /// Default sections for name search
    pub fn default_for_name() -> Vec<Self> {
        vec![Self::Basic]
    }

    /// Parse from string
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "basic" => Some(Self::Basic),
            "prefixes" => Some(Self::Prefixes),
            "connectivity" => Some(Self::Connectivity),
            "rpki" => Some(Self::Rpki),
            _ => None,
        }
    }

    /// Get all section names as strings
    pub fn all_names() -> Vec<&'static str> {
        vec!["basic", "prefixes", "connectivity", "rpki"]
    }
}

impl std::fmt::Display for InspectDataSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Basic => write!(f, "basic"),
            Self::Prefixes => write!(f, "prefixes"),
            Self::Connectivity => write!(f, "connectivity"),
            Self::Rpki => write!(f, "rpki"),
        }
    }
}

// =============================================================================
// Query Options
// =============================================================================

/// Options for controlling inspect query behavior
#[derive(Debug, Clone)]
pub struct InspectQueryOptions {
    /// Which data sections to include (None = defaults based on query type)
    pub select: Option<HashSet<InspectDataSection>>,

    /// Maximum ROAs to return (0 = unlimited)
    pub max_roas: usize,

    /// Maximum prefixes to return (0 = unlimited)
    pub max_prefixes: usize,

    /// Maximum neighbors per category (0 = unlimited)
    pub max_neighbors: usize,

    /// Maximum search results (0 = unlimited)
    pub max_search_results: usize,
}

impl Default for InspectQueryOptions {
    fn default() -> Self {
        Self {
            select: None, // Use defaults based on query type
            max_roas: 10,
            max_prefixes: 10,
            max_neighbors: 5,
            max_search_results: 20,
        }
    }
}

impl InspectQueryOptions {
    /// Create options for full output with no limits
    pub fn full() -> Self {
        Self {
            select: Some(InspectDataSection::all().into_iter().collect()),
            max_roas: 0,
            max_prefixes: 0,
            max_neighbors: 0,
            max_search_results: 0,
        }
    }

    /// Set specific sections to select
    pub fn with_select(mut self, sections: Vec<InspectDataSection>) -> Self {
        self.select = Some(sections.into_iter().collect());
        self
    }

    /// Check if a section should be included for the given query type
    pub fn should_include(
        &self,
        section: InspectDataSection,
        query_type: InspectQueryType,
    ) -> bool {
        match &self.select {
            Some(selected) => selected.contains(&section),
            None => {
                let defaults = match query_type {
                    InspectQueryType::Asn => InspectDataSection::default_for_asn(),
                    InspectQueryType::Prefix => InspectDataSection::default_for_prefix(),
                    InspectQueryType::Name => InspectDataSection::default_for_name(),
                };
                defaults.contains(&section)
            }
        }
    }
}

// =============================================================================
// RPKI Types
// =============================================================================

/// RPKI information for an ASN
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiAsnInfo {
    /// ROAs where this ASN is the origin
    pub roas: Option<RoaSummary>,

    /// ASPA record for this ASN (if exists)
    pub aspa: Option<AspaInfo>,
}

/// Summary of ROAs for an ASN
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoaSummary {
    /// Total ROA count for this ASN
    pub total_count: usize,

    /// IPv4 ROA count
    pub ipv4_count: usize,

    /// IPv6 ROA count
    pub ipv6_count: usize,

    /// ROA entries (limited by default, sorted by prefix)
    pub entries: Vec<RpkiRoaRecord>,

    /// Whether entries were truncated
    pub truncated: bool,
}

/// ASPA information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AspaInfo {
    pub customer_asn: u32,
    pub provider_asns: Vec<u32>,
    /// Provider names (enriched from asinfo)
    pub provider_names: Vec<Option<String>>,
}

/// RPKI information for a prefix
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiPrefixInfo {
    /// Covering ROAs (sorted by prefix, then max_length, then ASN)
    pub roas: Vec<RpkiRoaRecord>,

    /// ROA count
    pub roa_count: usize,

    /// Validation state (if single origin ASN known)
    pub validation_state: Option<String>,

    /// Whether ROAs were truncated
    pub truncated: bool,
}

// =============================================================================
// Connectivity Types
// =============================================================================

// Note: AsConnectivitySummary, ConnectivityGroup, and ConnectivityEntry
// are re-exported from crate::database::as2rel at the top of this file.

/// Connectivity section wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivitySection {
    pub summary: AsConnectivitySummary,

    /// Whether neighbor lists were truncated
    pub truncated: bool,
}

// =============================================================================
// Prefix Types
// =============================================================================

/// Prefix-to-AS mapping info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asInfo {
    pub prefix: String,
    pub origin_asns: Vec<u32>,
    /// "exact" or "longest"
    pub match_type: String,
    /// RPKI validation status for each origin ASN
    pub validations: Vec<String>,
}

/// Prefix information section (for prefix queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixSection {
    /// Prefix-to-AS mapping result
    pub pfx2as: Option<Pfx2asInfo>,

    /// RPKI information for this prefix
    pub rpki: Option<RpkiPrefixInfo>,
}

/// Announced prefixes section (for ASN queries with --select prefixes)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnouncedPrefixesSection {
    /// Total prefix count
    pub total_count: usize,

    /// IPv4 count
    pub ipv4_count: usize,

    /// IPv6 count
    pub ipv6_count: usize,

    /// RPKI validation summary
    pub validation_summary: ValidationSummary,

    /// Prefix entries with validation status (sorted by validation then prefix)
    pub prefixes: Vec<PrefixEntry>,

    /// Whether prefixes were truncated
    pub truncated: bool,
}

/// A single prefix entry with origin ASN info and validation status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixEntry {
    pub prefix: String,
    pub origin_asn: u32,
    pub origin_name: Option<String>,
    pub origin_country: Option<String>,
    pub validation: String,
}

/// RPKI validation summary for prefixes
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationSummary {
    pub valid_count: usize,
    pub valid_percent: f64,
    pub invalid_count: usize,
    pub invalid_percent: f64,
    pub unknown_count: usize,
    pub unknown_percent: f64,
}

impl ValidationSummary {
    pub fn from_counts(valid: usize, invalid: usize, unknown: usize) -> Self {
        let total = valid + invalid + unknown;
        let total_f64 = total as f64;
        Self {
            valid_count: valid,
            valid_percent: if total > 0 {
                (valid as f64 / total_f64) * 100.0
            } else {
                0.0
            },
            invalid_count: invalid,
            invalid_percent: if total > 0 {
                (invalid as f64 / total_f64) * 100.0
            } else {
                0.0
            },
            unknown_count: unknown,
            unknown_percent: if total > 0 {
                (unknown as f64 / total_f64) * 100.0
            } else {
                0.0
            },
        }
    }
}

// =============================================================================
// ASInfo Section Types
// =============================================================================

/// Wrapper for ASInfo in results - distinguishes between direct query vs origin lookup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoSection {
    /// Full AS info for directly queried ASN (ASN queries)
    pub detail: Option<AsinfoFullRecord>,

    /// AS info for origin ASNs (prefix queries via pfx2as)
    pub origins: Option<Vec<AsinfoFullRecord>>,
}

// =============================================================================
// Search Results
// =============================================================================

/// Search results section (for name queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultsSection {
    /// Total matches found
    pub total_matches: usize,

    /// Results (sorted by ASN, limited by default)
    pub results: Vec<AsinfoCoreRecord>,

    /// Whether results were truncated
    pub truncated: bool,
}

// =============================================================================
// Main Query Result Types
// =============================================================================

/// Result for a single query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectQueryResult {
    /// Original query string
    pub query: String,

    /// Detected query type
    pub query_type: InspectQueryType,

    /// ASN information section
    /// - For ASN queries: contains `detail` (full record for queried ASN)
    /// - For prefix queries: contains `origins` (records for origin ASNs)
    pub asinfo: Option<AsinfoSection>,

    /// Prefix information (for prefix queries only)
    /// Contains pfx2as mapping and RPKI validation
    pub prefix: Option<PrefixSection>,

    /// Announced prefixes (for ASN queries with --select prefixes)
    pub prefixes: Option<AnnouncedPrefixesSection>,

    /// Connectivity information (for ASN queries)
    pub connectivity: Option<ConnectivitySection>,

    /// RPKI information (for ASN queries - ROAs originated, ASPA)
    pub rpki: Option<RpkiAsnInfo>,

    /// Search results (for name queries only)
    pub search_results: Option<SearchResultsSection>,
}

impl InspectQueryResult {
    /// Create a new empty result for an ASN query
    pub fn new_asn(query: String) -> Self {
        Self {
            query,
            query_type: InspectQueryType::Asn,
            asinfo: None,
            prefix: None,
            prefixes: None,
            connectivity: None,
            rpki: None,
            search_results: None,
        }
    }

    /// Create a new empty result for a prefix query
    pub fn new_prefix(query: String) -> Self {
        Self {
            query,
            query_type: InspectQueryType::Prefix,
            asinfo: None,
            prefix: None,
            prefixes: None,
            connectivity: None,
            rpki: None,
            search_results: None,
        }
    }

    /// Create a new empty result for a name query
    pub fn new_name(query: String) -> Self {
        Self {
            query,
            query_type: InspectQueryType::Name,
            asinfo: None,
            prefix: None,
            prefixes: None,
            connectivity: None,
            rpki: None,
            search_results: None,
        }
    }
}

/// Combined result for multiple queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResult {
    /// Individual query results
    pub queries: Vec<InspectQueryResult>,

    /// Processing metadata
    pub meta: InspectResultMeta,
}

/// Metadata about the inspect operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResultMeta {
    pub query_count: usize,
    pub asn_queries: usize,
    pub prefix_queries: usize,
    pub name_queries: usize,
    pub processing_time_ms: u64,
}

// =============================================================================
// Display Configuration
// =============================================================================

/// Display mode for multi-ASN queries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MultiAsnDisplayMode {
    /// Standard mode - show each ASN result separately
    #[default]
    Standard,
    /// Table mode - show all ASNs in a single table, one per row
    Table,
}

/// Determines display configuration based on terminal width
#[derive(Debug, Clone)]
pub struct InspectDisplayConfig {
    pub terminal_width: usize,
    pub show_hegemony: bool,
    pub show_population: bool,
    pub show_peeringdb: bool,
    pub truncate_names: bool,
    pub name_max_width: usize,
    /// Use markdown table style instead of rounded
    pub use_markdown_style: bool,
    /// Force showing extended info (peeringdb, hegemony, population) regardless of width
    pub force_extended_info: bool,
    /// Display mode for multi-ASN queries
    pub multi_asn_mode: MultiAsnDisplayMode,
}

impl InspectDisplayConfig {
    /// Create display config based on terminal width
    pub fn from_terminal_width(width: usize) -> Self {
        match width {
            0..=80 => Self {
                terminal_width: width,
                show_hegemony: false,
                show_population: false,
                show_peeringdb: false,
                truncate_names: true,
                name_max_width: 25,
                use_markdown_style: false,
                force_extended_info: false,
                multi_asn_mode: MultiAsnDisplayMode::Standard,
            },
            81..=120 => Self {
                terminal_width: width,
                show_hegemony: false,
                show_population: false,
                show_peeringdb: false,
                truncate_names: true,
                name_max_width: 35,
                use_markdown_style: false,
                force_extended_info: false,
                multi_asn_mode: MultiAsnDisplayMode::Standard,
            },
            121..=160 => Self {
                terminal_width: width,
                show_hegemony: true,
                show_population: false,
                show_peeringdb: false,
                truncate_names: true,
                name_max_width: 45,
                use_markdown_style: false,
                force_extended_info: false,
                multi_asn_mode: MultiAsnDisplayMode::Standard,
            },
            _ => Self {
                terminal_width: width,
                show_hegemony: true,
                show_population: true,
                show_peeringdb: true,
                truncate_names: false,
                name_max_width: 60,
                use_markdown_style: false,
                force_extended_info: false,
                multi_asn_mode: MultiAsnDisplayMode::Standard,
            },
        }
    }

    /// Auto-detect terminal width
    ///
    /// Uses the COLUMNS environment variable if available, otherwise defaults to 80.
    pub fn auto() -> Self {
        let width = std::env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(80);
        Self::from_terminal_width(width)
    }

    /// Set markdown style output
    pub fn with_markdown(mut self, use_markdown: bool) -> Self {
        self.use_markdown_style = use_markdown;
        self
    }

    /// Force extended info (peeringdb, hegemony, population) regardless of terminal width
    pub fn with_extended_info(mut self, force: bool) -> Self {
        self.force_extended_info = force;
        if force {
            self.show_hegemony = true;
            self.show_population = true;
            self.show_peeringdb = true;
        }
        self
    }

    /// Set multi-ASN display mode
    pub fn with_multi_asn_mode(mut self, mode: MultiAsnDisplayMode) -> Self {
        self.multi_asn_mode = mode;
        self
    }

    /// Check if hegemony should be shown (respects force_extended_info)
    pub fn should_show_hegemony(&self) -> bool {
        self.force_extended_info || self.show_hegemony
    }

    /// Check if population should be shown (respects force_extended_info)
    pub fn should_show_population(&self) -> bool {
        self.force_extended_info || self.show_population
    }

    /// Check if peeringdb should be shown (respects force_extended_info)
    pub fn should_show_peeringdb(&self) -> bool {
        self.force_extended_info || self.show_peeringdb
    }
}

impl Default for InspectDisplayConfig {
    fn default() -> Self {
        Self::auto()
    }
}
