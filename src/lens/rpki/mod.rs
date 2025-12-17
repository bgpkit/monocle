//! RPKI (Resource Public Key Infrastructure) lens module
//!
//! This module provides RPKI-related functionality including:
//! - ROA (Route Origin Authorization) lookup and validation
//! - ASPA (Autonomous System Provider Authorization) data access
//! - Historical RPKI data support via RIPE NCC and RPKIviews
//!
//! The lens uses `RpkiRepository` for cached/current data operations,
//! and bgpkit-commons for historical data loading (with date parameter).
//!
//! All functionality is accessed through the `RpkiLens` struct.

// Public modules (for advanced use cases like database refresh)
pub mod commons;

// Re-export types needed for external use (input/output structs)
pub use commons::{RpkiAspaEntry, RpkiAspaTableEntry, RpkiRoaEntry};

use crate::database::MonocleDatabase;
use crate::lens::utils::option_u32_from_str;
use anyhow::Result;
use bgpkit_commons::rpki::RpkiTrie;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// =============================================================================
// Types
// =============================================================================

/// Output format for RPKI lens results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum RpkiOutputFormat {
    /// Table format (default)
    #[default]
    Table,
    /// JSON format
    Json,
    /// Pretty-printed JSON
    Pretty,
}

/// Data source for RPKI data
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum RpkiDataSource {
    /// Current data from Cloudflare (default)
    #[default]
    Cloudflare,
    /// Historical data from RIPE NCC
    Ripe,
    /// Historical data from RPKIviews
    RpkiViews,
}

/// RPKIviews collector options
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum RpkiViewsCollectorOption {
    /// SoborostNet collector (default)
    #[default]
    Soborost,
    /// MassarsNet collector
    Massars,
    /// AttnJp collector
    Attn,
    /// KerfuffleNet collector
    Kerfuffle,
}

/// Validation state for RPKI route origin validation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RpkiValidationState {
    /// ROA exists with matching ASN and valid prefix length
    Valid,
    /// ROA exists but ASN doesn't match or prefix length exceeds max_length
    Invalid,
    /// No covering ROA exists for the prefix
    NotFound,
}

impl std::fmt::Display for RpkiValidationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpkiValidationState::Valid => write!(f, "valid"),
            RpkiValidationState::Invalid => write!(f, "invalid"),
            RpkiValidationState::NotFound => write!(f, "not_found"),
        }
    }
}

/// Detailed validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiValidationResult {
    /// The prefix being validated
    pub prefix: String,
    /// The ASN being validated
    pub asn: u32,
    /// Validation state
    pub state: RpkiValidationState,
    /// Human-readable reason for the validation state
    pub reason: String,
    /// Covering ROAs that were considered
    pub covering_roas: Vec<RpkiRoaRecord>,
}

/// ROA record (from database cache)
#[derive(Debug, Clone, Serialize, Deserialize, tabled::Tabled)]
pub struct RpkiRoaRecord {
    /// IP prefix
    pub prefix: String,
    /// Maximum prefix length
    pub max_length: u8,
    /// Origin ASN
    pub origin_asn: u32,
    /// Trust anchor (e.g., "ARIN", "RIPE", "APNIC")
    pub ta: String,
}

/// ASPA record (from database cache)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiAspaRecord {
    /// Customer ASN
    pub customer_asn: u32,
    /// List of authorized provider ASNs
    pub provider_asns: Vec<u32>,
}

// =============================================================================
// Args
// =============================================================================

/// Arguments for ROA lookup operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RpkiRoaLookupArgs {
    /// Filter by prefix
    #[cfg_attr(feature = "cli", clap(short, long))]
    pub prefix: Option<String>,

    /// Filter by origin ASN
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default, deserialize_with = "option_u32_from_str")]
    pub asn: Option<u32>,

    /// Load historical data for this date
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub date: Option<NaiveDate>,

    /// Data source for historical data
    #[cfg_attr(feature = "cli", clap(long, default_value = "cloudflare"))]
    #[serde(default)]
    pub source: RpkiDataSource,

    /// RPKIviews collector (only used with rpkiviews source)
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub collector: Option<RpkiViewsCollectorOption>,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiRoaLookupArgs {
    /// Create new ROA lookup args with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Set prefix filter
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Set ASN filter
    pub fn with_asn(mut self, asn: u32) -> Self {
        self.asn = Some(asn);
        self
    }

    /// Set historical date
    pub fn with_date(mut self, date: NaiveDate) -> Self {
        self.date = Some(date);
        self
    }

    /// Set data source
    pub fn with_source(mut self, source: RpkiDataSource) -> Self {
        self.source = source;
        self
    }

    /// Set output format
    pub fn with_format(mut self, format: RpkiOutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Check if this is a historical query (date is specified)
    pub fn is_historical(&self) -> bool {
        self.date.is_some()
    }
}

/// Arguments for ASPA lookup operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RpkiAspaLookupArgs {
    /// Filter by customer ASN
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default, deserialize_with = "option_u32_from_str")]
    pub customer_asn: Option<u32>,

    /// Filter by provider ASN
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default, deserialize_with = "option_u32_from_str")]
    pub provider_asn: Option<u32>,

    /// Load historical data for this date
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub date: Option<NaiveDate>,

    /// Data source for historical data
    #[cfg_attr(feature = "cli", clap(long, default_value = "cloudflare"))]
    #[serde(default)]
    pub source: RpkiDataSource,

    /// RPKIviews collector (only used with rpkiviews source)
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub collector: Option<RpkiViewsCollectorOption>,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiAspaLookupArgs {
    /// Create new ASPA lookup args with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Set customer ASN filter
    pub fn with_customer(mut self, asn: u32) -> Self {
        self.customer_asn = Some(asn);
        self
    }

    /// Set provider ASN filter
    pub fn with_provider(mut self, asn: u32) -> Self {
        self.provider_asn = Some(asn);
        self
    }

    /// Set output format
    pub fn with_format(mut self, format: RpkiOutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Check if this is a historical query (date is specified)
    pub fn is_historical(&self) -> bool {
        self.date.is_some()
    }
}

/// Arguments for validation operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RpkiValidateArgs {
    /// IP prefix to validate
    pub prefix: String,

    /// Origin ASN to validate
    pub asn: u32,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiValidateArgs {
    /// Create new validation args
    pub fn new(prefix: impl Into<String>, asn: u32) -> Self {
        Self {
            prefix: prefix.into(),
            asn,
            format: RpkiOutputFormat::default(),
        }
    }

    /// Set output format
    pub fn with_format(mut self, format: RpkiOutputFormat) -> Self {
        self.format = format;
        self
    }
}

// =============================================================================
// Lens
// =============================================================================

/// RPKI lens for ROA/ASPA lookup and validation
///
/// This lens provides two modes of operation:
///
/// 1. **Cached data operations** (default): Uses `RpkiRepository` for fast local
///    SQLite-based lookups and validation. Data must be loaded into the cache
///    first via `refresh()`.
///
/// 2. **Historical data operations**: When a date is specified in lookup args,
///    loads data directly from bgpkit-commons (RIPE NCC or RPKIviews).
///
/// # Example
///
/// ```rust,ignore
/// use monocle::database::MonocleDatabase;
/// use monocle::lens::rpki::{RpkiLens, RpkiRoaLookupArgs, RpkiValidateArgs};
///
/// let db = MonocleDatabase::open()?;
/// let lens = RpkiLens::new(&db);
///
/// // Ensure cache is populated
/// if lens.needs_refresh()? {
///     lens.refresh()?;
/// }
///
/// // Validate a prefix-ASN pair
/// let result = lens.validate("1.1.1.0/24", 13335)?;
///
/// // Get ROAs for an ASN (from cache)
/// let args = RpkiRoaLookupArgs::new().with_asn(13335);
/// let roas = lens.get_roas(&args)?;
/// ```
pub struct RpkiLens<'a> {
    /// Reference to the monocle database
    db: &'a MonocleDatabase,
    /// Cached RPKI trie for historical queries (lazy loaded)
    historical_trie: Option<RpkiTrie>,
}

impl<'a> RpkiLens<'a> {
    /// Create a new RPKI lens with database reference
    pub fn new(db: &'a MonocleDatabase) -> Self {
        Self {
            db,
            historical_trie: None,
        }
    }

    // =========================================================================
    // Cache management
    // =========================================================================

    /// Check if the cache is empty
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.db.rpki().is_empty())
    }

    /// Check if the cache needs refresh (empty or expired)
    pub fn needs_refresh(&self) -> Result<bool> {
        Ok(self
            .db
            .rpki()
            .needs_refresh(crate::database::DEFAULT_RPKI_CACHE_TTL))
    }

    /// Get cache metadata
    pub fn get_metadata(&self) -> Result<Option<crate::database::RpkiCacheMetadata>> {
        self.db.rpki().get_metadata()
    }

    /// Refresh the cache by loading current data from Cloudflare
    ///
    /// Returns the number of ROAs and ASPAs loaded.
    pub fn refresh(&self) -> Result<(usize, usize)> {
        let trie = commons::load_current_rpki()?;

        // Extract ROAs from the trie - iterate through all prefixes
        let mut roas: Vec<crate::database::RpkiRoaRecord> = Vec::new();
        for (prefix, prefix_roas) in trie.trie.iter() {
            for roa in prefix_roas {
                roas.push(crate::database::RpkiRoaRecord {
                    prefix: prefix.to_string(),
                    max_length: roa.max_length,
                    origin_asn: roa.asn,
                    ta: roa.rir.map(|r| format!("{:?}", r)).unwrap_or_default(),
                });
            }
        }

        // Extract ASPAs
        let aspas: Vec<_> = trie
            .aspas
            .iter()
            .map(|a| crate::database::RpkiAspaRecord {
                customer_asn: a.customer_asn,
                provider_asns: a.providers.clone(),
            })
            .collect();

        let roa_count = roas.len();
        let aspa_count = aspas.len();

        self.db.rpki().store(&roas, &aspas)?;

        Ok((roa_count, aspa_count))
    }

    // =========================================================================
    // Validation (policy logic - belongs in lens layer)
    // =========================================================================

    /// Validate a prefix-ASN pair against the cached ROAs
    ///
    /// This implements RFC 6811 Route Origin Validation:
    /// - **Valid**: A covering ROA exists with matching ASN and the announced
    ///   prefix length is <= max_length
    /// - **Invalid**: A covering ROA exists but either:
    ///   - The ASN doesn't match (unauthorized AS)
    ///   - The prefix length exceeds max_length (length violation)
    /// - **NotFound**: No covering ROA exists for the prefix
    pub fn validate(&self, prefix: &str, asn: u32) -> Result<RpkiValidationResult> {
        let covering_roas = self.get_covering_roas(prefix)?;

        if covering_roas.is_empty() {
            return Ok(RpkiValidationResult {
                prefix: prefix.to_string(),
                asn,
                state: RpkiValidationState::NotFound,
                reason: "No covering ROA found".to_string(),
                covering_roas: Vec::new(),
            });
        }

        // Parse the query prefix to get its length
        let query_prefix_len = parse_prefix_length(prefix)?;

        // Check if any ROA makes this valid
        for roa in &covering_roas {
            if roa.origin_asn == asn && query_prefix_len <= roa.max_length {
                return Ok(RpkiValidationResult {
                    prefix: prefix.to_string(),
                    asn,
                    state: RpkiValidationState::Valid,
                    reason: "ROA exists with matching ASN and valid prefix length".to_string(),
                    covering_roas,
                });
            }
        }

        // Determine the reason for invalidity
        let has_matching_asn = covering_roas.iter().any(|r| r.origin_asn == asn);
        let reason = if has_matching_asn {
            format!(
                "Prefix length {} exceeds max_length in covering ROAs",
                query_prefix_len
            )
        } else {
            let authorized_asns: Vec<String> = covering_roas
                .iter()
                .map(|r| r.origin_asn.to_string())
                .collect();
            format!(
                "ASN {} not authorized; authorized ASNs: {}",
                asn,
                authorized_asns.join(", ")
            )
        };

        Ok(RpkiValidationResult {
            prefix: prefix.to_string(),
            asn,
            state: RpkiValidationState::Invalid,
            reason,
            covering_roas,
        })
    }

    /// Get covering ROAs for a prefix (from cache)
    pub fn get_covering_roas(&self, prefix: &str) -> Result<Vec<RpkiRoaRecord>> {
        let db_roas = self.db.rpki().get_covering_roas(prefix)?;
        Ok(db_roas
            .into_iter()
            .map(|r| RpkiRoaRecord {
                prefix: r.prefix,
                max_length: r.max_length,
                origin_asn: r.origin_asn,
                ta: r.ta,
            })
            .collect())
    }

    // =========================================================================
    // ROA operations
    // =========================================================================

    /// Get ROAs based on lookup args
    ///
    /// For current data (no date specified), uses the local SQLite cache.
    /// For historical data (date specified), loads from bgpkit-commons.
    pub fn get_roas(&mut self, args: &RpkiRoaLookupArgs) -> Result<Vec<RpkiRoaEntry>> {
        if args.is_historical() {
            // Historical query: use bgpkit-commons
            let trie =
                self.load_historical_data(args.date, &args.source, args.collector.as_ref())?;
            commons::get_roas(trie, args.prefix.as_deref(), args.asn)
        } else {
            // Current query: use cache
            self.get_roas_from_cache(args.prefix.as_deref(), args.asn)
        }
    }

    /// Get ROAs from cache
    fn get_roas_from_cache(
        &self,
        prefix: Option<&str>,
        asn: Option<u32>,
    ) -> Result<Vec<RpkiRoaEntry>> {
        let repo = self.db.rpki();

        let roas = match (prefix, asn) {
            (Some(p), Some(a)) => {
                // Filter by both prefix and ASN
                let covering = repo.get_covering_roas(p)?;
                covering.into_iter().filter(|r| r.origin_asn == a).collect()
            }
            (Some(p), None) => {
                // Filter by prefix only
                repo.get_covering_roas(p)?
            }
            (None, Some(a)) => {
                // Filter by ASN only
                repo.get_roas_by_asn(a)?
            }
            (None, None) => {
                // Get all ROAs
                repo.get_all_roas()?
            }
        };

        Ok(roas
            .into_iter()
            .map(|r| RpkiRoaEntry {
                prefix: r.prefix,
                max_length: r.max_length,
                origin_asn: r.origin_asn,
                ta: r.ta,
            })
            .collect())
    }

    /// Get ROAs by ASN from cache
    pub fn get_roas_by_asn(&self, asn: u32) -> Result<Vec<RpkiRoaRecord>> {
        let db_roas = self.db.rpki().get_roas_by_asn(asn)?;
        Ok(db_roas
            .into_iter()
            .map(|r| RpkiRoaRecord {
                prefix: r.prefix,
                max_length: r.max_length,
                origin_asn: r.origin_asn,
                ta: r.ta,
            })
            .collect())
    }

    // =========================================================================
    // ASPA operations
    // =========================================================================

    /// Get ASPAs based on lookup args
    ///
    /// For current data (no date specified), uses the local SQLite cache.
    /// For historical data (date specified), loads from bgpkit-commons.
    pub fn get_aspas(&mut self, args: &RpkiAspaLookupArgs) -> Result<Vec<RpkiAspaEntry>> {
        if args.is_historical() {
            // Historical query: use bgpkit-commons
            let trie =
                self.load_historical_data(args.date, &args.source, args.collector.as_ref())?;
            commons::get_aspas(trie, args.customer_asn, args.provider_asn)
        } else {
            // Current query: use cache
            self.get_aspas_from_cache(args.customer_asn, args.provider_asn)
        }
    }

    /// Get ASPAs from cache
    fn get_aspas_from_cache(
        &self,
        customer_asn: Option<u32>,
        provider_asn: Option<u32>,
    ) -> Result<Vec<RpkiAspaEntry>> {
        let repo = self.db.rpki();

        let aspas = match (customer_asn, provider_asn) {
            (Some(c), Some(p)) => {
                // Filter by both customer and provider
                let by_customer = repo.get_aspas_by_customer(c)?;
                by_customer
                    .into_iter()
                    .filter(|a| a.provider_asns.contains(&p))
                    .collect()
            }
            (Some(c), None) => {
                // Filter by customer only
                repo.get_aspas_by_customer(c)?
            }
            (None, Some(p)) => {
                // Filter by provider only
                repo.get_aspas_by_provider(p)?
            }
            (None, None) => {
                // Get all ASPAs
                repo.get_all_aspas()?
            }
        };

        Ok(aspas
            .into_iter()
            .map(|a| RpkiAspaEntry {
                customer_asn: a.customer_asn,
                providers: a.provider_asns,
            })
            .collect())
    }

    /// Get ASPA by customer ASN from cache
    pub fn get_aspa_by_customer(&self, customer_asn: u32) -> Result<Option<RpkiAspaRecord>> {
        let aspas = self.db.rpki().get_aspas_by_customer(customer_asn)?;
        Ok(aspas.into_iter().next().map(|a| RpkiAspaRecord {
            customer_asn: a.customer_asn,
            provider_asns: a.provider_asns,
        }))
    }

    // =========================================================================
    // Historical data loading (internal)
    // =========================================================================

    /// Load historical RPKI data from bgpkit-commons
    fn load_historical_data(
        &mut self,
        date: Option<NaiveDate>,
        source: &RpkiDataSource,
        collector: Option<&RpkiViewsCollectorOption>,
    ) -> Result<&RpkiTrie> {
        let source_str = match source {
            RpkiDataSource::Cloudflare => None,
            RpkiDataSource::Ripe => Some("ripe"),
            RpkiDataSource::RpkiViews => Some("rpkiviews"),
        };

        let collector_str = collector.map(|c| match c {
            RpkiViewsCollectorOption::Soborost => "soborost",
            RpkiViewsCollectorOption::Massars => "massars",
            RpkiViewsCollectorOption::Attn => "attn",
            RpkiViewsCollectorOption::Kerfuffle => "kerfuffle",
        });

        let trie = commons::load_rpki_data(date, source_str, collector_str)?;
        self.historical_trie = Some(trie);

        #[allow(clippy::expect_used)]
        Ok(self.historical_trie.as_ref().expect("trie was just set"))
    }

    // =========================================================================
    // Formatting methods
    // =========================================================================

    /// Format ROA results for display
    pub fn format_roas(&self, roas: &[RpkiRoaEntry], format: &RpkiOutputFormat) -> String {
        match format {
            RpkiOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;
                Table::new(roas).with(Style::rounded()).to_string()
            }
            RpkiOutputFormat::Json => serde_json::to_string(roas).unwrap_or_default(),
            RpkiOutputFormat::Pretty => serde_json::to_string_pretty(roas).unwrap_or_default(),
        }
    }

    /// Format ASPA results for display
    pub fn format_aspas(&self, aspas: &[RpkiAspaEntry], format: &RpkiOutputFormat) -> String {
        match format {
            RpkiOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;
                let table_entries: Vec<RpkiAspaTableEntry> =
                    aspas.iter().map(|a| a.into()).collect();
                Table::new(table_entries).with(Style::rounded()).to_string()
            }
            RpkiOutputFormat::Json => serde_json::to_string(aspas).unwrap_or_default(),
            RpkiOutputFormat::Pretty => serde_json::to_string_pretty(aspas).unwrap_or_default(),
        }
    }

    /// Format validation result for display
    pub fn format_validation(
        &self,
        result: &RpkiValidationResult,
        format: &RpkiOutputFormat,
    ) -> String {
        match format {
            RpkiOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;

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

                output
            }
            RpkiOutputFormat::Json => serde_json::to_string(result).unwrap_or_default(),
            RpkiOutputFormat::Pretty => serde_json::to_string_pretty(result).unwrap_or_default(),
        }
    }
}

// =============================================================================
// Helper functions
// =============================================================================

/// Parse prefix length from a CIDR string
fn parse_prefix_length(prefix: &str) -> Result<u8> {
    let parts: Vec<&str> = prefix.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid prefix format: {}", prefix);
    }
    parts[1]
        .parse::<u8>()
        .map_err(|e| anyhow::anyhow!("Invalid prefix length: {}", e))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roa_lookup_args_builder() {
        let args = RpkiRoaLookupArgs::new()
            .with_prefix("1.1.1.0/24")
            .with_asn(13335)
            .with_format(RpkiOutputFormat::Json);

        assert_eq!(args.prefix, Some("1.1.1.0/24".to_string()));
        assert_eq!(args.asn, Some(13335));
        assert!(matches!(args.format, RpkiOutputFormat::Json));
        assert!(!args.is_historical());
    }

    #[test]
    fn test_roa_lookup_args_historical() {
        let date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let args = RpkiRoaLookupArgs::new()
            .with_date(date)
            .with_source(RpkiDataSource::Ripe);

        assert!(args.is_historical());
    }

    #[test]
    fn test_aspa_lookup_args_builder() {
        let args = RpkiAspaLookupArgs::new()
            .with_customer(13335)
            .with_provider(174);

        assert_eq!(args.customer_asn, Some(13335));
        assert_eq!(args.provider_asn, Some(174));
        assert!(!args.is_historical());
    }

    #[test]
    fn test_validate_args() {
        let args = RpkiValidateArgs::new("1.1.1.0/24", 13335).with_format(RpkiOutputFormat::Json);

        assert_eq!(args.prefix, "1.1.1.0/24");
        assert_eq!(args.asn, 13335);
        assert!(matches!(args.format, RpkiOutputFormat::Json));
    }

    #[test]
    fn test_validation_state_display() {
        assert_eq!(RpkiValidationState::Valid.to_string(), "valid");
        assert_eq!(RpkiValidationState::Invalid.to_string(), "invalid");
        assert_eq!(RpkiValidationState::NotFound.to_string(), "not_found");
    }

    #[test]
    fn test_parse_prefix_length() {
        assert_eq!(parse_prefix_length("1.1.1.0/24").unwrap(), 24);
        assert_eq!(parse_prefix_length("10.0.0.0/8").unwrap(), 8);
        assert_eq!(parse_prefix_length("2001:db8::/32").unwrap(), 32);
        assert!(parse_prefix_length("invalid").is_err());
    }
}
