//! RPKI (Resource Public Key Infrastructure) lens module
//!
//! This module provides RPKI-related functionality including:
//! - ROA (Route Origin Authorization) lookup from bgpkit-commons
//! - ASPA (Autonomous System Provider Authorization) data access
//! - Historical RPKI data support via RIPE NCC and RPKIviews
//!
//! For RPKI validation, use the `RpkiRepository` from the database module,
//! which provides local SQLite-based validation using cached data.
//!
//! All functionality is accessed through the `RpkiLens` struct.

// Internal modules
mod commons;

// Re-export types needed for external use (input/output structs)
pub use commons::{RpkiAspaEntry, RpkiAspaTableEntry, RpkiRoaEntry};

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
}

// =============================================================================
// Lens
// =============================================================================

/// RPKI lens for ROA/ASPA lookup
///
/// Provides methods for:
/// - Looking up ROAs by prefix or ASN (from bgpkit-commons data)
/// - Looking up ASPAs by customer or provider ASN
/// - Loading current data from Cloudflare or historical data from RIPE/RPKIviews
///
/// For RPKI validation (checking if a prefix-ASN pair is valid/invalid/not-found),
/// use the `RpkiRepository` from the database module instead, which provides
/// efficient local validation using SQLite-cached data.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::rpki::{RpkiLens, RpkiRoaLookupArgs, RpkiAspaLookupArgs};
///
/// let mut lens = RpkiLens::new();
///
/// // Get ROAs for an ASN
/// let args = RpkiRoaLookupArgs::new().with_asn(13335);
/// let roas = lens.get_roas(&args)?;
///
/// // Get ASPAs for a customer ASN
/// let args = RpkiAspaLookupArgs::new().with_customer(64496);
/// let aspas = lens.get_aspas(&args)?;
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
}
