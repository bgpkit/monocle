//! Prefix-to-ASN mapping lens module
//!
//! This module provides the `Pfx2asLens` for prefix-to-ASN mapping operations.
//! The lens wraps `Pfx2asRepository` and provides:
//! - Prefix lookup (exact, longest, covering, covered)
//! - ASN-to-prefixes lookup
//! - Cache management (refresh, needs_refresh)
//! - Output formatting
//!
//! # Example
//!
//! ```rust,ignore
//! use monocle::database::MonocleDatabase;
//! use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asLookupArgs, Pfx2asLookupMode};
//!
//! let db = MonocleDatabase::open()?;
//! let lens = Pfx2asLens::new(&db);
//!
//! // Ensure cache is populated
//! if lens.needs_refresh()? {
//!     lens.refresh(None)?;
//! }
//!
//! // Lookup a prefix
//! let args = Pfx2asLookupArgs::new("1.1.1.0/24").longest();
//! let result = lens.lookup(&args)?;
//!
//! // Get all prefixes for an ASN
//! let prefixes = lens.get_prefixes_for_asn(13335)?;
//! ```

use crate::database::MonocleDatabase;
use anyhow::Result;
use serde::{Deserialize, Serialize};
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

// =============================================================================
// Lens
// =============================================================================

/// Pfx2as lens for prefix-to-ASN mapping operations
///
/// This lens wraps `Pfx2asRepository` and provides:
/// - Prefix lookup with various modes (exact, longest, covering, covered)
/// - ASN-to-prefixes lookup
/// - Cache management
/// - Output formatting
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
    pub fn needs_refresh(&self) -> Result<bool> {
        Ok(self
            .db
            .pfx2as()
            .needs_refresh(crate::database::DEFAULT_PFX2AS_CACHE_TTL))
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
    // Lookup operations
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
}
