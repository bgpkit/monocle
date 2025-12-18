//! AS2Rel lens
//!
//! This module provides the AS2Rel lens for querying AS-level relationships.
//! It uses SQLite as the backend database.

pub mod args;
pub mod types;

pub use args::{As2relOutputArgs, As2relSearchArgs, As2relUpdateArgs, RelationshipFilter};
pub use types::{
    As2relDataMeta, As2relOutputFormat, As2relSearchResult, As2relSearchResultWithName,
    As2relSortOrder, As2relUpdateProgress, As2relUpdateStage,
};

// Re-export common utilities for convenience
pub use crate::lens::utils::{truncate_name, DEFAULT_NAME_MAX_LEN};

use crate::database::{MonocleDatabase, BGPKIT_AS2REL_URL};
use anyhow::Result;
use serde_json::json;

/// AS2Rel lens for querying AS-level relationships
///
/// This lens provides high-level operations for:
/// - Searching for AS relationships by ASN or ASN pair
/// - Filtering by relationship type, visibility, and single-homed status
/// - Updating AS2Rel data
/// - Formatting results for output
pub struct As2relLens<'a> {
    db: &'a MonocleDatabase,
}

impl<'a> As2relLens<'a> {
    /// Create a new AS2Rel lens
    pub fn new(db: &'a MonocleDatabase) -> Self {
        Self { db }
    }

    /// Check if data is available
    pub fn is_data_available(&self) -> bool {
        !self.db.as2rel().is_empty()
    }

    /// Check if data needs to be updated
    pub fn needs_update(&self) -> bool {
        self.db.needs_as2rel_update()
    }

    /// Update AS2Rel data from the default URL
    pub fn update(&self) -> Result<usize> {
        self.db.update_as2rel()
    }

    /// Update AS2Rel data from a custom path
    pub fn update_from(&self, path: &str) -> Result<usize> {
        self.db.update_as2rel_from(path)
    }

    /// Get the maximum peers count (for percentage calculation)
    pub fn get_max_peers_count(&self) -> u32 {
        self.db.as2rel().get_max_peers_count()
    }

    /// Search using the provided arguments
    ///
    /// Supports:
    /// - Single ASN queries (optionally filtered by relationship type, single-homed)
    /// - Pair queries (two ASNs)
    /// - Multi-ASN queries (all pairs among provided ASNs)
    /// - Minimum visibility filtering
    pub fn search(&self, args: &As2relSearchArgs) -> Result<Vec<As2relSearchResult>> {
        let max_peers = self.get_max_peers_count();

        let results = if args.asns.len() == 1 {
            self.search_single_asn(args, max_peers)?
        } else if args.asns.len() == 2 {
            self.search_pair(args, max_peers)?
        } else {
            self.search_multi_asn(args, max_peers)?
        };

        Ok(results)
    }

    /// Search for a single ASN with optional filters
    fn search_single_asn(
        &self,
        args: &As2relSearchArgs,
        max_peers: u32,
    ) -> Result<Vec<As2relSearchResult>> {
        let asn = args.asns[0];

        // Handle single-homed filter specially
        if args.single_homed {
            return self.search_single_homed(asn, args, max_peers);
        }

        // Get relationships based on filter
        let aggregated = match args.relationship_filter() {
            RelationshipFilter::All => self.db.as2rel().search_asn_with_names(asn)?,
            RelationshipFilter::IsUpstream => {
                // ASN is upstream (provider) - show its downstreams/customers
                self.db.as2rel().search_asn_with_names_by_rel_type(asn, 1)?
            }
            RelationshipFilter::IsDownstream => {
                // ASN is downstream (customer) - show its upstreams/providers
                self.db
                    .as2rel()
                    .search_asn_with_names_by_rel_type(asn, -1)?
            }
            RelationshipFilter::IsPeer => {
                // Show only peer relationships
                self.db.as2rel().search_asn_with_names_by_rel_type(asn, 0)?
            }
        };

        // Convert to search results with percentages
        let mut results: Vec<As2relSearchResult> = aggregated
            .into_iter()
            .map(|a| {
                As2relSearchResult::from_aggregated(
                    a.asn1,
                    a.asn2,
                    a.asn2_name,
                    a.connected_count,
                    a.as1_upstream_count,
                    a.as2_upstream_count,
                    max_peers,
                )
            })
            .collect();

        // Apply minimum visibility filter
        if let Some(min_vis) = args.min_visibility {
            results.retain(|r| r.connected_pct >= min_vis);
        }

        // Sort results
        self.sort_results(&mut results, &args.sort_order());

        Ok(results)
    }

    /// Search for ASNs that are single-homed to the given upstream
    fn search_single_homed(
        &self,
        upstream_asn: u32,
        args: &As2relSearchArgs,
        max_peers: u32,
    ) -> Result<Vec<As2relSearchResult>> {
        let single_homed = self
            .db
            .as2rel()
            .find_single_homed_to(upstream_asn, args.min_visibility)?;

        let mut results: Vec<As2relSearchResult> = single_homed
            .into_iter()
            .map(|(customer_asn, peers_count, name)| {
                let connected_pct = if max_peers > 0 {
                    (peers_count as f32 / max_peers as f32) * 100.0
                } else {
                    0.0
                };

                As2relSearchResult {
                    asn1: upstream_asn,
                    asn2: customer_asn,
                    asn2_name: name,
                    connected: format!("{:.1}%", connected_pct),
                    connected_pct,
                    // For single-homed, upstream_asn is always the upstream
                    peer: "0.0%".to_string(),
                    as1_upstream: format!("{:.1}%", connected_pct),
                    as2_upstream: "0.0%".to_string(),
                }
            })
            .collect();

        // Sort results
        self.sort_results(&mut results, &args.sort_order());

        Ok(results)
    }

    /// Search for a pair of ASNs
    fn search_pair(
        &self,
        args: &As2relSearchArgs,
        max_peers: u32,
    ) -> Result<Vec<As2relSearchResult>> {
        let asn1 = args.asns[0];
        let asn2 = args.asns[1];

        let aggregated = self.db.as2rel().search_pair_with_names(asn1, asn2)?;

        let mut results: Vec<As2relSearchResult> = aggregated
            .into_iter()
            .map(|a| {
                As2relSearchResult::from_aggregated(
                    a.asn1,
                    a.asn2,
                    a.asn2_name,
                    a.connected_count,
                    a.as1_upstream_count,
                    a.as2_upstream_count,
                    max_peers,
                )
            })
            .collect();

        // Apply minimum visibility filter
        if let Some(min_vis) = args.min_visibility {
            results.retain(|r| r.connected_pct >= min_vis);
        }

        // Sort results
        self.sort_results(&mut results, &args.sort_order());

        Ok(results)
    }

    /// Search for all pairs among multiple ASNs
    fn search_multi_asn(
        &self,
        args: &As2relSearchArgs,
        max_peers: u32,
    ) -> Result<Vec<As2relSearchResult>> {
        let aggregated = self
            .db
            .as2rel()
            .search_multi_asn_pairs_with_names(&args.asns)?;

        let mut results: Vec<As2relSearchResult> = aggregated
            .into_iter()
            .map(|a| {
                As2relSearchResult::from_aggregated(
                    a.asn1,
                    a.asn2,
                    a.asn2_name,
                    a.connected_count,
                    a.as1_upstream_count,
                    a.as2_upstream_count,
                    max_peers,
                )
            })
            .collect();

        // Apply minimum visibility filter
        if let Some(min_vis) = args.min_visibility {
            results.retain(|r| r.connected_pct >= min_vis);
        }

        // For multi-ASN queries, always sort by asn1 (already sorted from query)
        // but allow override with sort_by_asn flag
        if !args.sort_by_asn {
            // Default: sort by asn1, then connected desc
            results.sort_by(|a, b| {
                a.asn1.cmp(&b.asn1).then_with(|| {
                    b.connected_pct
                        .partial_cmp(&a.connected_pct)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
            });
        } else {
            // Sort by asn1, then asn2
            results.sort_by(|a, b| a.asn1.cmp(&b.asn1).then_with(|| a.asn2.cmp(&b.asn2)));
        }

        Ok(results)
    }

    /// Sort results by the specified order
    pub fn sort_results(&self, results: &mut [As2relSearchResult], order: &As2relSortOrder) {
        match order {
            As2relSortOrder::ConnectedDesc => {
                results.sort_by(|a, b| {
                    b.connected_pct
                        .partial_cmp(&a.connected_pct)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            As2relSortOrder::Asn2Asc => {
                results.sort_by_key(|r| r.asn2);
            }
        }
    }

    /// Get explanation text for the data
    pub fn get_explanation(&self) -> String {
        let max_peers = self.get_max_peers_count();
        format!(
            "Explanation:\n\
             - connected: % of {} peers that see this AS relationship\n\
             - peer: % where the relationship is peer-to-peer\n\
             - as1_upstream: % where ASN1 is the upstream (provider)\n\
             - as2_upstream: % where ASN2 is the upstream (provider)\n\
             \n\
             Data source: {}\n",
            max_peers, BGPKIT_AS2REL_URL
        )
    }

    /// Get explanation text for single-homed results
    pub fn get_single_homed_explanation(&self, upstream_asn: u32) -> String {
        let max_peers = self.get_max_peers_count();
        format!(
            "Single-homed ASNs to AS{}\n\
             These ASNs have AS{} as their ONLY upstream provider.\n\
             \n\
             - connected: % of {} peers that see this relationship\n\
             \n\
             Data source: {}\n",
            upstream_asn, upstream_asn, max_peers, BGPKIT_AS2REL_URL
        )
    }

    /// Format results for output
    ///
    /// When `truncate_names` is true, names are truncated to 20 characters for table output.
    /// JSON output never truncates names.
    pub fn format_results(
        &self,
        results: &[As2relSearchResult],
        format: &As2relOutputFormat,
        show_name: bool,
        truncate_names: bool,
    ) -> String {
        match format {
            As2relOutputFormat::Json => {
                let max_peers = self.get_max_peers_count();
                let json_results: Vec<_> = results
                    .iter()
                    .map(|r| {
                        if show_name {
                            json!({
                                "asn1": r.asn1,
                                "asn2": r.asn2,
                                "asn2_name": r.asn2_name.as_deref().unwrap_or(""),
                                "connected": &r.connected,
                                "peer": &r.peer,
                                "as1_upstream": &r.as1_upstream,
                                "as2_upstream": &r.as2_upstream,
                            })
                        } else {
                            json!({
                                "asn1": r.asn1,
                                "asn2": r.asn2,
                                "connected": &r.connected,
                                "peer": &r.peer,
                                "as1_upstream": &r.as1_upstream,
                                "as2_upstream": &r.as2_upstream,
                            })
                        }
                    })
                    .collect();
                let output = json!({
                    "max_peers_count": max_peers,
                    "results": json_results,
                });
                serde_json::to_string_pretty(&output).unwrap_or_default()
            }
            As2relOutputFormat::Pretty => {
                #[cfg(feature = "display")]
                {
                    use tabled::settings::Style;
                    use tabled::Table;
                    if show_name {
                        let results_with_name: Vec<_> = results
                            .iter()
                            .cloned()
                            .map(|r| r.with_name(truncate_names))
                            .collect();
                        Table::new(&results_with_name)
                            .with(Style::rounded())
                            .to_string()
                    } else {
                        Table::new(results).with(Style::rounded()).to_string()
                    }
                }
                #[cfg(not(feature = "display"))]
                {
                    // Fall back to JSON when display feature is not enabled
                    self.format_results(
                        results,
                        &As2relOutputFormat::Json,
                        show_name,
                        truncate_names,
                    )
                }
            }
            As2relOutputFormat::Markdown => {
                #[cfg(feature = "display")]
                {
                    use tabled::settings::Style;
                    use tabled::Table;
                    if show_name {
                        let results_with_name: Vec<_> = results
                            .iter()
                            .cloned()
                            .map(|r| r.with_name(truncate_names))
                            .collect();
                        Table::new(&results_with_name)
                            .with(Style::markdown())
                            .to_string()
                    } else {
                        Table::new(results).with(Style::markdown()).to_string()
                    }
                }
                #[cfg(not(feature = "display"))]
                {
                    // Fall back to JSON when display feature is not enabled
                    self.format_results(
                        results,
                        &As2relOutputFormat::Json,
                        show_name,
                        truncate_names,
                    )
                }
            }
        }
    }

    /// Format results as JSON
    ///
    /// This is a convenience method that always works regardless of features.
    pub fn format_json(&self, results: &[As2relSearchResult], pretty: bool) -> String {
        let max_peers = self.get_max_peers_count();
        let json_results: Vec<_> = results
            .iter()
            .map(|r| {
                json!({
                    "asn1": r.asn1,
                    "asn2": r.asn2,
                    "asn2_name": r.asn2_name.as_deref().unwrap_or(""),
                    "connected": &r.connected,
                    "peer": &r.peer,
                    "as1_upstream": &r.as1_upstream,
                    "as2_upstream": &r.as2_upstream,
                })
            })
            .collect();
        let output = json!({
            "max_peers_count": max_peers,
            "results": json_results,
        });
        if pretty {
            serde_json::to_string_pretty(&output).unwrap_or_default()
        } else {
            serde_json::to_string(&output).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_creation() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = As2relLens::new(&db);
        assert!(!lens.is_data_available());
        assert!(lens.needs_update());
    }

    #[test]
    fn test_get_explanation() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = As2relLens::new(&db);

        let explanation = lens.get_explanation();
        assert!(explanation.contains("connected"));
        assert!(explanation.contains("peer"));
    }

    #[test]
    fn test_get_single_homed_explanation() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = As2relLens::new(&db);

        let explanation = lens.get_single_homed_explanation(2914);
        assert!(explanation.contains("2914"));
        assert!(explanation.contains("Single-homed"));
        assert!(explanation.contains("ONLY upstream"));
    }

    #[test]
    fn test_sort_results() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = As2relLens::new(&db);

        let mut results = vec![
            As2relSearchResult {
                asn1: 65000,
                asn2: 65002,
                asn2_name: None,
                connected: "30.0%".to_string(),
                connected_pct: 30.0,
                peer: "50.0%".to_string(),
                as1_upstream: "25.0%".to_string(),
                as2_upstream: "25.0%".to_string(),
            },
            As2relSearchResult {
                asn1: 65000,
                asn2: 65001,
                asn2_name: None,
                connected: "50.0%".to_string(),
                connected_pct: 50.0,
                peer: "50.0%".to_string(),
                as1_upstream: "25.0%".to_string(),
                as2_upstream: "25.0%".to_string(),
            },
        ];

        lens.sort_results(&mut results, &As2relSortOrder::ConnectedDesc);
        assert_eq!(results[0].asn2, 65001); // Higher connected first

        lens.sort_results(&mut results, &As2relSortOrder::Asn2Asc);
        assert_eq!(results[0].asn2, 65001); // Lower ASN first
    }

    #[test]
    fn test_search_args_with_filters() {
        // Test that args with filters validate correctly
        let args = As2relSearchArgs::new(2914).single_homed_only();
        assert!(args.validate().is_ok());
        assert!(args.single_homed);

        let args = As2relSearchArgs::new(2914).upstream_only();
        assert!(args.validate().is_ok());
        assert!(args.is_upstream);

        let args = As2relSearchArgs::new(2914).with_min_visibility(10.0);
        assert!(args.validate().is_ok());
        assert_eq!(args.min_visibility, Some(10.0));
    }

    #[test]
    fn test_multi_asn_args() {
        let args = As2relSearchArgs::multiple(vec![174, 2914, 3356]);
        assert!(args.validate().is_ok());
        assert!(args.is_multi_lookup());
        assert!(!args.is_single_lookup());
        assert!(!args.is_pair_lookup());
    }
}
