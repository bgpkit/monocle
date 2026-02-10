//! AS2Rel handlers for AS-level relationship lookup operations
//!
//! This module provides handlers for AS2Rel-related methods like `as2rel.search`,
//! `as2rel.relationship`, and `as2rel.update`.

use crate::database::MonocleDatabase;
use crate::lens::as2rel::{As2relLens, As2relSearchArgs, As2relSearchResult, As2relSortOrder};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// as2rel.search
// =============================================================================

/// Parameters for as2rel.search
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct As2relSearchParams {
    /// ASN(s) to search for (1 or more ASNs)
    ///
    /// - Single ASN: shows all relationships for that ASN
    /// - Two ASNs: shows the relationship between them
    /// - Multiple ASNs: shows relationships for all pairs (asn1 < asn2)
    #[serde(default)]
    pub asns: Vec<u32>,

    /// Sort by ASN instead of connection percentage
    #[serde(default)]
    pub sort_by_asn: Option<bool>,

    /// Show AS names in results
    #[serde(default)]
    pub show_name: Option<bool>,

    /// Minimum visibility percentage (0-100) to include in results
    ///
    /// Filters out relationships seen by fewer than this percentage of peers.
    #[serde(default)]
    pub min_visibility: Option<f32>,

    /// Only show ASNs that are single-homed to the queried ASN
    ///
    /// An ASN is single-homed if it has exactly one upstream provider.
    /// Only applicable when querying a single ASN.
    #[serde(default)]
    pub single_homed: Option<bool>,

    /// Only show relationships where the queried ASN is an upstream (provider)
    ///
    /// Shows the downstream customers of the queried ASN.
    /// Only applicable when querying a single ASN.
    #[serde(default)]
    pub is_upstream: Option<bool>,

    /// Only show relationships where the queried ASN is a downstream (customer)
    ///
    /// Shows the upstream providers of the queried ASN.
    /// Only applicable when querying a single ASN.
    #[serde(default)]
    pub is_downstream: Option<bool>,

    /// Only show peer relationships
    ///
    /// Only applicable when querying a single ASN.
    #[serde(default)]
    pub is_peer: Option<bool>,
}

/// Response for as2rel.search
#[derive(Debug, Clone, Serialize)]
pub struct As2relSearchResponse {
    /// Maximum peers count (for percentage calculation reference)
    pub max_peers_count: u32,

    /// Search results
    pub results: Vec<As2relSearchResult>,
}

/// Handler for as2rel.search method
pub struct As2relSearchHandler;

#[async_trait]
impl WsMethod for As2relSearchHandler {
    const METHOD: &'static str = "as2rel.search";
    const IS_STREAMING: bool = false;

    type Params = As2relSearchParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        if params.asns.is_empty() {
            return Err(WsError::invalid_params("At least one ASN is required"));
        }

        // Validate single-ASN-only filters
        if params.asns.len() != 1 {
            if params.single_homed.unwrap_or(false) {
                return Err(WsError::invalid_params(
                    "--single-homed can only be used with a single ASN",
                ));
            }
            if params.is_upstream.unwrap_or(false)
                || params.is_downstream.unwrap_or(false)
                || params.is_peer.unwrap_or(false)
            {
                return Err(WsError::invalid_params(
                    "--is-upstream, --is-downstream, and --is-peer can only be used with a single ASN",
                ));
            }
        }

        // Validate min_visibility range
        if let Some(min_vis) = params.min_visibility {
            if !(0.0..=100.0).contains(&min_vis) {
                return Err(WsError::invalid_params(
                    "--min-visibility must be between 0 and 100",
                ));
            }
        }

        // Validate mutually exclusive relationship filters
        let filter_count = [
            params.is_upstream.unwrap_or(false),
            params.is_downstream.unwrap_or(false),
            params.is_peer.unwrap_or(false),
        ]
        .iter()
        .filter(|&&x| x)
        .count();

        if filter_count > 1 {
            return Err(WsError::invalid_params(
                "Only one of --is-upstream, --is-downstream, or --is-peer can be specified",
            ));
        }

        Ok(())
    }

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // NOTE: `MonocleDatabase`/`As2relLens<'_>` are not `Send`. This `handle()` must return a
        // `Send` future, so we must not hold a DB-backed lens across an `.await`.
        //
        // Do all DB work synchronously first, then await only for sending the response.
        let response = {
            // Open the database
            let db = MonocleDatabase::open_in_dir(ctx.data_dir()).map_err(|e| {
                WsError::operation_failed(format!("Failed to open database: {}", e))
            })?;

            let lens = As2relLens::new(&db);

            // Check if data is available
            if !lens.is_data_available() {
                return Err(WsError::not_initialized("AS2Rel"));
            }

            // Build search args
            let sort_order = if params.sort_by_asn.unwrap_or(false) {
                As2relSortOrder::Asn2Asc
            } else {
                As2relSortOrder::ConnectedDesc
            };

            let args = As2relSearchArgs {
                asns: params.asns,
                sort_by_asn: params.sort_by_asn.unwrap_or(false),
                show_name: params.show_name.unwrap_or(false),
                min_visibility: params.min_visibility,
                single_homed: params.single_homed.unwrap_or(false),
                is_upstream: params.is_upstream.unwrap_or(false),
                is_downstream: params.is_downstream.unwrap_or(false),
                is_peer: params.is_peer.unwrap_or(false),
                ..Default::default()
            };

            // Get max peers count
            let max_peers_count = lens.get_max_peers_count();

            // Perform the search
            let mut results = lens
                .search(&args)
                .map_err(|e| WsError::operation_failed(e.to_string()))?;

            // Sort results
            lens.sort_results(&mut results, &sort_order);

            As2relSearchResponse {
                max_peers_count,
                results,
            }
        };

        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// as2rel.relationship
// =============================================================================

/// Parameters for as2rel.relationship
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct As2relRelationshipParams {
    /// First ASN
    pub asn1: u32,

    /// Second ASN
    pub asn2: u32,
}

/// Response for as2rel.relationship
#[derive(Debug, Clone, Serialize)]
pub struct As2relRelationshipResponse {
    /// Relationship result (if found)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<As2relSearchResult>,

    /// Whether a relationship was found
    pub found: bool,
}

/// Handler for as2rel.relationship method
pub struct As2relRelationshipHandler;

#[async_trait]
impl WsMethod for As2relRelationshipHandler {
    const METHOD: &'static str = "as2rel.relationship";
    const IS_STREAMING: bool = false;

    type Params = As2relRelationshipParams;

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // NOTE: `MonocleDatabase`/`As2relLens<'_>` are not `Send`. This `handle()` must return a
        // `Send` future, so we must not hold a DB-backed lens across an `.await`.
        //
        // We do all DB work synchronously first, then await only for sending the response.

        // Open the database
        let db = MonocleDatabase::open_in_dir(ctx.data_dir())
            .map_err(|e| WsError::operation_failed(format!("Failed to open database: {}", e)))?;

        // Run search (DB-first) without any `.await`
        let results = {
            let lens = As2relLens::new(&db);

            // Check if data is available
            if !lens.is_data_available() {
                return Err(WsError::not_initialized("AS2Rel"));
            }

            let args = As2relSearchArgs {
                asns: vec![params.asn1, params.asn2],
                ..Default::default()
            };

            lens.search(&args)
                .map_err(|e| WsError::operation_failed(e.to_string()))?
        };

        let response = As2relRelationshipResponse {
            found: !results.is_empty(),
            relationship: results.into_iter().next(),
        };

        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// as2rel.update
// =============================================================================

/// Parameters for as2rel.update
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct As2relUpdateParams {
    /// (Deprecated in WS) Custom URL to fetch data from (optional).
    ///
    /// DB-first policy: WebSocket handlers must not fetch remote data. Use
    /// `database.refresh` (source=`as2rel`) to update local DB instead.
    #[serde(default)]
    pub url: Option<String>,
}

/// Response for as2rel.update
#[derive(Debug, Clone, Serialize)]
pub struct As2relUpdateResponse {
    /// Whether update was performed
    pub updated: bool,

    /// Number of entries loaded
    pub count: usize,

    /// Message
    pub message: String,
}

/// Handler for as2rel.update method
pub struct As2relUpdateHandler;

#[async_trait]
impl WsMethod for As2relUpdateHandler {
    const METHOD: &'static str = "as2rel.update";
    const IS_STREAMING: bool = false;

    type Params = As2relUpdateParams;

    async fn handle(
        _ctx: Arc<WsContext>,
        _req: WsRequest,
        _params: Self::Params,
        _sink: WsOpSink,
    ) -> WsResult<()> {
        // DB-first policy: this WebSocket method must not perform network fetches.
        // Use `database.refresh` for `as2rel` to populate/update the local database.
        Err(WsError::not_initialized(
            "AS2Rel (WebSocket is DB-first; run database.refresh source=as2rel)",
        ))
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_as2rel_search_params_default() {
        let params = As2relSearchParams::default();
        assert!(params.asns.is_empty());
        assert!(params.sort_by_asn.is_none());
        assert!(params.show_name.is_none());
        assert!(params.min_visibility.is_none());
        assert!(params.single_homed.is_none());
        assert!(params.is_upstream.is_none());
        assert!(params.is_downstream.is_none());
        assert!(params.is_peer.is_none());
    }

    #[test]
    fn test_as2rel_search_params_deserialization() {
        let json = r#"{"asns": [13335], "show_name": true}"#;
        let params: As2relSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.asns.len(), 1);
        assert_eq!(params.asns[0], 13335);
        assert_eq!(params.show_name, Some(true));
    }

    #[test]
    fn test_as2rel_search_params_with_filters() {
        let json = r#"{"asns": [2914], "single_homed": true, "min_visibility": 10.0}"#;
        let params: As2relSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.asns, vec![2914]);
        assert_eq!(params.single_homed, Some(true));
        assert_eq!(params.min_visibility, Some(10.0));
    }

    #[test]
    fn test_as2rel_search_params_validation() {
        // Empty ASNs should fail
        let params = As2relSearchParams::default();
        assert!(As2relSearchHandler::validate(&params).is_err());

        // Single ASN should pass
        let params = As2relSearchParams {
            asns: vec![13335],
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_ok());

        // Two ASNs should pass
        let params = As2relSearchParams {
            asns: vec![13335, 174],
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_ok());

        // Multiple ASNs should pass (new behavior)
        let params = As2relSearchParams {
            asns: vec![13335, 174, 3356],
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_ok());
    }

    #[test]
    fn test_as2rel_search_params_single_homed_validation() {
        // Single-homed with single ASN should pass
        let params = As2relSearchParams {
            asns: vec![2914],
            single_homed: Some(true),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_ok());

        // Single-homed with multiple ASNs should fail
        let params = As2relSearchParams {
            asns: vec![2914, 174],
            single_homed: Some(true),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_err());
    }

    #[test]
    fn test_as2rel_search_params_relationship_filter_validation() {
        // is_upstream with single ASN should pass
        let params = As2relSearchParams {
            asns: vec![2914],
            is_upstream: Some(true),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_ok());

        // is_upstream with multiple ASNs should fail
        let params = As2relSearchParams {
            asns: vec![2914, 174],
            is_upstream: Some(true),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_err());

        // Multiple relationship filters should fail
        let params = As2relSearchParams {
            asns: vec![2914],
            is_upstream: Some(true),
            is_downstream: Some(true),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_err());
    }

    #[test]
    fn test_as2rel_search_params_min_visibility_validation() {
        // Valid min_visibility should pass
        let params = As2relSearchParams {
            asns: vec![2914],
            min_visibility: Some(50.0),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_ok());

        // Out of range min_visibility should fail
        let params = As2relSearchParams {
            asns: vec![2914],
            min_visibility: Some(-1.0),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_err());

        let params = As2relSearchParams {
            asns: vec![2914],
            min_visibility: Some(101.0),
            ..Default::default()
        };
        assert!(As2relSearchHandler::validate(&params).is_err());
    }

    #[test]
    fn test_as2rel_relationship_params_deserialization() {
        let json = r#"{"asn1": 13335, "asn2": 174}"#;
        let params: As2relRelationshipParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.asn1, 13335);
        assert_eq!(params.asn2, 174);
    }

    #[test]
    fn test_as2rel_update_params_default() {
        let params = As2relUpdateParams::default();
        assert!(params.url.is_none());
    }

    #[test]
    fn test_as2rel_update_params_deserialization() {
        let json = r#"{"url": "https://example.com/data.json"}"#;
        let params: As2relUpdateParams = serde_json::from_str(json).unwrap();
        assert_eq!(
            params.url,
            Some("https://example.com/data.json".to_string())
        );
    }

    #[test]
    fn test_as2rel_search_response_serialization() {
        let response = As2relSearchResponse {
            max_peers_count: 1000,
            results: vec![As2relSearchResult {
                asn1: 13335,
                asn2: 174,
                asn2_name: Some("COGENT-174".to_string()),
                connected: "85.3%".to_string(),
                connected_pct: 85.3,
                peer: "45.2%".to_string(),
                as1_upstream: "20.1%".to_string(),
                as2_upstream: "20.0%".to_string(),
            }],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"max_peers_count\":1000"));
        assert!(json.contains("13335"));
        assert!(json.contains("174"));
        assert!(json.contains("85.3%"));
    }

    #[test]
    fn test_as2rel_relationship_response_serialization() {
        let response = As2relRelationshipResponse {
            found: true,
            relationship: Some(As2relSearchResult {
                asn1: 13335,
                asn2: 174,
                asn2_name: None,
                connected: "50.0%".to_string(),
                connected_pct: 50.0,
                peer: "50.0%".to_string(),
                as1_upstream: "25.0%".to_string(),
                as2_upstream: "25.0%".to_string(),
            }),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"found\":true"));
        assert!(json.contains("13335"));
    }

    #[test]
    fn test_as2rel_update_response_serialization() {
        let response = As2relUpdateResponse {
            updated: true,
            count: 500,
            message: "Successfully loaded 500 entries".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"updated\":true"));
        assert!(json.contains("\"count\":500"));
    }
}
