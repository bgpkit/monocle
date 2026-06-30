//! HTTP API error types and REST routing for Monocle's HTTP service.
//!
//! This module defines the API error response format, the Axum router for
//! MVP endpoints (`/health`, `/api/v1/system/info`), and shared types used
//! by the search stream handler in [`crate::server::search`].

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router as AxumRouter;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::ServerState;

// =============================================================================
// API Error Types
// =============================================================================

/// API error response body, returned as JSON for pre-stream errors and as
/// the `data` field of an SSE `error` event for in-stream errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    pub code: ApiErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ApiErrorResponse {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(code: ApiErrorCode, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details),
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::InvalidParams, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::InvalidRequest, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::InternalError, message)
    }
}

/// API error codes, serialized as `SCREAMING_SNAKE_CASE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    /// Malformed request body or structure
    InvalidRequest,
    /// Invalid or missing parameter values
    InvalidParams,
    /// Search was cancelled (client disconnect or timeout)
    Cancelled,
    /// Search failed during execution
    SearchFailed,
    /// Required local data not initialized
    NotInitialized,
    /// Unexpected server error
    InternalError,
}

/// Wrapper type so handlers can return `Result<T, ApiError>` and Axum converts
/// it into an appropriate HTTP response.
#[derive(Debug)]
pub struct ApiError(pub (StatusCode, ApiErrorResponse));

impl ApiError {
    pub fn new(status: StatusCode, body: ApiErrorResponse) -> Self {
        Self((status, body))
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ApiErrorResponse::invalid_params(message),
        )
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            ApiErrorResponse::invalid_request(message),
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorResponse::internal(message),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = self.0;
        (status, Json(body)).into_response()
    }
}

// =============================================================================
// System Info
// =============================================================================

/// System information response for `GET /api/v1/system/info`.
#[derive(Debug, Clone, Serialize)]
pub struct SystemInfoResponse {
    pub server_version: String,
    pub api_version: &'static str,
    pub endpoints: Vec<&'static str>,
}

impl Default for SystemInfoResponse {
    fn default() -> Self {
        Self {
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            api_version: "v1",
            endpoints: vec![
                "/health",
                "/api/v1/system/info",
                "/api/v1/search/stream",
                "/api/v1/time/parse",
                "/api/v1/country/lookup",
                "/api/v1/ip/lookup",
                "/api/v1/ip/public",
                "/api/v1/rpki/roa/lookup",
                "/api/v1/rpki/aspa/lookup",
                "/api/v1/rpki/roa/validate",
                "/api/v1/pfx2as/lookup",
                "/api/v1/as2rel/search",
                "/api/v1/as2rel/relationship",
                "/api/v1/as2rel/refresh",
                "/api/v1/inspect/query",
                "/api/v1/inspect/refresh",
                "/api/v1/database/status",
                "/api/v1/database/refresh",
            ],
        }
    }
}

// =============================================================================
// Router
// =============================================================================

/// Build the Axum router for all REST endpoints under `/api/v1`.
///
/// Takes `ServerState` by value; Axum's `with_state` consumes it.
pub fn router(state: ServerState) -> AxumRouter {
    use crate::server::rest;

    AxumRouter::new()
        // System
        .route("/system/info", get(system_info))
        // Search (SSE)
        .route("/search/stream", post(crate::server::search::stream_search))
        // Tier 1: Stateless
        .route("/time/parse", post(rest::time::time_parse))
        .route("/country/lookup", post(rest::country::country_lookup))
        .route("/ip/lookup", post(rest::ip::ip_lookup))
        .route("/ip/public", get(rest::ip::ip_public))
        // Tier 2: Database read-only
        .route("/database/status", get(rest::database::database_status))
        .route("/rpki/roa/lookup", get(rest::rpki::roa_lookup))
        .route("/rpki/aspa/lookup", get(rest::rpki::aspa_lookup))
        .route("/pfx2as/lookup", get(rest::pfx2as::pfx2as_lookup))
        .route(
            "/as2rel/relationship",
            get(rest::as2rel::as2rel_relationship),
        )
        .route("/as2rel/search", post(rest::as2rel::as2rel_search))
        // Tier 3: Database refresh
        .route("/database/refresh", post(rest::database::database_refresh))
        .route("/inspect/refresh", post(rest::database::inspect_refresh))
        .route("/as2rel/refresh", post(rest::as2rel::as2rel_refresh))
        // Tier 4: Composite query
        .route("/rpki/roa/validate", post(rest::rpki::roa_validate))
        .route("/inspect/query", post(rest::inspect::inspect_query))
        .with_state(state)
}

/// `GET /api/v1/system/info`
async fn system_info(State(_state): State<ServerState>) -> Json<SystemInfoResponse> {
    Json(SystemInfoResponse::default())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_response_serialization() {
        let err = ApiErrorResponse::invalid_params("missing prefix");
        let json = serde_json::to_string(&err).unwrap_or_default();
        assert!(json.contains("\"code\":\"INVALID_PARAMS\""));
        assert!(json.contains("\"message\":\"missing prefix\""));
        assert!(!json.contains("details")); // skipped when None
    }

    #[test]
    fn test_api_error_code_serialization() {
        assert_eq!(
            serde_json::to_string(&ApiErrorCode::InvalidRequest).unwrap_or_default(),
            "\"INVALID_REQUEST\""
        );
        assert_eq!(
            serde_json::to_string(&ApiErrorCode::SearchFailed).unwrap_or_default(),
            "\"SEARCH_FAILED\""
        );
        assert_eq!(
            serde_json::to_string(&ApiErrorCode::Cancelled).unwrap_or_default(),
            "\"CANCELLED\""
        );
    }

    #[test]
    fn test_system_info_default() {
        let info = SystemInfoResponse::default();
        assert_eq!(info.api_version, "v1");
        assert!(info.endpoints.contains(&"/api/v1/search/stream"));
    }
}
