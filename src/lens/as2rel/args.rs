//! AS2Rel lens arguments
//!
//! This module defines the argument structures for AS2Rel operations.
//! These arguments are designed to be reusable across CLI, REST API,
//! WebSocket, and GUI interfaces.

use serde::{Deserialize, Serialize};

use super::types::{As2relOutputFormat, As2relSortOrder};
use crate::lens::utils::{bool_from_str, u32_or_vec};

/// Filter for relationship type perspective
///
/// When querying a single ASN, this filters results based on
/// the queried ASN's role in the relationship.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum RelationshipFilter {
    /// Show all relationship types (default)
    #[default]
    All,
    /// Show only relationships where ASN1 is upstream (provider) of ASN2
    IsUpstream,
    /// Show only relationships where ASN1 is downstream (customer) of ASN2
    IsDownstream,
    /// Show only peer relationships
    IsPeer,
}

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
    /// One or more ASNs to query relationships for
    #[cfg_attr(feature = "cli", clap(required = true))]
    #[serde(default, deserialize_with = "u32_or_vec")]
    pub asns: Vec<u32>,

    /// Sort by ASN2 ascending instead of connected percentage descending
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default, deserialize_with = "bool_from_str")]
    pub sort_by_asn: bool,

    /// Show organization name for ASN2 (from as2org database)
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default, deserialize_with = "bool_from_str")]
    pub show_name: bool,

    /// Hide the explanation text
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default, deserialize_with = "bool_from_str")]
    pub no_explain: bool,

    /// Minimum visibility percentage (0-100) to include in results
    ///
    /// Filters out relationships seen by fewer than this percentage of peers.
    /// For example, `--min-visibility 10` excludes relationships seen by <10% of peers.
    #[cfg_attr(feature = "cli", clap(long, value_name = "PERCENT"))]
    #[serde(default)]
    pub min_visibility: Option<f32>,

    /// Only show ASNs that are single-homed to the queried ASN
    ///
    /// An ASN is single-homed if it has exactly one upstream provider.
    /// This flag filters results to only include ASNs where the queried ASN
    /// is their only upstream.
    ///
    /// Only applicable when querying a single ASN.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default, deserialize_with = "bool_from_str")]
    pub single_homed: bool,

    /// Only show relationships where the queried ASN is an upstream (provider)
    ///
    /// Only applicable when querying a single ASN.
    #[cfg_attr(feature = "cli", clap(long, conflicts_with_all = ["is_downstream", "is_peer"]))]
    #[serde(default, deserialize_with = "bool_from_str")]
    pub is_upstream: bool,

    /// Only show relationships where the queried ASN is a downstream (customer)
    ///
    /// Only applicable when querying a single ASN.
    #[cfg_attr(feature = "cli", clap(long, conflicts_with_all = ["is_upstream", "is_peer"]))]
    #[serde(default, deserialize_with = "bool_from_str")]
    pub is_downstream: bool,

    /// Only show peer relationships
    ///
    /// Only applicable when querying a single ASN.
    #[cfg_attr(feature = "cli", clap(long, conflicts_with_all = ["is_upstream", "is_downstream"]))]
    #[serde(default, deserialize_with = "bool_from_str")]
    pub is_peer: bool,
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

    /// Create new search arguments with multiple ASNs
    pub fn multiple(asns: Vec<u32>) -> Self {
        Self {
            asns,
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

    /// Set minimum visibility threshold
    pub fn with_min_visibility(mut self, percent: f32) -> Self {
        self.min_visibility = Some(percent);
        self
    }

    /// Filter to single-homed ASNs only
    pub fn single_homed_only(mut self) -> Self {
        self.single_homed = true;
        self
    }

    /// Filter to show only downstream relationships (queried ASN is upstream)
    pub fn upstream_only(mut self) -> Self {
        self.is_upstream = true;
        self.is_downstream = false;
        self.is_peer = false;
        self
    }

    /// Filter to show only upstream relationships (queried ASN is downstream)
    pub fn downstream_only(mut self) -> Self {
        self.is_downstream = true;
        self.is_upstream = false;
        self.is_peer = false;
        self
    }

    /// Filter to show only peer relationships
    pub fn peer_only(mut self) -> Self {
        self.is_peer = true;
        self.is_upstream = false;
        self.is_downstream = false;
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

    /// Get the relationship filter based on flags
    pub fn relationship_filter(&self) -> RelationshipFilter {
        if self.is_upstream {
            RelationshipFilter::IsUpstream
        } else if self.is_downstream {
            RelationshipFilter::IsDownstream
        } else if self.is_peer {
            RelationshipFilter::IsPeer
        } else {
            RelationshipFilter::All
        }
    }

    /// Check if any relationship filter is set
    pub fn has_relationship_filter(&self) -> bool {
        self.is_upstream || self.is_downstream || self.is_peer
    }

    /// Validate the arguments
    ///
    /// Returns an error message if the arguments are invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.asns.is_empty() {
            return Err("At least one ASN is required".to_string());
        }

        // Single-homed filter only makes sense for single ASN queries
        if self.single_homed && self.asns.len() != 1 {
            return Err("--single-homed can only be used with a single ASN".to_string());
        }

        // Relationship filters only make sense for single ASN queries
        if self.has_relationship_filter() && self.asns.len() != 1 {
            return Err(
                "--is-upstream, --is-downstream, and --is-peer can only be used with a single ASN"
                    .to_string(),
            );
        }

        // Validate min_visibility range
        if let Some(min_vis) = self.min_visibility {
            if !(0.0..=100.0).contains(&min_vis) {
                return Err("--min-visibility must be between 0 and 100".to_string());
            }
        }

        Ok(())
    }

    /// Check if this is a pair lookup (exactly two ASNs)
    pub fn is_pair_lookup(&self) -> bool {
        self.asns.len() == 2
    }

    /// Check if this is a single ASN lookup
    pub fn is_single_lookup(&self) -> bool {
        self.asns.len() == 1
    }

    /// Check if this is a multi-ASN lookup (more than 2)
    pub fn is_multi_lookup(&self) -> bool {
        self.asns.len() > 2
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
        assert!(args.is_single_lookup());
        assert!(!args.is_pair_lookup());
        assert!(!args.is_multi_lookup());
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_search_args_pair() {
        let args = As2relSearchArgs::pair(65000, 65001);
        assert_eq!(args.asns, vec![65000, 65001]);
        assert!(args.is_pair_lookup());
        assert!(!args.is_single_lookup());
        assert!(!args.is_multi_lookup());
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_search_args_multiple() {
        let args = As2relSearchArgs::multiple(vec![65000, 65001, 65002]);
        assert_eq!(args.asns, vec![65000, 65001, 65002]);
        assert!(args.is_multi_lookup());
        assert!(!args.is_single_lookup());
        assert!(!args.is_pair_lookup());
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_search_args_builder() {
        let args = As2relSearchArgs::new(65000)
            .sort_by_asn()
            .with_names()
            .no_explain()
            .with_min_visibility(10.0);

        assert!(args.sort_by_asn);
        assert!(args.show_name);
        assert!(args.no_explain);
        assert_eq!(args.min_visibility, Some(10.0));
        assert_eq!(args.sort_order(), As2relSortOrder::Asn2Asc);
    }

    #[test]
    fn test_relationship_filters() {
        let args = As2relSearchArgs::new(65000).upstream_only();
        assert!(args.is_upstream);
        assert!(!args.is_downstream);
        assert!(!args.is_peer);
        assert_eq!(args.relationship_filter(), RelationshipFilter::IsUpstream);
        assert!(args.validate().is_ok());

        let args = As2relSearchArgs::new(65000).downstream_only();
        assert_eq!(args.relationship_filter(), RelationshipFilter::IsDownstream);
        assert!(args.validate().is_ok());

        let args = As2relSearchArgs::new(65000).peer_only();
        assert_eq!(args.relationship_filter(), RelationshipFilter::IsPeer);
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_single_homed_filter() {
        let args = As2relSearchArgs::new(65000).single_homed_only();
        assert!(args.single_homed);
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_validate_empty() {
        let args = As2relSearchArgs::default();
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_single_homed_with_multiple_asns() {
        let args = As2relSearchArgs {
            asns: vec![1, 2],
            single_homed: true,
            ..Default::default()
        };
        assert!(args.validate().is_err());
        assert!(args.validate().unwrap_err().contains("single ASN"));
    }

    #[test]
    fn test_validate_relationship_filter_with_multiple_asns() {
        let args = As2relSearchArgs {
            asns: vec![1, 2],
            is_upstream: true,
            ..Default::default()
        };
        assert!(args.validate().is_err());
        assert!(args.validate().unwrap_err().contains("single ASN"));
    }

    #[test]
    fn test_validate_min_visibility_range() {
        let args = As2relSearchArgs {
            asns: vec![65000],
            min_visibility: Some(-1.0),
            ..Default::default()
        };
        assert!(args.validate().is_err());

        let args = As2relSearchArgs {
            asns: vec![65000],
            min_visibility: Some(101.0),
            ..Default::default()
        };
        assert!(args.validate().is_err());

        let args = As2relSearchArgs {
            asns: vec![65000],
            min_visibility: Some(50.0),
            ..Default::default()
        };
        assert!(args.validate().is_ok());
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
