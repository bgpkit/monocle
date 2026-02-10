//! Inspect handlers for unified AS and prefix information lookup
//!
//! This module provides handlers for inspect-related methods like `inspect.query`.
//!
//! The handler uses the InspectLens for unified queries and sends progress notifications
//! when data sources need to be refreshed.

use crate::database::MonocleDatabase;
use crate::lens::inspect::{
    DataRefreshSummary, InspectDataSection, InspectLens, InspectQueryOptions, InspectResult,
};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

// =============================================================================
// inspect.query
// =============================================================================

/// Parameters for inspect.query
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InspectQueryParams {
    /// One or more queries: ASN, prefix, IP, or name
    pub queries: Vec<String>,

    /// Force query type: "asn", "prefix", or "name" (optional, auto-detect if not specified)
    #[serde(default)]
    pub query_type: Option<String>,

    /// Sections to include (default: varies by query type)
    /// Available: core, peeringdb, hegemony, population, prefixes, connectivity, roas, aspa, all
    #[serde(default)]
    pub select: Option<Vec<String>>,

    /// Maximum ROAs to return (0 = unlimited, default: 10)
    #[serde(default)]
    pub max_roas: Option<usize>,

    /// Maximum prefixes to return (0 = unlimited, default: 10)
    #[serde(default)]
    pub max_prefixes: Option<usize>,

    /// Maximum neighbors per category (0 = unlimited, default: 5)
    #[serde(default)]
    pub max_neighbors: Option<usize>,

    /// Maximum search results (0 = unlimited, default: 20)
    #[serde(default)]
    pub max_search_results: Option<usize>,

    /// Country code filter for country-based search
    #[serde(default)]
    pub country: Option<String>,
}

/// Progress notification for data refresh
#[derive(Debug, Clone, Serialize)]
pub struct InspectDataRefreshProgress {
    /// Stage of the operation
    pub stage: String,
    /// Message describing the current operation
    pub message: String,
    /// Data source being refreshed (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Number of records loaded (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
}

/// Response for inspect.query
#[derive(Debug, Clone, Serialize)]
pub struct InspectQueryResponse {
    /// Whether any data was refreshed before the query
    pub data_refreshed: bool,
    /// Summary of data refreshes (if any occurred)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_summary: Option<DataRefreshSummary>,
    /// Query results
    pub result: InspectResult,
}

/// Handler for inspect.query method
pub struct InspectQueryHandler;

#[async_trait]
impl WsMethod for InspectQueryHandler {
    const METHOD: &'static str = "inspect.query";
    const IS_STREAMING: bool = true; // We send progress notifications for data refresh

    type Params = InspectQueryParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        // Must have at least one query or a country filter
        if params.queries.is_empty() && params.country.is_none() {
            return Err(WsError::invalid_params(
                "At least one query or country filter is required",
            ));
        }

        // Validate query_type if provided
        if let Some(ref qt) = params.query_type {
            match qt.to_lowercase().as_str() {
                "asn" | "prefix" | "name" => {}
                _ => {
                    return Err(WsError::invalid_params(format!(
                        "Invalid query_type: {}. Use 'asn', 'prefix', or 'name'",
                        qt
                    )));
                }
            }
        }

        // Validate select sections if provided
        if let Some(ref sections) = params.select {
            for section in sections {
                let s_lower = section.to_lowercase();
                if s_lower != "all" && InspectDataSection::from_str(&s_lower).is_none() {
                    return Err(WsError::invalid_params(format!(
                        "Invalid section: {}. Available: {}",
                        section,
                        InspectDataSection::all_names().join(", ")
                    )));
                }
            }
        }

        Ok(())
    }

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // Build query options first (no DB needed)
        let options = build_query_options(&params);

        // Do all DB work in a block before any awaits to avoid Send issues
        let (refresh_summary, result): (DataRefreshSummary, InspectResult) = {
            let db = MonocleDatabase::open_in_dir(ctx.data_dir())
                .map_err(|e| WsError::internal(format!("Failed to open database: {}", e)))?;

            let lens = InspectLens::new(&db, &ctx.config);

            // Ensure data is available, refreshing if needed
            let refresh_summary = lens
                .ensure_data_available()
                .map_err(|e| WsError::operation_failed(format!("Failed to ensure data: {}", e)))?;

            // Execute query
            let result = if let Some(ref country) = params.country {
                lens.query_by_country(country, &options)
                    .map_err(|e| WsError::operation_failed(e.to_string()))?
            } else if let Some(ref query_type) = params.query_type {
                match query_type.to_lowercase().as_str() {
                    "asn" => lens
                        .query_as_asn(&params.queries, &options)
                        .map_err(|e| WsError::operation_failed(e.to_string()))?,
                    "prefix" => lens
                        .query_as_prefix(&params.queries, &options)
                        .map_err(|e| WsError::operation_failed(e.to_string()))?,
                    "name" => lens
                        .query_as_name(&params.queries, &options)
                        .map_err(|e| WsError::operation_failed(e.to_string()))?,
                    _ => {
                        return Err(WsError::invalid_params(format!(
                            "Invalid query_type: {}",
                            query_type
                        )));
                    }
                }
            } else {
                // Auto-detect query types
                lens.query(&params.queries, &options)
                    .map_err(|e| WsError::operation_failed(e.to_string()))?
            };

            (refresh_summary, result)
        };

        // Now we can safely do awaits - send progress notifications for any refreshes
        if refresh_summary.any_refreshed {
            for refresh in &refresh_summary.sources {
                if refresh.refreshed {
                    sink.send_progress(InspectDataRefreshProgress {
                        stage: "refreshed".to_string(),
                        message: refresh.message.clone(),
                        source: Some(refresh.source.clone()),
                        count: refresh.count,
                    })
                    .await
                    .map_err(|e| WsError::internal(e.to_string()))?;
                }
            }
        }

        // Build response
        let response = InspectQueryResponse {
            data_refreshed: refresh_summary.any_refreshed,
            refresh_summary: if refresh_summary.any_refreshed {
                Some(refresh_summary)
            } else {
                None
            },
            result,
        };

        // Send final result
        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

/// Build query options from parameters
fn build_query_options(params: &InspectQueryParams) -> InspectQueryOptions {
    let mut options = InspectQueryOptions::default();

    // Parse select sections
    if let Some(ref sections) = params.select {
        let mut selected = HashSet::new();

        for s in sections {
            let s_lower = s.to_lowercase();
            if s_lower == "all" {
                selected.extend(InspectDataSection::all());
            } else if let Some(section) = InspectDataSection::from_str(&s_lower) {
                selected.insert(section);
            }
        }

        if !selected.is_empty() {
            options.select = Some(selected);
        }
    }

    // Apply limits
    if let Some(max_roas) = params.max_roas {
        options.max_roas = max_roas;
    }

    if let Some(max_prefixes) = params.max_prefixes {
        options.max_prefixes = max_prefixes;
    }

    if let Some(max_neighbors) = params.max_neighbors {
        options.max_neighbors = max_neighbors;
    }

    if let Some(max_search_results) = params.max_search_results {
        options.max_search_results = max_search_results;
    }

    options
}

// =============================================================================
// inspect.refresh
// =============================================================================

/// Parameters for inspect.refresh
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct InspectRefreshParams {
    /// Force refresh even if data is not stale
    #[serde(default)]
    pub force: bool,
}

/// Response for inspect.refresh
#[derive(Debug, Clone, Serialize)]
pub struct InspectRefreshResponse {
    /// Summary of refresh operations
    pub summary: DataRefreshSummary,
}

/// Handler for inspect.refresh method
pub struct InspectRefreshHandler;

#[async_trait]
impl WsMethod for InspectRefreshHandler {
    const METHOD: &'static str = "inspect.refresh";
    const IS_STREAMING: bool = true; // We send progress notifications

    type Params = InspectRefreshParams;

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        _params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // Do all DB work in a block before any awaits
        let summary: DataRefreshSummary = {
            let db = MonocleDatabase::open_in_dir(ctx.data_dir())
                .map_err(|e| WsError::internal(format!("Failed to open database: {}", e)))?;

            let lens = InspectLens::new(&db, &ctx.config);

            // Perform refresh
            lens.ensure_data_available()
                .map_err(|e| WsError::operation_failed(format!("Failed to refresh data: {}", e)))?
        };

        // Now we can safely do awaits - send progress for each refresh
        for refresh in &summary.sources {
            sink.send_progress(InspectDataRefreshProgress {
                stage: if refresh.refreshed {
                    "refreshed"
                } else {
                    "skipped"
                }
                .to_string(),
                message: refresh.message.clone(),
                source: Some(refresh.source.clone()),
                count: refresh.count,
            })
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;
        }

        // Send final result
        let response = InspectRefreshResponse { summary };

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
    fn test_inspect_query_params_default() {
        let params: InspectQueryParams = serde_json::from_str(r#"{"queries": ["13335"]}"#).unwrap();
        assert_eq!(params.queries, vec!["13335"]);
        assert!(params.query_type.is_none());
        assert!(params.select.is_none());
        assert!(params.max_roas.is_none());
    }

    #[test]
    fn test_inspect_query_params_full() {
        let params: InspectQueryParams = serde_json::from_str(
            r#"{
                "queries": ["13335", "1.1.1.0/24"],
                "query_type": "asn",
                "select": ["core", "connectivity"],
                "max_roas": 5,
                "max_neighbors": 10
            }"#,
        )
        .unwrap();

        assert_eq!(params.queries.len(), 2);
        assert_eq!(params.query_type, Some("asn".to_string()));
        assert_eq!(
            params.select,
            Some(vec!["core".to_string(), "connectivity".to_string()])
        );
        assert_eq!(params.max_roas, Some(5));
        assert_eq!(params.max_neighbors, Some(10));
    }

    #[test]
    fn test_inspect_query_validation_empty_queries() {
        let params = InspectQueryParams {
            queries: vec![],
            query_type: None,
            select: None,
            max_roas: None,
            max_prefixes: None,
            max_neighbors: None,
            max_search_results: None,
            country: None,
        };

        let result = InspectQueryHandler::validate(&params);
        assert!(result.is_err());
    }

    #[test]
    fn test_inspect_query_validation_with_country() {
        let params = InspectQueryParams {
            queries: vec![],
            query_type: None,
            select: None,
            max_roas: None,
            max_prefixes: None,
            max_neighbors: None,
            max_search_results: None,
            country: Some("US".to_string()),
        };

        let result = InspectQueryHandler::validate(&params);
        assert!(result.is_ok());
    }

    #[test]
    fn test_inspect_query_validation_invalid_query_type() {
        let params = InspectQueryParams {
            queries: vec!["13335".to_string()],
            query_type: Some("invalid".to_string()),
            select: None,
            max_roas: None,
            max_prefixes: None,
            max_neighbors: None,
            max_search_results: None,
            country: None,
        };

        let result = InspectQueryHandler::validate(&params);
        assert!(result.is_err());
    }

    #[test]
    fn test_inspect_query_validation_invalid_section() {
        let params = InspectQueryParams {
            queries: vec!["13335".to_string()],
            query_type: None,
            select: Some(vec!["invalid_section".to_string()]),
            max_roas: None,
            max_prefixes: None,
            max_neighbors: None,
            max_search_results: None,
            country: None,
        };

        let result = InspectQueryHandler::validate(&params);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_query_options_defaults() {
        let params = InspectQueryParams {
            queries: vec!["13335".to_string()],
            query_type: None,
            select: None,
            max_roas: None,
            max_prefixes: None,
            max_neighbors: None,
            max_search_results: None,
            country: None,
        };

        let options = build_query_options(&params);
        assert!(options.select.is_none());
        assert_eq!(options.max_roas, 10); // default
        assert_eq!(options.max_prefixes, 10); // default
        assert_eq!(options.max_neighbors, 5); // default
        assert_eq!(options.max_search_results, 20); // default
    }

    #[test]
    fn test_build_query_options_with_select() {
        let params = InspectQueryParams {
            queries: vec!["13335".to_string()],
            query_type: None,
            select: Some(vec!["basic".to_string(), "rpki".to_string()]),
            max_roas: Some(100),
            max_prefixes: None,
            max_neighbors: None,
            max_search_results: None,
            country: None,
        };

        let options = build_query_options(&params);
        assert!(options.select.is_some());
        let select = options.select.unwrap();
        assert!(select.contains(&InspectDataSection::Basic));
        assert!(select.contains(&InspectDataSection::Rpki));
        assert_eq!(options.max_roas, 100);
    }

    #[test]
    fn test_inspect_refresh_params_default() {
        let params: InspectRefreshParams = serde_json::from_str(r#"{}"#).unwrap();
        assert!(!params.force);
    }

    #[test]
    fn test_data_refresh_progress_serialization() {
        let progress = InspectDataRefreshProgress {
            stage: "refreshing".to_string(),
            message: "Refreshing RPKI data...".to_string(),
            source: Some("rpki".to_string()),
            count: Some(1000),
        };

        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"stage\":\"refreshing\""));
        assert!(json.contains("\"source\":\"rpki\""));
        assert!(json.contains("\"count\":1000"));
    }
}
