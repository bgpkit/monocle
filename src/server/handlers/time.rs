//! Time handlers for time parsing and formatting
//!
//! This module provides handlers for time-related methods like `time.parse`.

use crate::lens::time::{TimeBgpTime, TimeLens, TimeParseArgs};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// time.parse
// =============================================================================

/// Parameters for time.parse
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TimeParseParams {
    /// Time strings to parse (Unix timestamp, RFC3339, or human-readable)
    /// If empty, uses current time
    #[serde(default)]
    pub times: Vec<String>,

    /// Output format (table, rfc3339, unix, json)
    #[serde(default)]
    pub format: Option<String>,
}

/// Response for time.parse
#[derive(Debug, Clone, Serialize)]
pub struct TimeParseResponse {
    /// Parsed time results
    pub results: Vec<TimeBgpTime>,
}

/// Handler for time.parse method
pub struct TimeParseHandler;

#[async_trait]
impl WsMethod for TimeParseHandler {
    const METHOD: &'static str = "time.parse";
    const IS_STREAMING: bool = false;

    type Params = TimeParseParams;

    async fn handle(
        _ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // Create the time lens
        let lens = TimeLens::new();

        // Create args from params
        let args = TimeParseArgs::new(params.times);

        // Parse the times
        let results = lens
            .parse(&args)
            .map_err(|e| WsError::operation_failed(e.to_string()))?;

        // Send response
        let response = TimeParseResponse { results };
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
    fn test_time_parse_params_default() {
        let params = TimeParseParams::default();
        assert!(params.times.is_empty());
        assert!(params.format.is_none());
    }

    #[test]
    fn test_time_parse_params_deserialization() {
        let json = r#"{"times": ["1697043600", "2023-10-11T00:00:00Z"]}"#;
        let params: TimeParseParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.times.len(), 2);
        assert_eq!(params.times[0], "1697043600");
        assert_eq!(params.times[1], "2023-10-11T00:00:00Z");
    }

    #[test]
    fn test_time_parse_params_empty() {
        let json = r#"{}"#;
        let params: TimeParseParams = serde_json::from_str(json).unwrap();
        assert!(params.times.is_empty());
    }

    #[test]
    fn test_time_parse_response_serialization() {
        let response = TimeParseResponse {
            results: vec![TimeBgpTime {
                unix: 1697043600,
                rfc3339: "2023-10-11T15:00:00+00:00".to_string(),
                human: "about 1 year ago".to_string(),
            }],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("1697043600"));
        assert!(json.contains("2023-10-11T15:00:00+00:00"));
    }

    #[test]
    fn test_time_lens_parse() {
        let lens = TimeLens::new();
        let args = TimeParseArgs::new(vec!["1697043600".to_string()]);
        let results = lens.parse(&args).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].unix, 1697043600);
    }
}
