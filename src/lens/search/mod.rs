//! Search lens module
//!
//! This module provides filter types for searching BGP messages across multiple MRT files.
//! The filter types can optionally derive Clap's Args trait when the `cli` feature is enabled.

use crate::lens::parse::ParseFilters;
use anyhow::Result;
use bgpkit_broker::BrokerItem;
use bgpkit_parser::BgpkitParser;
use serde::{Deserialize, Serialize};
use std::io::Read;

#[cfg(feature = "cli")]
use clap::{Args, ValueEnum};

// =============================================================================
// Types
// =============================================================================

/// Dump type for BGP data
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(ValueEnum))]
pub enum SearchDumpType {
    /// BGP updates only
    #[default]
    Updates,
    /// BGP RIB dump only
    Rib,
    /// BGP RIB dump and BGP updates
    RibUpdates,
}

// =============================================================================
// Args
// =============================================================================

/// Filters for searching BGP messages across multiple MRT files
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(Args))]
pub struct SearchFilters {
    #[cfg_attr(feature = "cli", clap(flatten))]
    #[serde(flatten)]
    pub parse_filters: ParseFilters,

    /// Filter by collector, e.g., rrc00 or route-views2
    #[cfg_attr(feature = "cli", clap(short = 'c', long))]
    pub collector: Option<String>,

    /// Filter by route collection project, i.e., riperis or routeviews
    #[cfg_attr(feature = "cli", clap(short = 'P', long))]
    pub project: Option<String>,

    /// Specify data dump type to search (updates or RIB dump)
    #[cfg_attr(feature = "cli", clap(short = 'D', long, default_value_t, value_enum))]
    #[serde(default)]
    pub dump_type: SearchDumpType,
}

impl SearchFilters {
    /// Query broker items based on filters
    pub fn to_broker_items(&self) -> Result<Vec<BrokerItem>> {
        self.build_broker()?
            .query()
            .map_err(|_| anyhow::anyhow!("broker query error: please check filters are valid"))
    }

    /// Build a broker from the filters
    pub fn build_broker(&self) -> Result<bgpkit_broker::BgpkitBroker> {
        let (ts_start, ts_end) = self.parse_filters.parse_start_end_strings()?;

        let mut broker = bgpkit_broker::BgpkitBroker::new()
            .ts_start(ts_start)
            .ts_end(ts_end)
            .page_size(1000);

        if let Some(project) = &self.project {
            broker = broker.project(project.as_str());
        }
        if let Some(collector) = &self.collector {
            broker = broker.collector_id(collector.as_str());
        }

        match self.dump_type {
            SearchDumpType::Updates => {
                broker = broker.data_type("updates");
            }
            SearchDumpType::Rib => {
                broker = broker.data_type("rib");
            }
            SearchDumpType::RibUpdates => {
                // do nothing here -> getting all RIB and updates
            }
        }

        Ok(broker)
    }

    /// Validate the filters
    pub fn validate(&self) -> Result<()> {
        let _ = self.parse_filters.parse_start_end_strings()?;
        Ok(())
    }

    /// Convert filters to a BgpkitParser for a given file
    pub fn to_parser(&self, file_path: &str) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        self.parse_filters.to_parser(file_path)
    }
}

// =============================================================================
// Lens
// =============================================================================

/// Search lens for BGP message search operations
///
/// This lens provides high-level operations for searching BGP messages
/// across multiple MRT files using the BGPKIT broker.
pub struct SearchLens;

impl SearchLens {
    /// Create a new search lens
    pub fn new() -> Self {
        Self
    }

    /// Query broker items based on filters
    pub fn query_broker(&self, filters: &SearchFilters) -> Result<Vec<BrokerItem>> {
        filters.to_broker_items()
    }

    /// Build a broker from filters
    pub fn build_broker(&self, filters: &SearchFilters) -> Result<bgpkit_broker::BgpkitBroker> {
        filters.build_broker()
    }

    /// Create a parser for a specific file
    pub fn create_parser(
        &self,
        filters: &SearchFilters,
        file_path: &str,
    ) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        filters.to_parser(file_path)
    }

    /// Validate filters
    pub fn validate_filters(&self, filters: &SearchFilters) -> Result<()> {
        filters.validate()
    }
}

impl Default for SearchLens {
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
    fn test_pagination_logic() {
        // Create a test filter with a short time range to get manageable results
        let search_filters = SearchFilters {
            parse_filters: ParseFilters {
                origin_asn: None,
                prefix: None,
                include_super: false,
                include_sub: false,
                peer_ip: Vec::new(),
                peer_asn: None,
                elem_type: None,
                start_ts: Some("2022-01-01T00:00:00Z".to_string()),
                end_ts: Some("2022-01-01T01:00:00Z".to_string()), // 1 hour window
                duration: None,
                as_path: None,
            },
            collector: None,
            project: None,
            dump_type: SearchDumpType::Updates,
        };

        // Test broker creation
        let base_broker = search_filters
            .build_broker()
            .expect("Failed to build broker");

        // Test pagination with small page size for testing
        let test_broker = base_broker.clone().page_size(10); // Small page for testing

        let mut total_items = 0;
        let mut page = 1i64;
        let mut pages_processed = 0;

        // Test pagination loop similar to main implementation
        loop {
            let items = match test_broker.clone().page(page).query_single_page() {
                Ok(items) => items,
                Err(e) => {
                    println!("Failed to fetch page {}: {}", page, e);
                    break;
                }
            };

            if items.is_empty() {
                println!("Reached empty page {}, stopping", page);
                break;
            }

            total_items += items.len();
            pages_processed += 1;

            println!(
                "Page {}: {} items (total: {})",
                page,
                items.len(),
                total_items
            );

            // Verify items have timestamps
            if let Some(first_item) = items.first() {
                println!(
                    "  First item timestamp: {}",
                    first_item.ts_start.format("%Y-%m-%d %H:%M UTC")
                );
            }

            page += 1;

            // Safety check to prevent infinite loops in test
            if pages_processed >= 5 || items.len() < 10 {
                println!(
                    "Test complete: processed {} pages with {} total items",
                    pages_processed, total_items
                );
                break;
            }
        }

        // Verify we processed some data
        assert!(total_items > 0, "Should have found some items");
        assert!(
            pages_processed > 0,
            "Should have processed at least one page"
        );

        println!("Pagination test completed successfully");
    }

    #[test]
    fn test_build_broker_with_filters() {
        let search_filters = SearchFilters {
            parse_filters: ParseFilters {
                origin_asn: None,
                prefix: None,
                include_super: false,
                include_sub: false,
                peer_ip: Vec::new(),
                peer_asn: None,
                elem_type: None,
                start_ts: Some("2022-01-01T00:00:00Z".to_string()),
                end_ts: Some("2022-01-01T01:00:00Z".to_string()),
                duration: None,
                as_path: None,
            },
            collector: Some("rrc00".to_string()),
            project: Some("riperis".to_string()),
            dump_type: SearchDumpType::Updates,
        };

        let broker = search_filters
            .build_broker()
            .expect("Failed to build broker");

        // Test that we can get at least one page
        let items = broker
            .page(1)
            .query_single_page()
            .expect("Failed to query first page");

        println!("First page with filters: {} items", items.len());

        // Verify all items match the collector filter if any items found
        if !items.is_empty() {
            for item in &items {
                assert_eq!(
                    item.collector_id, "rrc00",
                    "Item collector should match filter"
                );
            }
            println!("All items correctly filtered by collector");
        }
    }
}
