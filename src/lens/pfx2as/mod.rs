//! Prefix-to-ASN mapping types
//!
//! This module provides types for prefix-to-ASN mapping operations.
//! The actual lookup functionality is provided by `Pfx2asRepository` in the database module.

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
    /// List of origin ASNs
    pub asns: String,
    /// Match type (exact or longest)
    pub match_type: String,
}

/// Output format for Pfx2as lens results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Pfx2asOutputFormat {
    /// JSON format (default)
    #[default]
    Json,
    /// Table format
    Table,
    /// Simple text format (ASNs only)
    Simple,
}

/// Lookup mode for prefix queries
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
}
