//! RPKI (Resource Public Key Infrastructure) lens module
//!
//! This module provides RPKI-related functionality including:
//! - ROA (Route Origin Authorization) lookup and validation
//! - ASPA (Autonomous System Provider Authorization) data access
//! - RPKI validation via Cloudflare's GraphQL API
//! - Historical RPKI data support via RIPE NCC and RPKIviews
//!
//! All functionality is accessed through the `RpkiLens` struct.

// Internal modules - all access should go through RpkiLens
mod commons;
mod validator;

// Re-export only types needed for external use (input/output structs)
// These are used as return types from RpkiLens methods
pub use commons::{RpkiAspaEntry, RpkiAspaTableEntry, RpkiRoaEntry};
pub use validator::{
    RpkiRoa, RpkiRoaPrefix, RpkiRoaResource, RpkiRoaTableItem, RpkiSummaryTableItem,
    RpkiValidationState, RpkiValidity,
};

use crate::lens::utils::{option_u32_from_str, u32_from_str};
use anyhow::Result;
use bgpkit_commons::rpki::RpkiTrie;
use chrono::NaiveDate;
use ipnet::IpNet;
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

    /// Historical date (format: YYYY-MM-DD)
    #[cfg_attr(feature = "cli", clap(short, long))]
    pub date: Option<NaiveDate>,

    /// Data source for historical data
    #[cfg_attr(feature = "cli", clap(long, default_value = "cloudflare"))]
    #[serde(default)]
    pub source: RpkiDataSource,

    /// RPKIviews collector (only used with rpkiviews source)
    #[cfg_attr(feature = "cli", clap(long))]
    pub collector: Option<RpkiViewsCollectorOption>,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiRoaLookupArgs {
    /// Create new args for ROA lookup
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by prefix
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Filter by ASN
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
}

/// Arguments for ASPA lookup operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RpkiAspaLookupArgs {
    /// Filter by customer ASN
    #[cfg_attr(feature = "cli", clap(short = 'c', long))]
    #[serde(default, deserialize_with = "option_u32_from_str")]
    pub customer_asn: Option<u32>,

    /// Filter by provider ASN
    #[cfg_attr(feature = "cli", clap(short = 'p', long))]
    #[serde(default, deserialize_with = "option_u32_from_str")]
    pub provider_asn: Option<u32>,

    /// Historical date (format: YYYY-MM-DD)
    #[cfg_attr(feature = "cli", clap(short, long))]
    pub date: Option<NaiveDate>,

    /// Data source for historical data
    #[cfg_attr(feature = "cli", clap(long, default_value = "cloudflare"))]
    #[serde(default)]
    pub source: RpkiDataSource,

    /// RPKIviews collector (only used with rpkiviews source)
    #[cfg_attr(feature = "cli", clap(long))]
    pub collector: Option<RpkiViewsCollectorOption>,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiAspaLookupArgs {
    /// Create new args for ASPA lookup
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by customer ASN
    pub fn with_customer(mut self, asn: u32) -> Self {
        self.customer_asn = Some(asn);
        self
    }

    /// Filter by provider ASN
    pub fn with_provider(mut self, asn: u32) -> Self {
        self.provider_asn = Some(asn);
        self
    }

    /// Set output format
    pub fn with_format(mut self, format: RpkiOutputFormat) -> Self {
        self.format = format;
        self
    }
}

/// Arguments for RPKI validation operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RpkiValidationArgs {
    /// Origin ASN to validate
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(deserialize_with = "u32_from_str")]
    pub asn: u32,

    /// Prefix to validate
    #[cfg_attr(feature = "cli", clap(short, long))]
    pub prefix: String,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiValidationArgs {
    /// Create new validation args
    pub fn new(asn: u32, prefix: impl Into<String>) -> Self {
        Self {
            asn,
            prefix: prefix.into(),
            format: RpkiOutputFormat::default(),
        }
    }

    /// Set output format
    pub fn with_format(mut self, format: RpkiOutputFormat) -> Self {
        self.format = format;
        self
    }
}

/// Arguments for ASN summary operations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RpkiSummaryArgs {
    /// ASN to summarize
    #[cfg_attr(feature = "cli", clap(value_name = "ASN"))]
    #[serde(deserialize_with = "u32_from_str")]
    pub asn: u32,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiSummaryArgs {
    /// Create new summary args
    pub fn new(asn: u32) -> Self {
        Self {
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

/// Arguments for listing ROAs by resource (ASN or prefix)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct RpkiListArgs {
    /// Resource to list ROAs for (ASN number or IP prefix)
    #[cfg_attr(feature = "cli", clap(value_name = "RESOURCE"))]
    pub resource: String,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: RpkiOutputFormat,
}

impl RpkiListArgs {
    /// Create new list args for an ASN
    pub fn for_asn(asn: u32) -> Self {
        Self {
            resource: asn.to_string(),
            format: RpkiOutputFormat::default(),
        }
    }

    /// Create new list args for a prefix
    pub fn for_prefix(prefix: impl Into<String>) -> Self {
        Self {
            resource: prefix.into(),
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
/// Provides methods for:
/// - Looking up ROAs by prefix or ASN (from bgpkit-commons data)
/// - Looking up ASPAs by customer or provider ASN
/// - Validating prefix/ASN pairs against current RPKI data (Cloudflare API)
/// - Listing ROAs for a resource (Cloudflare API)
/// - Summarizing RPKI coverage for an ASN (Cloudflare API)
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs, RpkiListArgs};
///
/// let lens = RpkiLens::new();
///
/// // Validate a prefix/ASN pair
/// let args = RpkiValidationArgs::new(13335, "1.1.1.0/24");
/// let result = lens.validate(&args)?;
///
/// // List ROAs for an ASN
/// let args = RpkiListArgs::for_asn(13335);
/// let roas = lens.list_roas(&args)?;
/// ```
pub struct RpkiLens {
    /// Cached RPKI trie (lazy loaded)
    trie: Option<RpkiTrie>,
}

impl RpkiLens {
    /// Create a new RPKI lens
    pub fn new() -> Self {
        Self { trie: None }
    }

    /// Create a new RPKI lens with pre-loaded data
    pub fn with_trie(trie: RpkiTrie) -> Self {
        Self { trie: Some(trie) }
    }

    // =========================================================================
    // Internal helper methods
    // =========================================================================

    /// Load RPKI data based on source and date (internal)
    fn load_data_internal(
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
        self.trie = Some(trie);

        #[allow(clippy::expect_used)]
        Ok(self.trie.as_ref().expect("trie was just set"))
    }

    // =========================================================================
    // Public API - ROA operations (bgpkit-commons data)
    // =========================================================================

    /// Get ROAs based on lookup args (uses bgpkit-commons data)
    ///
    /// This loads ROA data from bgpkit-commons (current from Cloudflare,
    /// or historical from RIPE/RPKIviews) and filters by prefix and/or ASN.
    pub fn get_roas(&mut self, args: &RpkiRoaLookupArgs) -> Result<Vec<RpkiRoaEntry>> {
        let trie = self.load_data_internal(args.date, &args.source, args.collector.as_ref())?;
        commons::get_roas(trie, args.prefix.as_deref(), args.asn)
    }

    /// Get ASPAs based on lookup args (uses bgpkit-commons data)
    ///
    /// This loads ASPA data from bgpkit-commons and filters by customer
    /// and/or provider ASN.
    pub fn get_aspas(&mut self, args: &RpkiAspaLookupArgs) -> Result<Vec<RpkiAspaEntry>> {
        let trie = self.load_data_internal(args.date, &args.source, args.collector.as_ref())?;
        commons::get_aspas(trie, args.customer_asn, args.provider_asn)
    }

    // =========================================================================
    // Public API - Cloudflare GraphQL API operations
    // =========================================================================

    /// Validate a prefix/ASN pair using Cloudflare's RPKI API
    ///
    /// Returns the validation result and any covering ROAs.
    pub fn validate(&self, args: &RpkiValidationArgs) -> Result<(RpkiValidity, Vec<RpkiRoa>)> {
        validator::validate(args.asn, &args.prefix)
    }

    /// List ROAs for a resource (ASN or prefix) using Cloudflare's RPKI API
    ///
    /// The resource can be either:
    /// - An ASN (e.g., "13335")
    /// - An IP prefix (e.g., "1.1.1.0/24")
    pub fn list_roas(&self, args: &RpkiListArgs) -> Result<Vec<RpkiRoaTableItem>> {
        // Try to parse as ASN first, then as prefix
        let resources = if let Ok(asn) = args.resource.parse::<u32>() {
            validator::list_by_asn(asn)?
        } else if let Ok(prefix) = args.resource.parse::<IpNet>() {
            validator::list_by_prefix(&prefix)?
        } else {
            return Err(anyhow::anyhow!(
                "Resource '{}' is neither a valid ASN nor a valid prefix",
                args.resource
            ));
        };

        // Convert to table items
        let roas: Vec<RpkiRoaTableItem> = resources
            .into_iter()
            .flat_map(Into::<Vec<RpkiRoaTableItem>>::into)
            .collect();

        Ok(roas)
    }

    /// Get RPKI summary for an ASN using Cloudflare's RPKI API
    ///
    /// Returns statistics about signed prefixes and routing validity.
    pub fn summarize(&self, args: &RpkiSummaryArgs) -> Result<RpkiSummaryTableItem> {
        validator::summarize_asn(args.asn)
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

    /// Format ROA table items for display
    pub fn format_roa_items(&self, roas: &[RpkiRoaTableItem], format: &RpkiOutputFormat) -> String {
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

    /// Format validation results for display
    pub fn format_validation(
        &self,
        validity: &RpkiValidity,
        covering: &[RpkiRoa],
        format: &RpkiOutputFormat,
    ) -> String {
        match format {
            RpkiOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;

                let mut output = Table::new(vec![validity])
                    .with(Style::rounded())
                    .to_string();

                if !covering.is_empty() {
                    let covering_items: Vec<RpkiRoaTableItem> =
                        covering.iter().cloned().map(|r| r.into()).collect();
                    output.push_str("\n\nCovering ROAs:\n");
                    output.push_str(
                        &Table::new(covering_items)
                            .with(Style::rounded())
                            .to_string(),
                    );
                }

                output
            }
            RpkiOutputFormat::Json | RpkiOutputFormat::Pretty => {
                let result = serde_json::json!({
                    "validity": validity,
                    "covering_roas": covering,
                });
                if matches!(format, RpkiOutputFormat::Pretty) {
                    serde_json::to_string_pretty(&result).unwrap_or_default()
                } else {
                    serde_json::to_string(&result).unwrap_or_default()
                }
            }
        }
    }

    /// Format summary results for display
    pub fn format_summary(
        &self,
        summary: &RpkiSummaryTableItem,
        format: &RpkiOutputFormat,
    ) -> String {
        match format {
            RpkiOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;
                Table::new(vec![summary]).with(Style::rounded()).to_string()
            }
            RpkiOutputFormat::Json => serde_json::to_string(summary).unwrap_or_default(),
            RpkiOutputFormat::Pretty => serde_json::to_string_pretty(summary).unwrap_or_default(),
        }
    }
}

impl Default for RpkiLens {
    fn default() -> Self {
        Self::new()
    }
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
    }

    #[test]
    fn test_aspa_lookup_args_builder() {
        let args = RpkiAspaLookupArgs::new()
            .with_customer(13335)
            .with_provider(174);

        assert_eq!(args.customer_asn, Some(13335));
        assert_eq!(args.provider_asn, Some(174));
    }

    #[test]
    fn test_validation_args() {
        let args = RpkiValidationArgs::new(13335, "1.1.1.0/24");

        assert_eq!(args.asn, 13335);
        assert_eq!(args.prefix, "1.1.1.0/24");
    }

    #[test]
    fn test_summary_args() {
        let args = RpkiSummaryArgs::new(13335).with_format(RpkiOutputFormat::Pretty);

        assert_eq!(args.asn, 13335);
        assert!(matches!(args.format, RpkiOutputFormat::Pretty));
    }

    #[test]
    fn test_list_args_for_asn() {
        let args = RpkiListArgs::for_asn(13335);
        assert_eq!(args.resource, "13335");
    }

    #[test]
    fn test_list_args_for_prefix() {
        let args = RpkiListArgs::for_prefix("1.1.1.0/24");
        assert_eq!(args.resource, "1.1.1.0/24");
    }
}
