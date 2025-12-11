//! AS2Rel lens arguments
//!
//! This module defines the argument structures for AS2Rel operations.
//! These arguments are designed to be reusable across CLI, REST API,
//! WebSocket, and GUI interfaces.

use serde::{Deserialize, Serialize};

use super::types::{As2relOutputFormat, As2relSortOrder};

/// Arguments for AS2Rel search operations
///
/// This struct can be used in multiple contexts:
/// - CLI: with clap derives (when `cli` feature is enabled)
/// - REST API: as query parameters (via serde)
/// - WebSocket: as JSON message payload (via serde)
/// - GUI: as form state (via serde)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct As2relSearchArgs {
    /// One or two ASNs to query relationships for
    #[cfg_attr(feature = "cli", clap(required = true))]
    pub asns: Vec<u32>,

    /// Sort by ASN2 ascending instead of connected percentage descending
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub sort_by_asn: bool,

    /// Show organization name for ASN2 (from as2org database)
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub show_name: bool,

    /// Hide the explanation text
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub no_explain: bool,
}

impl As2relSearchArgs {
    /// Create new search arguments with a single ASN
    pub fn new(asn: u32) -> Self {
        Self {
            asns: vec![asn],
            ..Default::default()
        }
    }

    /// Create new search arguments with two ASNs (pair lookup)
    pub fn pair(asn1: u32, asn2: u32) -> Self {
        Self {
            asns: vec![asn1, asn2],
            ..Default::default()
        }
    }

    /// Set sort order to ASN ascending
    pub fn sort_by_asn(mut self) -> Self {
        self.sort_by_asn = true;
        self
    }

    /// Enable showing organization names
    pub fn with_names(mut self) -> Self {
        self.show_name = true;
        self
    }

    /// Hide explanation text
    pub fn no_explain(mut self) -> Self {
        self.no_explain = true;
        self
    }

    /// Get the sort order based on flags
    pub fn sort_order(&self) -> As2relSortOrder {
        if self.sort_by_asn {
            As2relSortOrder::Asn2Asc
        } else {
            As2relSortOrder::ConnectedDesc
        }
    }

    /// Validate the arguments
    ///
    /// Returns an error message if the arguments are invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.asns.is_empty() {
            return Err("At least one ASN is required".to_string());
        }
        if self.asns.len() > 2 {
            return Err("At most two ASNs can be specified".to_string());
        }
        Ok(())
    }

    /// Check if this is a pair lookup (two ASNs)
    pub fn is_pair_lookup(&self) -> bool {
        self.asns.len() == 2
    }
}

/// Arguments for AS2Rel update operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct As2relUpdateArgs {
    /// Force update even if data is fresh
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub force: bool,

    /// Update with a custom data file (local path or URL)
    #[cfg_attr(feature = "cli", clap(long))]
    pub update_with: Option<String>,
}

impl As2relUpdateArgs {
    /// Create update args for default URL
    pub fn new() -> Self {
        Self::default()
    }

    /// Create update args for custom path
    pub fn with_path(path: &str) -> Self {
        Self {
            update_with: Some(path.to_string()),
            force: true,
        }
    }

    /// Force update
    pub fn force(mut self) -> Self {
        self.force = true;
        self
    }
}

/// Arguments for AS2Rel output formatting
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct As2relOutputArgs {
    /// Output format
    #[cfg_attr(feature = "cli", clap(skip))]
    #[serde(default)]
    pub format: As2relOutputFormat,

    /// Output to pretty table (shortcut for format = Pretty)
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub pretty: bool,

    /// Output as JSON (shortcut for format = Json)
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub json: bool,
}

impl As2relOutputArgs {
    /// Determine the output format based on flags
    pub fn output_format(&self) -> As2relOutputFormat {
        if self.json {
            As2relOutputFormat::Json
        } else if self.pretty {
            As2relOutputFormat::Pretty
        } else {
            self.format.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_args_single() {
        let args = As2relSearchArgs::new(65000);
        assert_eq!(args.asns, vec![65000]);
        assert!(!args.is_pair_lookup());
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_search_args_pair() {
        let args = As2relSearchArgs::pair(65000, 65001);
        assert_eq!(args.asns, vec![65000, 65001]);
        assert!(args.is_pair_lookup());
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_search_args_builder() {
        let args = As2relSearchArgs::new(65000)
            .sort_by_asn()
            .with_names()
            .no_explain();

        assert!(args.sort_by_asn);
        assert!(args.show_name);
        assert!(args.no_explain);
        assert_eq!(args.sort_order(), As2relSortOrder::Asn2Asc);
    }

    #[test]
    fn test_validate_empty() {
        let args = As2relSearchArgs::default();
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_too_many() {
        let args = As2relSearchArgs {
            asns: vec![1, 2, 3],
            ..Default::default()
        };
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_update_args() {
        let args = As2relUpdateArgs::new();
        assert!(!args.force);
        assert!(args.update_with.is_none());

        let args = As2relUpdateArgs::with_path("/path/to/data.json");
        assert!(args.force);
        assert_eq!(args.update_with, Some("/path/to/data.json".to_string()));
    }

    #[test]
    fn test_output_format() {
        let args = As2relOutputArgs::default();
        assert_eq!(args.output_format(), As2relOutputFormat::Markdown);

        let args = As2relOutputArgs {
            json: true,
            ..Default::default()
        };
        assert_eq!(args.output_format(), As2relOutputFormat::Json);

        let args = As2relOutputArgs {
            pretty: true,
            ..Default::default()
        };
        assert_eq!(args.output_format(), As2relOutputFormat::Pretty);
    }
}
