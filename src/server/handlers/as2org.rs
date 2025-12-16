//! AS2Org handlers for AS-to-Organization lookup operations
//!
//! This module provides handlers for AS2Org-related methods like `as2org.search`
//! and `as2org.bootstrap`.

use crate::database::MonocleDatabase;
use crate::lens::as2org::{As2orgLens, As2orgSearchArgs, As2orgSearchResult};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

fn deserialize_string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        One(String),
        Many(Vec<String>),
    }

    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::One(s) => Ok(vec![s]),
        StringOrVec::Many(v) => Ok(v),
    }
}

// =============================================================================
// as2org.search
// =============================================================================

/// Parameters for as2org.search
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct As2orgSearchParams {
    /// Search queries (ASN or name).
    ///
    /// Backward compatibility: accept either a single string or an array of strings.
    /// Example (string):  {"query":"cloudflare"}
    /// Example (array):   {"query":["13335","cloudflare"]}
    #[serde(default, deserialize_with = "deserialize_string_or_vec")]
    pub query: Vec<String>,

    /// Search by ASN only
    #[serde(default)]
    pub asn_only: Option<bool>,

    /// Search by name only
    #[serde(default)]
    pub name_only: Option<bool>,

    /// Search by country only
    #[serde(default)]
    pub country_only: Option<bool>,

    /// Show full country name instead of code
    #[serde(default)]
    pub full_country: Option<bool>,

    /// Return full table (all fields)
    #[serde(default)]
    pub full_table: Option<bool>,
}

/// Response for as2org.search
#[derive(Debug, Clone, Serialize)]
pub struct As2orgSearchResponse {
    /// Search results
    pub results: Vec<As2orgSearchResult>,
}

/// Handler for as2org.search method
pub struct As2orgSearchHandler;

#[async_trait]
impl WsMethod for As2orgSearchHandler {
    const METHOD: &'static str = "as2org.search";
    const IS_STREAMING: bool = false;

    type Params = As2orgSearchParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        if params.query.is_empty() {
            return Err(WsError::invalid_params("At least one query is required"));
        }

        // Check for conflicting options
        let exclusive_count = [params.asn_only, params.name_only, params.country_only]
            .iter()
            .filter(|x| x.unwrap_or(false))
            .count();

        if exclusive_count > 1 {
            return Err(WsError::invalid_params(
                "Only one of asn_only, name_only, or country_only can be set",
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
        // NOTE: `MonocleDatabase`/`As2orgLens<'_>` are not `Send`. This `handle()` must return a
        // `Send` future, so we must not hold a DB-backed lens across an `.await`.
        //
        // We do all DB work synchronously first, then await only for sending the response.

        // Open the database
        let db = MonocleDatabase::open_in_dir(&ctx.data_dir)
            .map_err(|e| WsError::operation_failed(format!("Failed to open database: {}", e)))?;

        // Build search args
        let args = As2orgSearchArgs {
            query: params.query,
            asn_only: params.asn_only.unwrap_or(false),
            name_only: params.name_only.unwrap_or(false),
            country_only: params.country_only.unwrap_or(false),
            full_country: params.full_country.unwrap_or(false),
            full_table: params.full_table.unwrap_or(false),
            ..Default::default()
        };

        // Perform DB work without any `.await`
        let results = {
            let lens = As2orgLens::new(&db);

            // Check if data is available
            if !lens.is_data_available() {
                return Err(WsError::not_initialized("AS2Org"));
            }

            lens.search(&args)
                .map_err(|e| WsError::operation_failed(e.to_string()))?
        };

        // Send response
        let response = As2orgSearchResponse { results };
        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// as2org.bootstrap
// =============================================================================

/// Parameters for as2org.bootstrap
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct As2orgBootstrapParams {
    /// Force re-bootstrap even if data exists
    #[serde(default)]
    pub force: Option<bool>,
}

/// Response for as2org.bootstrap
#[derive(Debug, Clone, Serialize)]
pub struct As2orgBootstrapResponse {
    /// Whether bootstrap was performed
    pub bootstrapped: bool,

    /// Number of entries loaded
    pub count: usize,

    /// Message
    pub message: String,
}

/// Handler for as2org.bootstrap method
pub struct As2orgBootstrapHandler;

#[async_trait]
impl WsMethod for As2orgBootstrapHandler {
    const METHOD: &'static str = "as2org.bootstrap";
    const IS_STREAMING: bool = false;

    type Params = As2orgBootstrapParams;

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // NOTE: `MonocleDatabase`/`As2orgLens<'_>` are not `Send`. This `handle()` must return a
        // `Send` future, so we must not hold a DB-backed lens across an `.await`.
        //
        // We do all DB work synchronously first, then await only for sending the response.

        // Open the database
        let db = MonocleDatabase::open_in_dir(&ctx.data_dir)
            .map_err(|e| WsError::operation_failed(format!("Failed to open database: {}", e)))?;

        let force = params.force.unwrap_or(false);

        // Perform DB work without any `.await`
        let response = {
            let lens = As2orgLens::new(&db);

            // Check if bootstrap is needed
            if lens.is_data_available() && !force {
                As2orgBootstrapResponse {
                    bootstrapped: false,
                    count: 0,
                    message: "AS2Org data already exists. Use force=true to re-bootstrap."
                        .to_string(),
                }
            } else {
                // Perform bootstrap
                let count = lens
                    .bootstrap()
                    .map_err(|e| WsError::operation_failed(format!("Bootstrap failed: {}", e)))?;

                As2orgBootstrapResponse {
                    bootstrapped: true,
                    count,
                    message: format!("Successfully loaded {} AS2Org entries", count),
                }
            }
        };

        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_as2org_search_params_default() {
        let params = As2orgSearchParams::default();
        assert!(params.query.is_empty());
        assert!(params.asn_only.is_none());
        assert!(params.name_only.is_none());
        assert!(params.country_only.is_none());
    }

    #[test]
    fn test_as2org_search_params_deserialization() {
        let json = r#"{"query": ["13335", "cloudflare"], "full_country": true}"#;
        let params: As2orgSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query.len(), 2);
        assert_eq!(params.query[0], "13335");
        assert_eq!(params.query[1], "cloudflare");
        assert_eq!(params.full_country, Some(true));
    }

    #[test]
    fn test_as2org_search_params_validation() {
        // Empty query should fail
        let params = As2orgSearchParams::default();
        assert!(As2orgSearchHandler::validate(&params).is_err());

        // Valid query should pass
        let params = As2orgSearchParams {
            query: vec!["13335".to_string()],
            ..Default::default()
        };
        assert!(As2orgSearchHandler::validate(&params).is_ok());

        // Multiple exclusive options should fail
        let params = As2orgSearchParams {
            query: vec!["test".to_string()],
            asn_only: Some(true),
            name_only: Some(true),
            ..Default::default()
        };
        assert!(As2orgSearchHandler::validate(&params).is_err());
    }

    #[test]
    fn test_as2org_bootstrap_params_default() {
        let params = As2orgBootstrapParams::default();
        assert!(params.force.is_none());
    }

    #[test]
    fn test_as2org_bootstrap_params_deserialization() {
        let json = r#"{"force": true}"#;
        let params: As2orgBootstrapParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.force, Some(true));
    }

    #[test]
    fn test_as2org_search_response_serialization() {
        let response = As2orgSearchResponse {
            results: vec![As2orgSearchResult {
                asn: 13335,
                as_name: "CLOUDFLARENET".to_string(),
                org_name: "Cloudflare, Inc.".to_string(),
                org_id: "CLOUD14".to_string(),
                org_country: "US".to_string(),
                org_size: 10,
            }],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("13335"));
        assert!(json.contains("CLOUDFLARENET"));
        assert!(json.contains("Cloudflare, Inc."));
    }

    #[test]
    fn test_as2org_bootstrap_response_serialization() {
        let response = As2orgBootstrapResponse {
            bootstrapped: true,
            count: 100,
            message: "Successfully loaded 100 entries".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"bootstrapped\":true"));
        assert!(json.contains("\"count\":100"));
    }
}
