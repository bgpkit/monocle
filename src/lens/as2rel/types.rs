//! AS2Rel lens types
//!
//! This module defines the types used by the AS2Rel lens for relationship
//! queries and result formatting.

use crate::lens::utils::{truncate_name, DEFAULT_NAME_MAX_LEN};
use serde::{Deserialize, Serialize};
use tabled::Tabled;

/// Sort order for search results
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum As2relSortOrder {
    /// Sort by connected percentage descending (default)
    #[default]
    ConnectedDesc,
    /// Sort by ASN2 ascending
    Asn2Asc,
}

/// Output format for results
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum As2relOutputFormat {
    /// Markdown table
    #[default]
    Markdown,
    /// Pretty table with borders
    Pretty,
    /// JSON output
    Json,
}

/// Search result for AS relationships
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct As2relSearchResult {
    pub asn1: u32,
    pub asn2: u32,
    #[tabled(skip)]
    pub asn2_name: Option<String>,
    /// Percentage of peers that see the connection (formatted string)
    pub connected: String,
    #[tabled(skip)]
    pub connected_pct: f32,
    /// Percentage that see peer relationship
    pub peer: String,
    /// Percentage that see ASN1 as upstream
    pub as1_upstream: String,
    /// Percentage that see ASN2 as upstream
    pub as2_upstream: String,
}

/// Search result with name displayed (for table output)
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct As2relSearchResultWithName {
    pub asn1: u32,
    pub asn2: u32,
    pub asn2_name: String,
    pub connected: String,
    pub peer: String,
    pub as1_upstream: String,
    pub as2_upstream: String,
}

impl As2relSearchResult {
    /// Convert to a result with name for display (with optional truncation)
    pub fn with_name(self, truncate: bool) -> As2relSearchResultWithName {
        let name = self.asn2_name.unwrap_or_default();
        let display_name = if truncate {
            truncate_name(&name, DEFAULT_NAME_MAX_LEN)
        } else {
            name
        };
        As2relSearchResultWithName {
            asn1: self.asn1,
            asn2: self.asn2,
            asn2_name: display_name,
            connected: self.connected,
            peer: self.peer,
            as1_upstream: self.as1_upstream,
            as2_upstream: self.as2_upstream,
        }
    }

    /// Create a search result from aggregated data
    pub fn from_aggregated(
        asn1: u32,
        asn2: u32,
        asn2_name: Option<String>,
        connected_count: u32,
        as1_upstream_count: u32,
        as2_upstream_count: u32,
        max_peers: u32,
    ) -> Self {
        // All percentages are relative to max_peers
        // connected comes from rel=0 (total peers seeing any connection)
        // as1_upstream/as2_upstream come from rel=1/-1 (subsets seeing provider-customer)
        // peer = connected - as1_upstream - as2_upstream (remainder seeing pure peering)
        let format_pct = |count: u32| -> String {
            if max_peers > 0 {
                format!("{:.1}%", (count as f32 / max_peers as f32) * 100.0)
            } else {
                "0.0%".to_string()
            }
        };

        let connected_pct = if max_peers > 0 {
            (connected_count as f32 / max_peers as f32) * 100.0
        } else {
            0.0
        };

        // peer is the remainder after subtracting upstream counts from connected
        let peer_count = connected_count
            .saturating_sub(as1_upstream_count)
            .saturating_sub(as2_upstream_count);

        Self {
            asn1,
            asn2,
            asn2_name,
            connected: format!("{:.1}%", connected_pct),
            connected_pct,
            peer: format_pct(peer_count),
            as1_upstream: format_pct(as1_upstream_count),
            as2_upstream: format_pct(as2_upstream_count),
        }
    }
}

/// Metadata about AS2Rel data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct As2relDataMeta {
    pub file_url: String,
    pub last_updated: u64,
    pub max_peers_count: u32,
}

/// Progress update for data loading operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct As2relUpdateProgress {
    pub stage: As2relUpdateStage,
    pub current: usize,
    pub total: Option<usize>,
    pub message: String,
}

/// Stage of a data update operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum As2relUpdateStage {
    /// Downloading data from source
    Downloading,
    /// Parsing downloaded data
    Parsing,
    /// Inserting data into database
    Inserting,
    /// Operation completed successfully
    Complete,
    /// Operation failed with error
    Error,
}

impl As2relUpdateProgress {
    pub fn downloading(url: &str) -> Self {
        Self {
            stage: As2relUpdateStage::Downloading,
            current: 0,
            total: None,
            message: format!("Downloading from {}...", url),
        }
    }

    pub fn parsing() -> Self {
        Self {
            stage: As2relUpdateStage::Parsing,
            current: 0,
            total: None,
            message: "Parsing data...".to_string(),
        }
    }

    pub fn inserting(current: usize, total: usize) -> Self {
        Self {
            stage: As2relUpdateStage::Inserting,
            current,
            total: Some(total),
            message: format!("Inserting records ({}/{})", current, total),
        }
    }

    pub fn complete(entry_count: usize) -> Self {
        Self {
            stage: As2relUpdateStage::Complete,
            current: entry_count,
            total: Some(entry_count),
            message: format!("Complete: {} relationship entries", entry_count),
        }
    }

    pub fn error(message: &str) -> Self {
        Self {
            stage: As2relUpdateStage::Error,
            current: 0,
            total: None,
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_from_aggregated() {
        let result = As2relSearchResult::from_aggregated(
            65000,
            65001,
            Some("Test Org".to_string()),
            100, // connected (from rel=0)
            30,  // as1_upstream (from rel=1/-1)
            20,  // as2_upstream (from rel=1/-1)
            200, // max_peers
        );

        assert_eq!(result.asn1, 65000);
        assert_eq!(result.asn2, 65001);
        assert_eq!(result.asn2_name, Some("Test Org".to_string()));
        // All percentages relative to max_peers (200)
        assert_eq!(result.connected, "50.0%"); // 100/200
                                               // peer = connected - as1_upstream - as2_upstream = 100 - 30 - 20 = 50
        assert_eq!(result.peer, "25.0%"); // 50/200
        assert_eq!(result.as1_upstream, "15.0%"); // 30/200
        assert_eq!(result.as2_upstream, "10.0%"); // 20/200
    }

    #[test]
    fn test_search_result_with_name() {
        let result = As2relSearchResult {
            asn1: 65000,
            asn2: 65001,
            asn2_name: Some("Test Org".to_string()),
            connected: "50.0%".to_string(),
            connected_pct: 50.0,
            peer: "50.0%".to_string(),
            as1_upstream: "30.0%".to_string(),
            as2_upstream: "20.0%".to_string(),
        };

        let with_name = result.with_name(false);
        assert_eq!(with_name.asn2_name, "Test Org");
    }

    #[test]
    fn test_update_progress() {
        let prog = As2relUpdateProgress::downloading("https://example.com/data.json");
        assert_eq!(prog.stage, As2relUpdateStage::Downloading);

        let prog = As2relUpdateProgress::complete(1000);
        assert_eq!(prog.stage, As2relUpdateStage::Complete);
        assert_eq!(prog.current, 1000);
    }
}
