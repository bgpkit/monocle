//! AS2Rel lens
//!
//! This module provides the AS2Rel lens for querying AS-level relationships.
//! It uses SQLite as the backend database.

pub mod args;
pub mod types;

pub use args::{As2relOutputArgs, As2relSearchArgs, As2relUpdateArgs};
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
    pub fn search(&self, args: &As2relSearchArgs) -> Result<Vec<As2relSearchResult>> {
        let max_peers = self.get_max_peers_count();

        // Always use the aggregating search methods - they properly combine
        // multiple records for the same ASN pair into a single result
        let aggregated = if args.asns.len() == 1 {
            let asn = args.asns[0];
            self.db.as2rel().search_asn_with_names(asn)?
        } else {
            let asn1 = args.asns[0];
            let asn2 = args.asns[1];
            self.db.as2rel().search_pair_with_names(asn1, asn2)?
        };

        // Convert to As2relSearchResult with percentages
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

        // Sort results
        self.sort_results(&mut results, &args.sort_order());

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
}
