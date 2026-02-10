//! Inspect lens
//!
//! This module provides a unified lens for querying AS and prefix information.
//! It consolidates functionality from the former `whois`, `pfx2as`, and `as2rel` commands.
//!
//! # Query Types
//!
//! The lens supports three query types:
//! - **ASN**: Query information about an Autonomous System (e.g., "13335", "AS13335")
//! - **Prefix**: Query information about an IP prefix (e.g., "1.1.1.0/24")
//! - **Name**: Search for ASes by name or organization (e.g., "cloudflare")
//!
//! # Data Sources
//!
//! The lens aggregates data from multiple sources:
//! - **ASInfo**: Core AS information, AS2Org, PeeringDB, Hegemony, Population
//! - **AS2Rel**: AS-level relationships and connectivity
//! - **RPKI**: ROAs and ASPA records
//! - **Pfx2as**: Prefix-to-ASN mappings

pub mod types;

pub use types::*;

use crate::database::{
    AsinfoCoreRecord, AsinfoFullRecord, AsinfoStoreCounts, MonocleDatabase,
    DEFAULT_PFX2AS_CACHE_TTL, DEFAULT_RPKI_CACHE_TTL,
};
use crate::lens::country::CountryLens;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::Instant;
use tabled::settings::Style;
use tabled::{Table, Tabled};
use tracing::info;

// =============================================================================
// Data Source Status Types
// =============================================================================

/// Status of a data source refresh operation
#[derive(Debug, Clone, Serialize)]
pub struct DataSourceRefresh {
    /// Name of the data source
    pub source: String,
    /// Whether the refresh was performed
    pub refreshed: bool,
    /// Status message
    pub message: String,
    /// Number of records (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
}

/// Summary of all data source refresh operations
#[derive(Debug, Clone, Serialize, Default)]
pub struct DataRefreshSummary {
    /// List of refresh operations performed
    pub sources: Vec<DataSourceRefresh>,
    /// Whether any refresh was performed
    pub any_refreshed: bool,
}

impl DataRefreshSummary {
    /// Create a new empty summary
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a refresh result
    pub fn add(
        &mut self,
        source: impl Into<String>,
        refreshed: bool,
        message: impl Into<String>,
        count: Option<usize>,
    ) {
        if refreshed {
            self.any_refreshed = true;
        }
        self.sources.push(DataSourceRefresh {
            source: source.into(),
            refreshed,
            message: message.into(),
            count,
        });
    }

    /// Format as human-readable messages
    pub fn format_messages(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter(|s| s.refreshed)
            .map(|s| s.message.clone())
            .collect()
    }
}

/// Inspect lens for unified AS and prefix information queries
///
/// This lens provides high-level operations for:
/// - Querying AS information from multiple sources
/// - Looking up prefix-to-ASN mappings
/// - Searching for ASes by name or organization
/// - Retrieving RPKI data (ROAs, ASPA)
/// - Getting AS connectivity information
pub struct InspectLens<'a> {
    db: &'a MonocleDatabase,
    country_lookup: CountryLens,
}

impl<'a> InspectLens<'a> {
    /// Create a new Inspect lens
    pub fn new(db: &'a MonocleDatabase) -> Self {
        Self {
            db,
            country_lookup: CountryLens::new(),
        }
    }

    // =========================================================================
    // Status Methods
    // =========================================================================

    /// Check if ASInfo data is available
    pub fn is_data_available(&self) -> bool {
        !self.db.asinfo().is_empty()
    }

    /// Check if data needs to be bootstrapped
    pub fn needs_bootstrap(&self) -> bool {
        self.db.needs_asinfo_bootstrap()
    }

    /// Check if data needs refresh
    pub fn needs_refresh(&self) -> bool {
        self.db.needs_asinfo_refresh()
    }

    // =========================================================================
    // Data Management
    // =========================================================================

    /// Bootstrap ASInfo data from the default URL
    pub fn bootstrap(&self) -> Result<AsinfoStoreCounts> {
        info!("Bootstrapping ASInfo data...");
        self.db.bootstrap_asinfo()
    }

    /// Refresh ASInfo data (same as bootstrap, but logs differently)
    pub fn refresh(&self) -> Result<AsinfoStoreCounts> {
        info!("Refreshing ASInfo data...");
        self.db.bootstrap_asinfo()
    }

    /// Ensure all required data sources are available, refreshing if needed
    ///
    /// This method checks each data source and refreshes it if empty or expired.
    /// Returns a summary of what was refreshed.
    ///
    /// Data sources checked:
    /// - ASInfo: Core AS information (name, country, etc.)
    /// - AS2Rel: AS-level relationships for connectivity
    /// - RPKI: ROAs and ASPAs for RPKI information
    /// - Pfx2as: Prefix-to-ASN mappings
    pub fn ensure_data_available(&self) -> Result<DataRefreshSummary> {
        let mut summary = DataRefreshSummary::new();

        // Check and refresh ASInfo
        if self.db.asinfo().is_empty() {
            eprintln!("[monocle] Loading ASInfo data (AS names, organizations, PeeringDB)...");
            info!("ASInfo data is empty, bootstrapping...");
            match self.db.bootstrap_asinfo() {
                Ok(counts) => {
                    summary.add(
                        "asinfo",
                        true,
                        format!(
                            "ASInfo data loaded: {} core, {} as2org, {} peeringdb",
                            counts.core, counts.as2org, counts.peeringdb
                        ),
                        Some(counts.core),
                    );
                }
                Err(e) => {
                    summary.add(
                        "asinfo",
                        false,
                        format!("Failed to load ASInfo: {}", e),
                        None,
                    );
                }
            }
        } else if self.db.needs_asinfo_refresh() {
            eprintln!("[monocle] Refreshing ASInfo data (AS names, organizations, PeeringDB)...");
            info!("ASInfo data is stale, refreshing...");
            match self.db.bootstrap_asinfo() {
                Ok(counts) => {
                    summary.add(
                        "asinfo",
                        true,
                        format!(
                            "ASInfo data refreshed: {} core, {} as2org, {} peeringdb",
                            counts.core, counts.as2org, counts.peeringdb
                        ),
                        Some(counts.core),
                    );
                }
                Err(e) => {
                    summary.add(
                        "asinfo",
                        false,
                        format!("Failed to refresh ASInfo: {}", e),
                        None,
                    );
                }
            }
        }

        // Check and refresh AS2Rel
        if self.db.as2rel().is_empty() {
            eprintln!("[monocle] Loading AS2Rel data (AS relationships)...");
            info!("AS2Rel data is empty, loading...");
            match self.db.update_as2rel() {
                Ok(count) => {
                    summary.add(
                        "as2rel",
                        true,
                        format!("AS2Rel data loaded: {} relationships", count),
                        Some(count),
                    );
                }
                Err(e) => {
                    summary.add(
                        "as2rel",
                        false,
                        format!("Failed to load AS2Rel: {}", e),
                        None,
                    );
                }
            }
        } else if self.db.needs_as2rel_update() {
            eprintln!("[monocle] Refreshing AS2Rel data (AS relationships)...");
            info!("AS2Rel data is stale, refreshing...");
            match self.db.update_as2rel() {
                Ok(count) => {
                    summary.add(
                        "as2rel",
                        true,
                        format!("AS2Rel data refreshed: {} relationships", count),
                        Some(count),
                    );
                }
                Err(e) => {
                    summary.add(
                        "as2rel",
                        false,
                        format!("Failed to refresh AS2Rel: {}", e),
                        None,
                    );
                }
            }
        }

        // Check and refresh RPKI
        if self.db.rpki().is_empty() {
            eprintln!("[monocle] Loading RPKI data (ROAs, ASPA)...");
            info!("RPKI data is empty, loading from bgpkit-commons...");
            match self.refresh_rpki_from_commons() {
                Ok(count) => {
                    summary.add(
                        "rpki",
                        true,
                        format!("RPKI data loaded: {} ROAs", count),
                        Some(count),
                    );
                }
                Err(e) => {
                    summary.add("rpki", false, format!("Failed to load RPKI: {}", e), None);
                }
            }
        } else if self.db.rpki().needs_refresh(DEFAULT_RPKI_CACHE_TTL) {
            eprintln!("[monocle] Refreshing RPKI data (ROAs, ASPA)...");
            info!("RPKI data is stale, refreshing...");
            match self.refresh_rpki_from_commons() {
                Ok(count) => {
                    summary.add(
                        "rpki",
                        true,
                        format!("RPKI data refreshed: {} ROAs", count),
                        Some(count),
                    );
                }
                Err(e) => {
                    summary.add(
                        "rpki",
                        false,
                        format!("Failed to refresh RPKI: {}", e),
                        None,
                    );
                }
            }
        }

        // Check and refresh Pfx2as
        if self.db.pfx2as().is_empty() {
            eprintln!("[monocle] Loading Pfx2as data (prefix-to-AS mappings)...");
            info!("Pfx2as data is empty, loading...");
            match self.refresh_pfx2as() {
                Ok(count) => {
                    summary.add(
                        "pfx2as",
                        true,
                        format!("Pfx2as data loaded: {} prefixes", count),
                        Some(count),
                    );
                }
                Err(e) => {
                    summary.add(
                        "pfx2as",
                        false,
                        format!("Failed to load Pfx2as: {}", e),
                        None,
                    );
                }
            }
        } else if self.db.pfx2as().needs_refresh(DEFAULT_PFX2AS_CACHE_TTL) {
            eprintln!("[monocle] Refreshing Pfx2as data (prefix-to-AS mappings)...");
            info!("Pfx2as data is stale, refreshing...");
            match self.refresh_pfx2as() {
                Ok(count) => {
                    summary.add(
                        "pfx2as",
                        true,
                        format!("Pfx2as data refreshed: {} prefixes", count),
                        Some(count),
                    );
                }
                Err(e) => {
                    summary.add(
                        "pfx2as",
                        false,
                        format!("Failed to refresh Pfx2as: {}", e),
                        None,
                    );
                }
            }
        }

        Ok(summary)
    }

    /// Ensure only the data sources needed for specific sections are available
    ///
    /// This is more efficient than `ensure_data_available()` when you know
    /// which sections you need. It only loads/refreshes the required data sources.
    pub fn ensure_data_for_sections(
        &self,
        sections: &HashSet<InspectDataSection>,
    ) -> Result<DataRefreshSummary> {
        let mut summary = DataRefreshSummary::new();

        // ASInfo is always needed for basic information
        if sections.contains(&InspectDataSection::Basic) {
            if self.db.asinfo().is_empty() {
                eprintln!("[monocle] Loading ASInfo data (AS names, organizations, PeeringDB)...");
                match self.db.bootstrap_asinfo() {
                    Ok(counts) => {
                        summary.add(
                            "asinfo",
                            true,
                            format!(
                                "ASInfo data loaded: {} core, {} as2org, {} peeringdb",
                                counts.core, counts.as2org, counts.peeringdb
                            ),
                            Some(counts.core),
                        );
                    }
                    Err(e) => {
                        summary.add(
                            "asinfo",
                            false,
                            format!("Failed to load ASInfo: {}", e),
                            None,
                        );
                    }
                }
            } else if self.db.needs_asinfo_refresh() {
                eprintln!(
                    "[monocle] Refreshing ASInfo data (AS names, organizations, PeeringDB)..."
                );
                match self.db.bootstrap_asinfo() {
                    Ok(counts) => {
                        summary.add(
                            "asinfo",
                            true,
                            format!(
                                "ASInfo data refreshed: {} core, {} as2org, {} peeringdb",
                                counts.core, counts.as2org, counts.peeringdb
                            ),
                            Some(counts.core),
                        );
                    }
                    Err(e) => {
                        summary.add(
                            "asinfo",
                            false,
                            format!("Failed to refresh ASInfo: {}", e),
                            None,
                        );
                    }
                }
            }
        }

        // AS2Rel is needed for connectivity section
        if sections.contains(&InspectDataSection::Connectivity) {
            if self.db.as2rel().is_empty() {
                eprintln!("[monocle] Loading AS2Rel data (AS relationships)...");
                match self.db.update_as2rel() {
                    Ok(count) => {
                        summary.add(
                            "as2rel",
                            true,
                            format!("AS2Rel data loaded: {} relationships", count),
                            Some(count),
                        );
                    }
                    Err(e) => {
                        summary.add(
                            "as2rel",
                            false,
                            format!("Failed to load AS2Rel: {}", e),
                            None,
                        );
                    }
                }
            } else if self.db.needs_as2rel_update() {
                eprintln!("[monocle] Refreshing AS2Rel data (AS relationships)...");
                match self.db.update_as2rel() {
                    Ok(count) => {
                        summary.add(
                            "as2rel",
                            true,
                            format!("AS2Rel data refreshed: {} relationships", count),
                            Some(count),
                        );
                    }
                    Err(e) => {
                        summary.add(
                            "as2rel",
                            false,
                            format!("Failed to refresh AS2Rel: {}", e),
                            None,
                        );
                    }
                }
            }
        }

        // RPKI is needed for rpki section
        if sections.contains(&InspectDataSection::Rpki) {
            if self.db.rpki().is_empty() {
                eprintln!("[monocle] Loading RPKI data (ROAs, ASPA)...");
                match self.refresh_rpki_from_commons() {
                    Ok(count) => {
                        summary.add(
                            "rpki",
                            true,
                            format!("RPKI data loaded: {} ROAs", count),
                            Some(count),
                        );
                    }
                    Err(e) => {
                        summary.add("rpki", false, format!("Failed to load RPKI: {}", e), None);
                    }
                }
            } else if self.db.rpki().needs_refresh(DEFAULT_RPKI_CACHE_TTL) {
                eprintln!("[monocle] Refreshing RPKI data (ROAs, ASPA)...");
                match self.refresh_rpki_from_commons() {
                    Ok(count) => {
                        summary.add(
                            "rpki",
                            true,
                            format!("RPKI data refreshed: {} ROAs", count),
                            Some(count),
                        );
                    }
                    Err(e) => {
                        summary.add(
                            "rpki",
                            false,
                            format!("Failed to refresh RPKI: {}", e),
                            None,
                        );
                    }
                }
            }
        }

        // Pfx2as is needed for prefixes section
        if sections.contains(&InspectDataSection::Prefixes) {
            if self.db.pfx2as().is_empty() {
                eprintln!("[monocle] Loading Pfx2as data (prefix-to-AS mappings)...");
                match self.refresh_pfx2as() {
                    Ok(count) => {
                        summary.add(
                            "pfx2as",
                            true,
                            format!("Pfx2as data loaded: {} prefixes", count),
                            Some(count),
                        );
                    }
                    Err(e) => {
                        summary.add(
                            "pfx2as",
                            false,
                            format!("Failed to load Pfx2as: {}", e),
                            None,
                        );
                    }
                }
            } else if self.db.pfx2as().needs_refresh(DEFAULT_PFX2AS_CACHE_TTL) {
                eprintln!("[monocle] Refreshing Pfx2as data (prefix-to-AS mappings)...");
                match self.refresh_pfx2as() {
                    Ok(count) => {
                        summary.add(
                            "pfx2as",
                            true,
                            format!("Pfx2as data refreshed: {} prefixes", count),
                            Some(count),
                        );
                    }
                    Err(e) => {
                        summary.add(
                            "pfx2as",
                            false,
                            format!("Failed to refresh Pfx2as: {}", e),
                            None,
                        );
                    }
                }
            }
        }

        Ok(summary)
    }

    /// Refresh RPKI data from bgpkit-commons
    fn refresh_rpki_from_commons(&self) -> Result<usize> {
        use crate::lens::rpki::RpkiLens;

        // Use RpkiLens with database reference - it handles the refresh internally
        let lens = RpkiLens::new(self.db);
        let (roa_count, _aspa_count) = lens.refresh()?;

        Ok(roa_count)
    }

    /// Refresh Pfx2as data with RPKI validation
    ///
    /// This method loads pfx2as data and validates each prefix-ASN pair against
    /// the RPKI data in the database. The validation status is stored alongside
    /// each record.
    fn refresh_pfx2as(&self) -> Result<usize> {
        use crate::database::Pfx2asDbRecord;

        use ipnet::IpNet;
        use std::str::FromStr;

        const PFX2AS_URL: &str = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";

        #[derive(serde::Deserialize)]
        struct Pfx2asEntry {
            prefix: String,
            asn: u32,
        }

        info!("Loading pfx2as data from {}...", PFX2AS_URL);
        let entries: Vec<Pfx2asEntry> = oneio::read_json_struct(PFX2AS_URL)?;

        // Filter out invalid /0 prefixes (0.0.0.0/0 and ::/0)
        let entries: Vec<Pfx2asEntry> = entries
            .into_iter()
            .filter(|e| !e.prefix.ends_with("/0"))
            .collect();
        let entry_count = entries.len();

        // Load RPKI trie for validation using bgpkit-commons directly
        info!("Loading RPKI data for validation...");
        let trie = crate::lens::rpki::commons::load_current_rpki().ok();

        info!("Validating {} pfx2as records against RPKI...", entry_count);

        let records: Vec<Pfx2asDbRecord> = entries
            .into_iter()
            .map(|e| {
                let validation = if let Some(trie) = &trie {
                    // Parse prefix and validate against RPKI trie
                    if let Ok(prefix) = IpNet::from_str(&e.prefix) {
                        let roas = trie.lookup_by_prefix(&prefix);
                        if roas.is_empty() {
                            "unknown".to_string()
                        } else {
                            // Get prefix length
                            let prefix_len = prefix.prefix_len();
                            // Check if any ROA validates this announcement
                            let is_valid = roas
                                .iter()
                                .any(|roa| roa.asn == e.asn && prefix_len <= roa.max_length);
                            if is_valid {
                                "valid".to_string()
                            } else {
                                "invalid".to_string()
                            }
                        }
                    } else {
                        "unknown".to_string()
                    }
                } else {
                    "unknown".to_string()
                };

                Pfx2asDbRecord {
                    prefix: e.prefix,
                    origin_asn: e.asn,
                    validation,
                }
            })
            .collect();

        let count = records.len();
        self.db.pfx2as().store(&records, PFX2AS_URL)?;

        Ok(count)
    }

    // =========================================================================
    // Main Query Interface
    // =========================================================================

    /// Process multiple mixed queries
    ///
    /// This is the main entry point for the inspect lens. It accepts multiple
    /// queries of mixed types (ASN, prefix, name) and returns unified results.
    pub fn query(&self, inputs: &[String], options: &InspectQueryOptions) -> Result<InspectResult> {
        let start = Instant::now();
        let mut results = Vec::new();
        let mut asn_count = 0;
        let mut prefix_count = 0;
        let mut name_count = 0;

        for input in inputs {
            let query_type = self.detect_query_type(input);

            match query_type {
                InspectQueryType::Asn => {
                    asn_count += 1;
                    let result = self.query_asn(input, options)?;
                    results.push(result);
                }
                InspectQueryType::Prefix => {
                    prefix_count += 1;
                    let result = self.query_prefix(input, options)?;
                    results.push(result);
                }
                InspectQueryType::Name => {
                    name_count += 1;
                    let result = self.query_name(input, options)?;
                    results.push(result);
                }
            }
        }

        let elapsed = start.elapsed();

        Ok(InspectResult {
            queries: results,
            meta: InspectResultMeta {
                query_count: inputs.len(),
                asn_queries: asn_count,
                prefix_queries: prefix_count,
                name_queries: name_count,
                processing_time_ms: elapsed.as_millis() as u64,
            },
        })
    }

    /// Force query as ASN
    pub fn query_as_asn(
        &self,
        inputs: &[String],
        options: &InspectQueryOptions,
    ) -> Result<InspectResult> {
        let start = Instant::now();
        let mut results = Vec::new();

        for input in inputs {
            let result = self.query_asn(input, options)?;
            results.push(result);
        }

        let elapsed = start.elapsed();

        Ok(InspectResult {
            queries: results,
            meta: InspectResultMeta {
                query_count: inputs.len(),
                asn_queries: inputs.len(),
                prefix_queries: 0,
                name_queries: 0,
                processing_time_ms: elapsed.as_millis() as u64,
            },
        })
    }

    /// Force query as prefix
    pub fn query_as_prefix(
        &self,
        inputs: &[String],
        options: &InspectQueryOptions,
    ) -> Result<InspectResult> {
        let start = Instant::now();
        let mut results = Vec::new();

        for input in inputs {
            let result = self.query_prefix(input, options)?;
            results.push(result);
        }

        let elapsed = start.elapsed();

        Ok(InspectResult {
            queries: results,
            meta: InspectResultMeta {
                query_count: inputs.len(),
                asn_queries: 0,
                prefix_queries: inputs.len(),
                name_queries: 0,
                processing_time_ms: elapsed.as_millis() as u64,
            },
        })
    }

    /// Force query as name search
    pub fn query_as_name(
        &self,
        inputs: &[String],
        options: &InspectQueryOptions,
    ) -> Result<InspectResult> {
        let start = Instant::now();
        let mut results = Vec::new();

        for input in inputs {
            let result = self.query_name(input, options)?;
            results.push(result);
        }

        let elapsed = start.elapsed();

        Ok(InspectResult {
            queries: results,
            meta: InspectResultMeta {
                query_count: inputs.len(),
                asn_queries: 0,
                prefix_queries: 0,
                name_queries: inputs.len(),
                processing_time_ms: elapsed.as_millis() as u64,
            },
        })
    }

    /// Query by country code
    pub fn query_by_country(
        &self,
        country: &str,
        options: &InspectQueryOptions,
    ) -> Result<InspectResult> {
        let start = Instant::now();

        // Resolve country code
        let countries = self.country_lookup.lookup(country);
        let country_code = if countries.is_empty() {
            // Assume it's already a country code
            country.to_uppercase()
        } else if countries.len() == 1 {
            countries[0].code.clone()
        } else {
            return Err(anyhow!(
                "Multiple countries match '{}': {}",
                country,
                countries
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };

        let limit = if options.max_search_results > 0 {
            options.max_search_results
        } else {
            1000 // Default limit for country search
        };

        let core_results = self.db.asinfo().search_by_country(&country_code, limit)?;
        let total = core_results.len();
        let truncated = total >= limit;

        let elapsed = start.elapsed();

        Ok(InspectResult {
            queries: vec![InspectQueryResult {
                query: format!("country:{}", country_code),
                query_type: InspectQueryType::Name,
                asinfo: None,
                prefix: None,
                prefixes: None,
                connectivity: None,
                rpki: None,
                search_results: Some(SearchResultsSection {
                    total_matches: total,
                    results: core_results,
                    truncated,
                }),
            }],
            meta: InspectResultMeta {
                query_count: 1,
                asn_queries: 0,
                prefix_queries: 0,
                name_queries: 1,
                processing_time_ms: elapsed.as_millis() as u64,
            },
        })
    }

    // =========================================================================
    // Internal Query Methods
    // =========================================================================

    /// Query information for an ASN
    fn query_asn(&self, input: &str, options: &InspectQueryOptions) -> Result<InspectQueryResult> {
        let asn = self.parse_asn(input)?;
        let mut result = InspectQueryResult::new_asn(input.to_string());

        // Get ASInfo data if basic section is requested
        if options.should_include(InspectDataSection::Basic, InspectQueryType::Asn) {
            if let Ok(Some(full_record)) = self.db.asinfo().get_full(asn) {
                result.asinfo = Some(AsinfoSection {
                    detail: Some(full_record),
                    origins: None,
                });
            }
        }

        // Get connectivity data
        if options.should_include(InspectDataSection::Connectivity, InspectQueryType::Asn) {
            if let Some(connectivity) = self.get_connectivity_for_asn(asn, options) {
                result.connectivity = Some(connectivity);
            }
        }

        // Get RPKI data (ROAs and ASPA for this ASN)
        let include_rpki = options.should_include(InspectDataSection::Rpki, InspectQueryType::Asn);

        if include_rpki {
            let rpki_info = self.get_rpki_for_asn(asn, options, true, true);
            if rpki_info.roas.is_some() || rpki_info.aspa.is_some() {
                result.rpki = Some(rpki_info);
            }
        }

        // Get announced prefixes
        if options.should_include(InspectDataSection::Prefixes, InspectQueryType::Asn) {
            if let Some(prefixes) = self.get_prefixes_for_asn(asn, options) {
                result.prefixes = Some(prefixes);
            }
        }

        Ok(result)
    }

    /// Query information for a prefix
    fn query_prefix(
        &self,
        input: &str,
        options: &InspectQueryOptions,
    ) -> Result<InspectQueryResult> {
        let prefix_str = self.normalize_prefix(input)?;
        let mut result = InspectQueryResult::new_prefix(input.to_string());

        // Get prefix-to-AS mapping
        let pfx2as_info = self.get_pfx2as_for_prefix(&prefix_str)?;

        // Get RPKI info for the prefix
        let rpki_info =
            if options.should_include(InspectDataSection::Rpki, InspectQueryType::Prefix) {
                Some(self.get_rpki_for_prefix(&prefix_str, &pfx2as_info, options))
            } else {
                None
            };

        result.prefix = Some(PrefixSection {
            pfx2as: pfx2as_info,
            rpki: rpki_info,
        });

        // Get ASInfo for origin ASNs
        if options.should_include(InspectDataSection::Basic, InspectQueryType::Prefix) {
            if let Some(pfx2as) = result.prefix.as_ref().and_then(|p| p.pfx2as.as_ref()) {
                if !pfx2as.origin_asns.is_empty() {
                    let origins = self.db.asinfo().get_full_batch(&pfx2as.origin_asns)?;
                    if !origins.is_empty() {
                        result.asinfo = Some(AsinfoSection {
                            detail: None,
                            origins: Some(origins),
                        });
                    }
                }
            }
        }

        Ok(result)
    }

    /// Query by name search
    fn query_name(&self, input: &str, options: &InspectQueryOptions) -> Result<InspectQueryResult> {
        let mut result = InspectQueryResult::new_name(input.to_string());

        let limit = if options.max_search_results > 0 {
            options.max_search_results
        } else {
            100 // Reasonable default for name search
        };

        let search_results = self.db.asinfo().search_by_text(input, limit + 1)?;
        let total = search_results.len();
        let truncated = total > limit;

        let results: Vec<AsinfoCoreRecord> = if truncated {
            search_results.into_iter().take(limit).collect()
        } else {
            search_results
        };

        result.search_results = Some(SearchResultsSection {
            total_matches: if truncated { total } else { results.len() },
            results,
            truncated,
        });

        Ok(result)
    }

    // =========================================================================
    // Query Type Detection
    // =========================================================================

    /// Detect the query type from input string
    ///
    /// Detection rules:
    /// - `ASxxxx` or pure digits -> ASN
    /// - Contains `/` -> Prefix (CIDR)
    /// - Valid IPv4 without `/` -> /32 prefix
    /// - Valid IPv6 without `/` -> /128 prefix
    /// - Everything else -> Name search
    pub fn detect_query_type(&self, input: &str) -> InspectQueryType {
        let trimmed = input.trim();

        // Check for AS prefix (case-insensitive)
        if trimmed.to_uppercase().starts_with("AS") && trimmed[2..].parse::<u32>().is_ok() {
            return InspectQueryType::Asn;
        }

        // Check for pure numeric (ASN)
        if trimmed.parse::<u32>().is_ok() {
            return InspectQueryType::Asn;
        }

        // Check for CIDR notation
        if trimmed.contains('/') {
            return InspectQueryType::Prefix;
        }

        // Check for IP address (without CIDR)
        if let Ok(ip) = trimmed.parse::<IpAddr>() {
            // IP without CIDR is treated as a single-host prefix
            match ip {
                IpAddr::V4(_) => return InspectQueryType::Prefix,
                IpAddr::V6(_) => return InspectQueryType::Prefix,
            }
        }

        // Default to name search
        InspectQueryType::Name
    }

    // =========================================================================
    // Helper Methods
    // =========================================================================

    /// Parse ASN from various formats
    fn parse_asn(&self, input: &str) -> Result<u32> {
        let trimmed = input.trim();

        // Handle "ASxxxx" format
        if trimmed.to_uppercase().starts_with("AS") {
            return trimmed[2..]
                .parse::<u32>()
                .map_err(|_| anyhow!("Invalid ASN: {}", input));
        }

        // Handle pure numeric
        trimmed
            .parse::<u32>()
            .map_err(|_| anyhow!("Invalid ASN: {}", input))
    }

    /// Normalize prefix to standard CIDR notation
    fn normalize_prefix(&self, input: &str) -> Result<String> {
        let trimmed = input.trim();

        // If already has CIDR notation, validate and return
        if trimmed.contains('/') {
            // Basic validation
            let parts: Vec<&str> = trimmed.split('/').collect();
            if parts.len() != 2 {
                return Err(anyhow!("Invalid prefix format: {}", input));
            }

            let _ip: IpAddr = parts[0]
                .parse()
                .map_err(|_| anyhow!("Invalid IP address in prefix: {}", input))?;
            let _prefix_len: u8 = parts[1]
                .parse()
                .map_err(|_| anyhow!("Invalid prefix length: {}", input))?;

            return Ok(trimmed.to_string());
        }

        // Add appropriate prefix length for bare IP addresses
        let ip: IpAddr = trimmed
            .parse()
            .map_err(|_| anyhow!("Invalid IP address: {}", input))?;

        match ip {
            IpAddr::V4(_) => Ok(format!("{}/32", trimmed)),
            IpAddr::V6(_) => Ok(format!("{}/128", trimmed)),
        }
    }

    /// Get connectivity summary for an ASN
    fn get_connectivity_for_asn(
        &self,
        asn: u32,
        options: &InspectQueryOptions,
    ) -> Option<ConnectivitySection> {
        let as2rel = self.db.as2rel();

        if as2rel.is_empty() {
            return None;
        }

        let max_neighbors = if options.max_neighbors > 0 {
            options.max_neighbors
        } else {
            100 // Reasonable default
        };

        // Use the repository's get_connectivity_summary method
        let asinfo = self.db.asinfo();
        let name_lookup =
            |asns: &[u32]| -> HashMap<u32, String> { asinfo.lookup_preferred_names_batch(asns) };

        let summary = match as2rel.get_connectivity_summary(asn, max_neighbors, name_lookup) {
            Ok(Some(s)) => s,
            Ok(None) => return None,
            Err(_) => return None,
        };

        // Check if truncated
        let truncated = as2rel
            .would_truncate_connectivity(asn, max_neighbors)
            .unwrap_or(false);

        Some(ConnectivitySection { summary, truncated })
    }

    /// Get RPKI information for an ASN
    fn get_rpki_for_asn(
        &self,
        asn: u32,
        options: &InspectQueryOptions,
        include_roas: bool,
        include_aspa: bool,
    ) -> RpkiAsnInfo {
        let rpki = self.db.rpki();
        let mut rpki_info = RpkiAsnInfo {
            roas: None,
            aspa: None,
        };

        if include_roas {
            if let Ok(roas) = rpki.get_roas_by_asn(asn) {
                let total_count = roas.len();
                let ipv4_count = roas.iter().filter(|r| !r.prefix.contains(':')).count();
                let ipv6_count = total_count - ipv4_count;

                let max_roas = if options.max_roas > 0 {
                    options.max_roas
                } else {
                    total_count
                };

                let truncated = total_count > max_roas;
                let entries: Vec<_> = roas.into_iter().take(max_roas).collect();

                rpki_info.roas = Some(RoaSummary {
                    total_count,
                    ipv4_count,
                    ipv6_count,
                    entries,
                    truncated,
                });
            }
        }

        if include_aspa {
            if let Ok(aspa_records) = rpki.get_aspas_by_customer(asn) {
                if let Some(aspa_record) = aspa_records.into_iter().next() {
                    // Get customer AS info
                    let (customer_name, customer_country) = self
                        .db
                        .asinfo()
                        .get_core(aspa_record.customer_asn)
                        .ok()
                        .flatten()
                        .map(|r| {
                            (
                                self.db.asinfo().lookup_preferred_name(r.asn),
                                Some(r.country),
                            )
                        })
                        .unwrap_or((None, None));

                    // Get providers with names
                    let provider_names = self
                        .db
                        .asinfo()
                        .lookup_preferred_names_batch(&aspa_record.provider_asns);
                    let providers: Vec<AspaProvider> = aspa_record
                        .provider_asns
                        .iter()
                        .map(|asn| AspaProvider {
                            asn: *asn,
                            name: provider_names.get(asn).cloned(),
                        })
                        .collect();

                    rpki_info.aspa = Some(AspaInfo {
                        customer_asn: aspa_record.customer_asn,
                        customer_name,
                        customer_country,
                        providers,
                    });
                }
            }
        }

        rpki_info
    }

    /// Get RPKI information for a prefix
    fn get_rpki_for_prefix(
        &self,
        prefix: &str,
        pfx2as_info: &Option<Pfx2asInfo>,
        options: &InspectQueryOptions,
    ) -> RpkiPrefixInfo {
        let rpki = self.db.rpki();

        let roas = rpki.get_covering_roas(prefix).unwrap_or_default();
        let roa_count = roas.len();

        let max_roas = if options.max_roas > 0 {
            options.max_roas
        } else {
            roa_count
        };

        let truncated = roa_count > max_roas;
        let entries: Vec<_> = roas.into_iter().take(max_roas).collect();

        // Determine validation state if we have a single origin ASN
        let validation_state = pfx2as_info.as_ref().and_then(|info| {
            if info.origin_asns.len() == 1 {
                let origin_asn = info.origin_asns[0];
                rpki.validate(prefix, origin_asn)
                    .ok()
                    .map(|(state, _roas)| state.to_string())
            } else {
                None
            }
        });

        RpkiPrefixInfo {
            roas: entries,
            roa_count,
            validation_state,
            truncated,
        }
    }

    /// Get prefix-to-AS mapping for a prefix
    fn get_pfx2as_for_prefix(&self, prefix: &str) -> Result<Option<Pfx2asInfo>> {
        let pfx2as = self.db.pfx2as();

        if pfx2as.is_empty() {
            return Ok(None);
        }

        // Helper to get validation status for each origin ASN
        let get_validations = |prefix_str: &str, asns: &[u32]| -> Vec<String> {
            let rpki = self.db.rpki();
            asns.iter()
                .map(|asn| {
                    rpki.validate(prefix_str, *asn)
                        .map(|(state, _)| state.to_string())
                        .unwrap_or_else(|_| "unknown".to_string())
                })
                .collect()
        };

        // Try exact match first
        if let Ok(asns) = pfx2as.lookup_exact(prefix) {
            if !asns.is_empty() {
                let validations = get_validations(prefix, &asns);
                return Ok(Some(Pfx2asInfo {
                    prefix: prefix.to_string(),
                    origin_asns: asns,
                    match_type: "exact".to_string(),
                    validations,
                }));
            }
        }

        // Fall back to longest prefix match
        if let Ok(result) = pfx2as.lookup_longest(prefix) {
            let validations = get_validations(&result.prefix, &result.origin_asns);
            return Ok(Some(Pfx2asInfo {
                prefix: result.prefix,
                origin_asns: result.origin_asns,
                match_type: "longest".to_string(),
                validations,
            }));
        }

        Ok(None)
    }

    /// Get announced prefixes for an ASN
    fn get_prefixes_for_asn(
        &self,
        asn: u32,
        options: &InspectQueryOptions,
    ) -> Option<AnnouncedPrefixesSection> {
        use crate::lens::inspect::types::{PrefixEntry, ValidationSummary};

        let pfx2as = self.db.pfx2as();

        if pfx2as.is_empty() {
            return None;
        }

        let prefixes = match pfx2as.get_by_asn(asn) {
            Ok(records) => records,
            Err(_) => return None,
        };

        if prefixes.is_empty() {
            return None;
        }

        let total_count = prefixes.len();
        let ipv4_count = prefixes.iter().filter(|p| !p.prefix.contains(':')).count();
        let ipv6_count = total_count - ipv4_count;

        // Calculate validation summary
        let valid_count = prefixes.iter().filter(|p| p.validation == "valid").count();
        let invalid_count = prefixes
            .iter()
            .filter(|p| p.validation == "invalid")
            .count();
        let unknown_count = prefixes
            .iter()
            .filter(|p| p.validation != "valid" && p.validation != "invalid")
            .count();
        let validation_summary =
            ValidationSummary::from_counts(valid_count, invalid_count, unknown_count);

        let max_prefixes = if options.max_prefixes > 0 {
            options.max_prefixes
        } else {
            total_count
        };

        let truncated = total_count > max_prefixes;

        // Sort by validation status (invalid first, then unknown, then valid), then by prefix
        let mut sorted_prefixes = prefixes;
        sorted_prefixes.sort_by(|a, b| {
            let order = |v: &str| match v {
                "invalid" => 0,
                "unknown" => 1,
                "valid" => 2,
                _ => 3,
            };
            order(&a.validation)
                .cmp(&order(&b.validation))
                .then_with(|| a.prefix.cmp(&b.prefix))
        });

        // Get AS info for the queried ASN
        let asinfo = self.db.asinfo();
        let origin_info = asinfo.get_core(asn).ok().flatten();
        let origin_name = asinfo
            .lookup_preferred_name(asn)
            .or_else(|| origin_info.as_ref().map(|i| i.name.clone()));
        let origin_country = origin_info.as_ref().map(|i| i.country.clone());

        let prefix_entries: Vec<PrefixEntry> = sorted_prefixes
            .into_iter()
            .take(max_prefixes)
            .map(|p| PrefixEntry {
                prefix: p.prefix,
                origin_asn: asn,
                origin_name: origin_name.clone(),
                origin_country: origin_country.clone(),
                validation: p.validation,
            })
            .collect();

        Some(AnnouncedPrefixesSection {
            total_count,
            ipv4_count,
            ipv6_count,
            validation_summary,
            prefixes: prefix_entries,
            truncated,
        })
    }

    // =========================================================================
    // Quick Lookups (for enrichment in other commands)
    // =========================================================================

    /// Lookup AS name by ASN
    pub fn lookup_name(&self, asn: u32) -> Option<String> {
        self.db.asinfo().lookup_preferred_name(asn)
    }

    /// Lookup organization name by ASN
    pub fn lookup_org(&self, asn: u32) -> Option<String> {
        self.db
            .asinfo()
            .get_as2org(asn)
            .ok()
            .flatten()
            .map(|r| r.org_name)
    }

    /// Batch lookup of AS names
    pub fn lookup_names_batch(&self, asns: &[u32]) -> HashMap<u32, String> {
        self.db.asinfo().lookup_preferred_names_batch(asns)
    }

    // =========================================================================
    // Formatting Methods
    // =========================================================================

    /// Format results as JSON
    pub fn format_json(&self, result: &InspectResult, pretty: bool) -> String {
        if pretty {
            serde_json::to_string_pretty(result).unwrap_or_default()
        } else {
            serde_json::to_string(result).unwrap_or_default()
        }
    }

    /// Format results as table for terminal display
    pub fn format_table(&self, result: &InspectResult, config: &InspectDisplayConfig) -> String {
        let mut output = String::new();
        let divider = if config.use_markdown_style {
            "\n\n---\n\n"
        } else {
            "\n\n════════════════════════════════════════\n════════════════════════════════════════\n\n"
        };

        // Separate ASN and non-ASN (prefix/name) results
        let asn_results: Vec<_> = result
            .queries
            .iter()
            .filter(|q| q.query_type == InspectQueryType::Asn)
            .collect();
        let other_results: Vec<_> = result
            .queries
            .iter()
            .filter(|q| q.query_type != InspectQueryType::Asn)
            .collect();

        // Automatically show glance table when there are multiple ASN results (table output only)
        let show_glance = asn_results.len() > 1;
        if show_glance {
            let glance_table = self.format_glance_table(&asn_results, config);
            if !glance_table.is_empty() {
                // output.push_str("─── Glance ───\n");
                output.push_str(&glance_table);
            }
        }

        // Always show ASN results with detailed info
        if !asn_results.is_empty() {
            // Show each ASN query result with its sections
            // Use dividers between different ASN queries, not between sections of the same query
            for (idx, query_result) in asn_results.iter().enumerate() {
                // Add divider between different queries (not for the first one, or after glance)
                if idx > 0 || (show_glance && !output.is_empty()) {
                    output.push_str(divider);
                }

                // Always show query meta header
                output.push_str(&format!(
                    "Query: {} (type: {})\n",
                    query_result.query, query_result.query_type
                ));

                // Basic information section with row-based format (no truncation)
                let asn_info = self.format_asn_basic_rows(query_result, config);
                if !asn_info.is_empty() {
                    output.push_str("─── Basic Information ───\n");
                    output.push_str(&asn_info);
                }

                // Prefixes section
                if let Some(ref prefixes) = query_result.prefixes {
                    if !output.is_empty() {
                        output.push_str("\n\n");
                    }
                    output.push_str(&self.format_prefixes_section(prefixes, config));
                }

                // Connectivity section
                if let Some(ref connectivity) = query_result.connectivity {
                    if !output.is_empty() {
                        output.push_str("\n\n");
                    }
                    output.push_str(&self.format_connectivity_section(connectivity, config));
                }

                // RPKI section
                if let Some(ref rpki) = query_result.rpki {
                    if !output.is_empty() {
                        output.push_str("\n\n");
                    }
                    output.push_str(&self.format_rpki_asn_section(rpki, config));
                }
            }
        }

        // Show non-ASN results (prefixes, names) separately
        for query_result in &other_results {
            let section = self.format_single_result(query_result, config);
            if !section.is_empty() {
                if !output.is_empty() {
                    output.push_str(divider);
                }
                output.push_str(&section);
            }
        }

        output
    }

    /// Format ASN basic info as rows (not a table) with full names (no truncation)
    fn format_asn_basic_rows(
        &self,
        query_result: &InspectQueryResult,
        config: &InspectDisplayConfig,
    ) -> String {
        let asinfo = match query_result.asinfo.as_ref() {
            Some(a) => a,
            None => return String::new(),
        };
        let detail = match asinfo.detail.as_ref() {
            Some(d) => d,
            None => return String::new(),
        };

        let mut lines = Vec::new();

        // Core info - full names, no truncation
        lines.push(format!("ASN:     AS{}", detail.core.asn));
        lines.push(format!(
            "Name:    {}",
            self.preferred_name_from_full(detail)
        ));
        lines.push(format!("Country: {}", detail.core.country));

        if let Some(ref as2org) = detail.as2org {
            lines.push(format!("Org:     {}", as2org.org_name));
            lines.push(format!("Org ID:  {}", as2org.org_id));
        }

        // Always show peeringdb info if available (part of basic info)
        if let Some(ref pdb) = detail.peeringdb {
            if let Some(ref website) = pdb.website {
                lines.push(format!("Website: {}", website));
            }
            if let Some(ref irr) = pdb.irr_as_set {
                lines.push(format!("AS-SET:  {}", irr));
            }
        }

        if config.should_show_hegemony() {
            if let Some(ref heg) = detail.hegemony {
                lines.push(format!(
                    "Hegemony: IPv4={:.4}, IPv6={:.4}",
                    heg.ipv4, heg.ipv6
                ));
            }
        }

        if config.should_show_population() {
            if let Some(ref pop) = detail.population {
                lines.push(format!(
                    "Population: {:.2}% country, {:.4}% global ({} users)",
                    pop.percent_country, pop.percent_global, pop.user_count
                ));
            }
        }

        lines.join("\n")
    }

    /// Format glance table - quick overview of all ASNs in a single table
    fn format_glance_table(
        &self,
        asn_results: &[&InspectQueryResult],
        config: &InspectDisplayConfig,
    ) -> String {
        // Filter to only those with asinfo data
        let asn_results: Vec<_> = asn_results
            .iter()
            .filter(|q| q.asinfo.is_some())
            .copied()
            .collect();

        if asn_results.is_empty() {
            return "No ASN results to display".to_string();
        }

        // For markdown, use simple 4-column format (ASN, Name, Country, Org)
        if config.use_markdown_style {
            let mut lines = Vec::new();

            // Header row
            lines.push("ASN | Name | Country | Org".to_string());
            lines.push("--- | --- | --- | ---".to_string());

            // Data rows
            for q in &asn_results {
                let asinfo = match q.asinfo.as_ref() {
                    Some(a) => a,
                    None => continue,
                };
                let detail = match asinfo.detail.as_ref() {
                    Some(d) => d,
                    None => continue,
                };

                let org = detail
                    .as2org
                    .as_ref()
                    .map(|a| self.truncate_name(&a.org_name, config))
                    .unwrap_or_else(|| "-".to_string());

                let row = [
                    format!("AS{}", detail.core.asn),
                    self.truncate_name(&self.preferred_name_from_full(detail), config),
                    detail.core.country.clone(),
                    org,
                ];

                lines.push(row.join(" | "));
            }

            if lines.len() <= 2 {
                return "No ASN data available".to_string();
            }

            return lines.join("\n");
        }

        // For non-markdown, use tabled with simple 4-column format
        #[derive(Tabled)]
        struct SimpleRow {
            #[tabled(rename = "ASN")]
            asn: String,
            #[tabled(rename = "Name")]
            name: String,
            #[tabled(rename = "Country")]
            country: String,
            #[tabled(rename = "Org")]
            org: String,
        }

        let rows: Vec<SimpleRow> = asn_results
            .iter()
            .filter_map(|q| {
                let asinfo = q.asinfo.as_ref()?;
                let detail = asinfo.detail.as_ref()?;
                let org = detail
                    .as2org
                    .as_ref()
                    .map(|a| self.truncate_name(&a.org_name, config))
                    .unwrap_or_else(|| "-".to_string());
                Some(SimpleRow {
                    asn: format!("AS{}", detail.core.asn),
                    name: self.truncate_name(&self.preferred_name_from_full(detail), config),
                    country: detail.core.country.clone(),
                    org,
                })
            })
            .collect();

        Table::new(&rows).with(Style::rounded()).to_string()
    }

    /// Format a single query result
    fn format_single_result(
        &self,
        result: &InspectQueryResult,
        config: &InspectDisplayConfig,
    ) -> String {
        let mut sections = Vec::new();

        // Header
        sections.push(format!(
            "Query: {} (type: {})",
            result.query, result.query_type
        ));

        // ASInfo section
        if let Some(ref asinfo) = result.asinfo {
            if let Some(ref detail) = asinfo.detail {
                sections.push(self.format_asinfo_detail(detail, config));
            }
            // Skip origins for prefix queries - the info is already in the announced prefix table
            if result.query_type != InspectQueryType::Prefix {
                if let Some(ref origins) = asinfo.origins {
                    sections.push(self.format_asinfo_origins(origins, config));
                }
            }
        }

        // Prefix section
        if let Some(ref prefix) = result.prefix {
            sections.push(self.format_prefix_section(prefix, config));
        }

        // Connectivity section
        if let Some(ref connectivity) = result.connectivity {
            sections.push(self.format_connectivity_section(connectivity, config));
        }

        // RPKI section (for ASN queries)
        if let Some(ref rpki) = result.rpki {
            sections.push(self.format_rpki_asn_section(rpki, config));
        }

        // Prefixes section
        if let Some(ref prefixes) = result.prefixes {
            sections.push(self.format_prefixes_section(prefixes, config));
        }

        // Search results
        if let Some(ref search) = result.search_results {
            sections.push(self.format_search_results(search, config));
        }

        sections.join("\n\n")
    }

    fn format_asinfo_detail(
        &self,
        detail: &AsinfoFullRecord,
        config: &InspectDisplayConfig,
    ) -> String {
        let mut lines = vec!["─── AS Information ───".to_string()];

        lines.push(format!("ASN:     AS{}", detail.core.asn));
        lines.push(format!(
            "Name:    {}",
            self.truncate_name(&self.preferred_name_from_full(detail), config)
        ));
        lines.push(format!("Country: {}", detail.core.country));

        if let Some(ref as2org) = detail.as2org {
            lines.push(format!(
                "Org:     {}",
                self.truncate_name(&as2org.org_name, config)
            ));
            lines.push(format!("Org ID:  {}", as2org.org_id));
        }

        if config.should_show_peeringdb() {
            if let Some(ref pdb) = detail.peeringdb {
                if let Some(ref website) = pdb.website {
                    lines.push(format!("Website: {}", website));
                }
                if let Some(ref irr) = pdb.irr_as_set {
                    lines.push(format!("AS-SET:  {}", irr));
                }
            }
        }

        if config.should_show_hegemony() {
            if let Some(ref heg) = detail.hegemony {
                lines.push(format!(
                    "Hegemony: IPv4={:.4}, IPv6={:.4}",
                    heg.ipv4, heg.ipv6
                ));
            }
        }

        if config.should_show_population() {
            if let Some(ref pop) = detail.population {
                lines.push(format!(
                    "Population: {:.2}% country, {:.4}% global ({} users)",
                    pop.percent_country, pop.percent_global, pop.user_count
                ));
            }
        }

        lines.join("\n")
    }

    fn format_asinfo_origins(
        &self,
        origins: &[AsinfoFullRecord],
        config: &InspectDisplayConfig,
    ) -> String {
        let mut lines = vec!["─── Origin ASNs ───".to_string()];

        #[derive(Tabled)]
        struct OriginRow {
            #[tabled(rename = "ASN")]
            asn: String,
            #[tabled(rename = "Name")]
            name: String,
            #[tabled(rename = "Country")]
            country: String,
        }

        let rows: Vec<OriginRow> = origins
            .iter()
            .map(|o| OriginRow {
                asn: format!("AS{}", o.core.asn),
                name: self.truncate_name(&self.preferred_name_from_full(o), config),
                country: o.core.country.clone(),
            })
            .collect();

        let table = if config.use_markdown_style {
            Table::new(rows).with(Style::markdown()).to_string()
        } else {
            Table::new(rows).with(Style::rounded()).to_string()
        };
        lines.push(table);

        lines.join("\n")
    }

    fn format_prefix_section(
        &self,
        prefix: &PrefixSection,
        config: &InspectDisplayConfig,
    ) -> String {
        let mut sections = Vec::new();

        // Section 1: Announced Prefix Info
        if let Some(ref pfx2as) = prefix.pfx2as {
            let mut lines = vec!["─── Announced Prefix ───".to_string()];

            #[derive(Tabled)]
            struct OriginRow {
                #[tabled(rename = "Matched Prefix")]
                matched_prefix: String,
                #[tabled(rename = "Match Type")]
                match_type: String,
                #[tabled(rename = "Origin ASN")]
                asn: String,
                #[tabled(rename = "Validation")]
                validation: String,
            }

            let rows: Vec<OriginRow> = pfx2as
                .origin_asns
                .iter()
                .zip(pfx2as.validations.iter())
                .map(|(asn, validation)| OriginRow {
                    matched_prefix: pfx2as.prefix.clone(),
                    match_type: pfx2as.match_type.clone(),
                    asn: format!("AS{}", asn),
                    validation: validation.clone(),
                })
                .collect();

            if !rows.is_empty() {
                let table = if config.use_markdown_style {
                    Table::new(rows).with(Style::markdown()).to_string()
                } else {
                    Table::new(rows).with(Style::rounded()).to_string()
                };
                lines.push(table);
            }

            sections.push(lines.join("\n"));
        }

        // Section 2: Covering ROAs
        if let Some(ref rpki) = prefix.rpki {
            let mut lines = vec![format!("─── Covering ROAs ({}) ───", rpki.roa_count)];

            if !rpki.roas.is_empty() {
                #[derive(Tabled)]
                struct RoaRow {
                    #[tabled(rename = "Prefix")]
                    prefix: String,
                    #[tabled(rename = "Max Length")]
                    max_length: u8,
                    #[tabled(rename = "Origin ASN")]
                    origin_asn: String,
                    #[tabled(rename = "TA")]
                    ta: String,
                }

                let mut rows: Vec<RoaRow> = rpki
                    .roas
                    .iter()
                    .map(|r| RoaRow {
                        prefix: r.prefix.clone(),
                        max_length: r.max_length,
                        origin_asn: format!("AS{}", r.origin_asn),
                        ta: r.ta.clone(),
                    })
                    .collect();

                // Add a visual indicator row when results are truncated
                if rpki.truncated {
                    rows.push(RoaRow {
                        prefix: "...".to_string(),
                        max_length: 0,
                        origin_asn: "...".to_string(),
                        ta: "...".to_string(),
                    });
                }

                let table = if config.use_markdown_style {
                    Table::new(rows).with(Style::markdown()).to_string()
                } else {
                    Table::new(rows).with(Style::rounded()).to_string()
                };
                lines.push(table);

                if rpki.truncated {
                    lines.push("(ROA list truncated, use --full-roas to show all)".to_string());
                }
            } else {
                lines.push("No covering ROAs found".to_string());
            }

            sections.push(lines.join("\n"));
        }

        sections.join("\n\n")
    }

    fn format_connectivity_section(
        &self,
        connectivity: &ConnectivitySection,
        config: &InspectDisplayConfig,
    ) -> String {
        let mut lines = vec!["─── Connectivity ───".to_string()];

        let summary = &connectivity.summary;

        // Add summary with percentages at the top
        lines.push(format!(
            "Total neighbors: {} (max visibility: {} peers)",
            summary.total_neighbors, summary.max_peers_count
        ));

        // Summary table for upstreams/peers/downstreams
        #[derive(Tabled)]
        struct SummaryRow {
            #[tabled(rename = "Relationship")]
            relationship: String,
            #[tabled(rename = "Count")]
            count: u32,
            #[tabled(rename = "% of Neighbors")]
            percent: String,
        }

        let summary_rows = vec![
            SummaryRow {
                relationship: "Upstreams".to_string(),
                count: summary.upstreams.count,
                percent: format!("{:.1}%", summary.upstreams.percent),
            },
            SummaryRow {
                relationship: "Peers".to_string(),
                count: summary.peers.count,
                percent: format!("{:.1}%", summary.peers.percent),
            },
            SummaryRow {
                relationship: "Downstreams".to_string(),
                count: summary.downstreams.count,
                percent: format!("{:.1}%", summary.downstreams.percent),
            },
        ];

        let summary_table = if config.use_markdown_style {
            Table::new(summary_rows).with(Style::markdown()).to_string()
        } else {
            Table::new(summary_rows).with(Style::rounded()).to_string()
        };
        lines.push(summary_table);

        let format_group = |name: &str, group: &ConnectivityGroup, truncated: bool| -> String {
            let mut group_lines =
                vec![format!("{}: {} ({:.1}%)", name, group.count, group.percent)];

            if !group.top.is_empty() {
                #[derive(Tabled)]
                struct NeighborRow {
                    #[tabled(rename = "ASN")]
                    asn: String,
                    #[tabled(rename = "Name")]
                    name: String,
                    #[tabled(rename = "Visibility")]
                    visibility: String,
                }

                let mut rows: Vec<NeighborRow> = group
                    .top
                    .iter()
                    .map(|e| NeighborRow {
                        asn: format!("AS{}", e.asn),
                        name: e
                            .name
                            .as_ref()
                            .map(|n| self.truncate_name(n, config))
                            .unwrap_or_else(|| "-".to_string()),
                        visibility: format!("{:.1}% ({})", e.peers_percent, e.peers_count),
                    })
                    .collect();

                // Add a visual indicator row when results are truncated and group has more items
                if truncated && group.count as usize > group.top.len() {
                    rows.push(NeighborRow {
                        asn: "...".to_string(),
                        name: "...".to_string(),
                        visibility: "...".to_string(),
                    });
                }

                let table = if config.use_markdown_style {
                    Table::new(rows).with(Style::markdown()).to_string()
                } else {
                    Table::new(rows).with(Style::rounded()).to_string()
                };
                group_lines.push(table);
            }

            group_lines.join("\n")
        };

        lines.push(format_group(
            "Upstreams",
            &summary.upstreams,
            connectivity.truncated,
        ));
        lines.push(format_group(
            "Peers",
            &summary.peers,
            connectivity.truncated,
        ));
        lines.push(format_group(
            "Downstreams",
            &summary.downstreams,
            connectivity.truncated,
        ));

        if connectivity.truncated {
            lines.push("(results truncated, use --full-connectivity to show all)".to_string());
        }

        lines.join("\n\n")
    }

    fn format_rpki_asn_section(&self, rpki: &RpkiAsnInfo, config: &InspectDisplayConfig) -> String {
        let mut lines = vec!["─── RPKI ───".to_string()];

        if let Some(ref roas) = rpki.roas {
            lines.push(format!(
                "ROAs: {} total ({} IPv4, {} IPv6)",
                roas.total_count, roas.ipv4_count, roas.ipv6_count
            ));

            if !roas.entries.is_empty() {
                #[derive(Tabled)]
                struct RoaRow {
                    #[tabled(rename = "Prefix")]
                    prefix: String,
                    #[tabled(rename = "Max Len")]
                    max_length: u8,
                    #[tabled(rename = "TA")]
                    ta: String,
                }

                let mut rows: Vec<RoaRow> = roas
                    .entries
                    .iter()
                    .map(|r| RoaRow {
                        prefix: r.prefix.clone(),
                        max_length: r.max_length,
                        ta: r.ta.clone(),
                    })
                    .collect();

                // Add a visual indicator row when results are truncated
                if roas.truncated {
                    rows.push(RoaRow {
                        prefix: "...".to_string(),
                        max_length: 0,
                        ta: "...".to_string(),
                    });
                }

                let table = if config.use_markdown_style {
                    Table::new(rows).with(Style::markdown()).to_string()
                } else {
                    Table::new(rows).with(Style::rounded()).to_string()
                };
                lines.push(table);

                if roas.truncated {
                    lines.push("(ROA list truncated, use --full-roas to show all)".to_string());
                }
            }
        }

        // ASPA section - show as table or "No ASPA" message
        if let Some(ref aspa) = rpki.aspa {
            lines.push(String::new()); // Empty line separator

            // Format customer info with name and country if available
            let customer_info = match (&aspa.customer_name, &aspa.customer_country) {
                (Some(name), Some(country)) => {
                    format!("AS{} - {} [{}]", aspa.customer_asn, name, country)
                }
                (Some(name), None) => format!("AS{} - {}", aspa.customer_asn, name),
                _ => format!("AS{}", aspa.customer_asn),
            };
            lines.push(format!(
                "ASPA: {} ({} providers)",
                customer_info,
                aspa.providers.len()
            ));

            #[derive(Tabled)]
            struct AspaProviderRow {
                #[tabled(rename = "Provider ASN")]
                asn: String,
                #[tabled(rename = "Provider Name")]
                name: String,
            }

            let rows: Vec<AspaProviderRow> = aspa
                .providers
                .iter()
                .map(|provider| AspaProviderRow {
                    asn: format!("AS{}", provider.asn),
                    name: match &provider.name {
                        Some(n) => self.truncate_name(n, config),
                        None => "—".to_string(),
                    },
                })
                .collect();

            let table = if config.use_markdown_style {
                Table::new(rows).with(Style::markdown()).to_string()
            } else {
                Table::new(rows).with(Style::rounded()).to_string()
            };
            lines.push(table);
        } else {
            lines.push(String::new()); // Empty line separator
            lines.push("ASPA: No ASPA record for this AS".to_string());
        }

        lines.join("\n")
    }

    fn format_prefixes_section(
        &self,
        prefixes: &AnnouncedPrefixesSection,
        config: &InspectDisplayConfig,
    ) -> String {
        let mut lines = vec!["─── Announced Prefixes ───".to_string()];

        lines.push(format!(
            "Total: {} ({} IPv4, {} IPv6)",
            prefixes.total_count, prefixes.ipv4_count, prefixes.ipv6_count
        ));

        // Validation summary
        let vs = &prefixes.validation_summary;
        lines.push(format!(
            "RPKI Validation: valid {} ({:.1}%), invalid {} ({:.1}%), unknown {} ({:.1}%)",
            vs.valid_count,
            vs.valid_percent,
            vs.invalid_count,
            vs.invalid_percent,
            vs.unknown_count,
            vs.unknown_percent
        ));

        if !prefixes.prefixes.is_empty() {
            #[derive(Tabled)]
            struct PrefixRow {
                #[tabled(rename = "Prefix")]
                prefix: String,
                #[tabled(rename = "Validation")]
                validation: String,
            }

            let mut rows: Vec<PrefixRow> = prefixes
                .prefixes
                .iter()
                .map(|p| PrefixRow {
                    prefix: p.prefix.clone(),
                    validation: p.validation.clone(),
                })
                .collect();

            // Add a visual indicator row when results are truncated
            if prefixes.truncated {
                rows.push(PrefixRow {
                    prefix: "...".to_string(),
                    validation: "...".to_string(),
                });
            }

            let table = if config.use_markdown_style {
                Table::new(rows).with(Style::markdown()).to_string()
            } else {
                Table::new(rows).with(Style::rounded()).to_string()
            };
            lines.push(table);
        }

        if prefixes.truncated {
            lines.push(format!(
                "(showing {} of {} prefixes, use --full-prefixes to show all)",
                prefixes.prefixes.len(),
                prefixes.total_count
            ));
        }

        lines.join("\n")
    }

    fn format_search_results(
        &self,
        search: &SearchResultsSection,
        config: &InspectDisplayConfig,
    ) -> String {
        let mut lines = vec!["─── Search Results ───".to_string()];

        lines.push(format!("Found: {} matches", search.total_matches));

        if !search.results.is_empty() {
            #[derive(Tabled)]
            struct SearchRow {
                #[tabled(rename = "ASN")]
                asn: String,
                #[tabled(rename = "Name")]
                name: String,
                #[tabled(rename = "Country")]
                country: String,
            }

            let preferred_names: HashMap<u32, String> =
                self.db.asinfo().lookup_preferred_names_batch(
                    &search.results.iter().map(|r| r.asn).collect::<Vec<_>>(),
                );

            let mut rows: Vec<SearchRow> = search
                .results
                .iter()
                .map(|r| SearchRow {
                    asn: format!("AS{}", r.asn),
                    name: self.truncate_name(
                        preferred_names.get(&r.asn).unwrap_or(&r.name).as_str(),
                        config,
                    ),
                    country: r.country.clone(),
                })
                .collect();

            // Add a visual indicator row when results are truncated
            if search.truncated {
                rows.push(SearchRow {
                    asn: "...".to_string(),
                    name: "...".to_string(),
                    country: "...".to_string(),
                });
            }

            let table = if config.use_markdown_style {
                Table::new(rows).with(Style::markdown()).to_string()
            } else {
                Table::new(rows).with(Style::rounded()).to_string()
            };
            lines.push(table);

            if search.truncated {
                lines.push("(results truncated, use --limit to show more)".to_string());
            }
        }

        lines.join("\n")
    }

    fn preferred_name_from_full(&self, detail: &AsinfoFullRecord) -> String {
        if let Some(ref pdb) = detail.peeringdb {
            if let Some(ref aka) = pdb.aka {
                if !aka.is_empty() {
                    return aka.clone();
                }
            }
            if let Some(ref name_long) = pdb.name_long {
                if !name_long.is_empty() {
                    return name_long.clone();
                }
            }
            if !pdb.name.is_empty() {
                return pdb.name.clone();
            }
        }

        if let Some(ref as2org) = detail.as2org {
            if !as2org.org_name.is_empty() {
                return as2org.org_name.clone();
            }
            if !as2org.name.is_empty() {
                return as2org.name.clone();
            }
        }

        detail.core.name.clone()
    }

    /// Truncate a name based on display config
    fn truncate_name(&self, name: &str, config: &InspectDisplayConfig) -> String {
        if !config.truncate_names || name.len() <= config.name_max_width {
            name.to_string()
        } else {
            format!("{}...", &name[..config.name_max_width.saturating_sub(3)])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_query_type_asn() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = InspectLens::new(&db);

        assert_eq!(lens.detect_query_type("13335"), InspectQueryType::Asn);
        assert_eq!(lens.detect_query_type("AS13335"), InspectQueryType::Asn);
        assert_eq!(lens.detect_query_type("as13335"), InspectQueryType::Asn);
        assert_eq!(lens.detect_query_type("  13335  "), InspectQueryType::Asn);
    }

    #[test]
    fn test_detect_query_type_prefix() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = InspectLens::new(&db);

        assert_eq!(
            lens.detect_query_type("1.1.1.0/24"),
            InspectQueryType::Prefix
        );
        assert_eq!(
            lens.detect_query_type("2606:4700::/32"),
            InspectQueryType::Prefix
        );
        assert_eq!(lens.detect_query_type("1.1.1.1"), InspectQueryType::Prefix);
        assert_eq!(lens.detect_query_type("::1"), InspectQueryType::Prefix);
    }

    #[test]
    fn test_detect_query_type_name() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = InspectLens::new(&db);

        assert_eq!(lens.detect_query_type("cloudflare"), InspectQueryType::Name);
        assert_eq!(lens.detect_query_type("Google LLC"), InspectQueryType::Name);
        assert_eq!(lens.detect_query_type("AS-SET"), InspectQueryType::Name);
    }

    #[test]
    fn test_parse_asn() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = InspectLens::new(&db);

        assert_eq!(lens.parse_asn("13335").unwrap(), 13335);
        assert_eq!(lens.parse_asn("AS13335").unwrap(), 13335);
        assert_eq!(lens.parse_asn("as13335").unwrap(), 13335);
        assert!(lens.parse_asn("invalid").is_err());
    }

    #[test]
    fn test_normalize_prefix() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = InspectLens::new(&db);

        assert_eq!(lens.normalize_prefix("1.1.1.0/24").unwrap(), "1.1.1.0/24");
        assert_eq!(lens.normalize_prefix("1.1.1.1").unwrap(), "1.1.1.1/32");
        assert_eq!(lens.normalize_prefix("::1").unwrap(), "::1/128");
    }

    #[test]
    fn test_query_options_should_include() {
        let options = InspectQueryOptions::default();

        // All sections should be included by default for ASN and Prefix queries
        assert!(options.should_include(InspectDataSection::Basic, InspectQueryType::Asn));
        assert!(options.should_include(InspectDataSection::Prefixes, InspectQueryType::Asn));
        assert!(options.should_include(InspectDataSection::Connectivity, InspectQueryType::Asn));
        assert!(options.should_include(InspectDataSection::Rpki, InspectQueryType::Asn));

        assert!(options.should_include(InspectDataSection::Basic, InspectQueryType::Prefix));
        assert!(options.should_include(InspectDataSection::Prefixes, InspectQueryType::Prefix));

        // Name queries only show basic by default
        assert!(options.should_include(InspectDataSection::Basic, InspectQueryType::Name));
        assert!(!options.should_include(InspectDataSection::Prefixes, InspectQueryType::Name));

        // With explicit selection, only selected sections are included
        let options =
            InspectQueryOptions::default().with_select(vec![InspectDataSection::Connectivity]);

        assert!(!options.should_include(InspectDataSection::Basic, InspectQueryType::Asn));
        assert!(options.should_include(InspectDataSection::Connectivity, InspectQueryType::Asn));
    }
}
