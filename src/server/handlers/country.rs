//! Country handlers for country lookup operations
//!
//! This module provides handlers for country-related methods like `country.lookup`.

use crate::lens::country::{CountryEntry, CountryLens, CountryLookupArgs};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// country.lookup
// =============================================================================

/// Parameters for country.lookup
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct CountryLookupParams {
    /// Search query: country code (e.g., "US") or partial name (e.g., "united")
    #[serde(default)]
    pub query: Option<String>,

    /// List all countries
    #[serde(default)]
    pub all: Option<bool>,
}

/// Response for country.lookup
#[derive(Debug, Clone, Serialize)]
pub struct CountryLookupResponse {
    /// Matching countries
    pub countries: Vec<CountryEntry>,
}

/// Handler for country.lookup method
pub struct CountryLookupHandler;

#[async_trait]
impl WsMethod for CountryLookupHandler {
    const METHOD: &'static str = "country.lookup";
    const IS_STREAMING: bool = false;

    type Params = CountryLookupParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        // Must have either a query or all=true
        if params.query.is_none() && !params.all.unwrap_or(false) {
            return Err(WsError::invalid_params(
                "Either 'query' or 'all: true' is required",
            ));
        }
        Ok(())
    }

    async fn handle(
        _ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // Create the country lens
        let lens = CountryLens::new();

        // Create args from params
        let args = if params.all.unwrap_or(false) {
            CountryLookupArgs::all_countries()
        } else {
            CountryLookupArgs::new(params.query.unwrap_or_default())
        };

        // Perform the search
        let countries = lens
            .search(&args)
            .map_err(|e| WsError::operation_failed(e.to_string()))?;

        // Send response
        let response = CountryLookupResponse { countries };
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
    fn test_country_lookup_params_default() {
        let params = CountryLookupParams::default();
        assert!(params.query.is_none());
        assert!(params.all.is_none());
    }

    #[test]
    fn test_country_lookup_params_deserialization() {
        let json = r#"{"query": "US"}"#;
        let params: CountryLookupParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.query, Some("US".to_string()));
        assert!(params.all.is_none());

        let json = r#"{"all": true}"#;
        let params: CountryLookupParams = serde_json::from_str(json).unwrap();
        assert!(params.query.is_none());
        assert_eq!(params.all, Some(true));
    }

    #[test]
    fn test_country_lookup_params_validation() {
        // Empty params should fail
        let params = CountryLookupParams::default();
        assert!(CountryLookupHandler::validate(&params).is_err());

        // With query should pass
        let params = CountryLookupParams {
            query: Some("US".to_string()),
            all: None,
        };
        assert!(CountryLookupHandler::validate(&params).is_ok());

        // With all=true should pass
        let params = CountryLookupParams {
            query: None,
            all: Some(true),
        };
        assert!(CountryLookupHandler::validate(&params).is_ok());
    }

    #[test]
    fn test_country_lookup_response_serialization() {
        let response = CountryLookupResponse {
            countries: vec![CountryEntry {
                code: "US".to_string(),
                name: "United States".to_string(),
            }],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"US\""));
        assert!(json.contains("United States"));
    }

    #[test]
    fn test_country_lens_lookup() {
        let lens = CountryLens::new();
        let results = lens.lookup("US");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].code, "US");
    }
}
