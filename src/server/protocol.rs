//! Protocol types for the WebSocket API
//!
//! This module defines the core protocol types including request/response envelopes,
//! error codes, and progress stages.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// =============================================================================
// Request Types
// =============================================================================

/// Request envelope sent by clients
#[derive(Debug, Clone, Deserialize)]
pub struct RequestEnvelope {
    /// Optional request correlation ID (client may omit; server generates and echoes)
    #[serde(default)]
    pub id: Option<String>,

    /// Operation to perform (e.g., "rpki.validate")
    pub method: String,

    /// Operation-specific parameters
    #[serde(default)]
    pub params: Value,
}

// =============================================================================
// Response Types
// =============================================================================

/// Response envelope sent by the server
#[derive(Debug, Clone, Serialize)]
pub struct ResponseEnvelope {
    /// Request correlation ID (client-provided or server-generated)
    pub id: String,

    /// Server-generated operation identifier (present for streaming/long operations)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op_id: Option<String>,

    /// Response type
    #[serde(rename = "type")]
    pub response_type: ResponseType,

    /// Response payload
    pub data: Value,
}

impl ResponseEnvelope {
    /// Create a result response
    pub fn result(id: String, data: impl Serialize) -> Self {
        Self {
            id,
            op_id: None,
            response_type: ResponseType::Result,
            data: serde_json::to_value(data).unwrap_or(Value::Null),
        }
    }

    /// Create a result response with op_id
    pub fn result_with_op(id: String, op_id: String, data: impl Serialize) -> Self {
        Self {
            id,
            op_id: Some(op_id),
            response_type: ResponseType::Result,
            data: serde_json::to_value(data).unwrap_or(Value::Null),
        }
    }

    /// Create a progress response
    pub fn progress(id: String, op_id: String, data: impl Serialize) -> Self {
        Self {
            id,
            op_id: Some(op_id),
            response_type: ResponseType::Progress,
            data: serde_json::to_value(data).unwrap_or(Value::Null),
        }
    }

    /// Create a stream response
    pub fn stream(id: String, op_id: String, data: impl Serialize) -> Self {
        Self {
            id,
            op_id: Some(op_id),
            response_type: ResponseType::Stream,
            data: serde_json::to_value(data).unwrap_or(Value::Null),
        }
    }

    /// Create an error response
    pub fn error(id: String, op_id: Option<String>, error: ErrorData) -> Self {
        Self {
            id,
            op_id,
            response_type: ResponseType::Error,
            data: serde_json::to_value(error).unwrap_or(Value::Null),
        }
    }
}

/// Response type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseType {
    /// Final successful response for the operation (exactly once)
    Result,
    /// Intermediate progress update (0..N times)
    Progress,
    /// Streaming data batches (0..N times)
    Stream,
    /// Error response (terminal; ends the operation)
    Error,
}

// =============================================================================
// Error Types
// =============================================================================

/// Error data structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorData {
    /// Error code
    pub code: ErrorCode,

    /// Human-readable error message
    pub message: String,

    /// Optional additional details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ErrorData {
    /// Create a new error
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    /// Create an error with details
    pub fn with_details(code: ErrorCode, message: impl Into<String>, details: Value) -> Self {
        Self {
            code,
            message: message.into(),
            details: Some(details),
        }
    }

    /// Create an invalid request error
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    /// Create an unknown method error
    pub fn unknown_method(method: &str) -> Self {
        Self::new(
            ErrorCode::UnknownMethod,
            format!("Unknown method: {}", method),
        )
    }

    /// Create an invalid params error
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParams, message)
    }

    /// Create an operation failed error
    pub fn operation_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::OperationFailed, message)
    }

    /// Create an operation cancelled error
    pub fn operation_cancelled() -> Self {
        Self::new(ErrorCode::OperationCancelled, "Operation was cancelled")
    }

    /// Create a not initialized error
    pub fn not_initialized(resource: &str) -> Self {
        Self::new(
            ErrorCode::NotInitialized,
            format!(
                "{} data not initialized. Run bootstrap/refresh first.",
                resource
            ),
        )
    }

    /// Create a rate limited error
    pub fn rate_limited() -> Self {
        Self::new(ErrorCode::RateLimited, "Too many concurrent operations")
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }
}

/// Error codes as specified in the design document
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Malformed request message
    InvalidRequest,
    /// Method not found
    UnknownMethod,
    /// Invalid or missing parameters
    InvalidParams,
    /// Operation failed during execution
    OperationFailed,
    /// Operation was cancelled by client
    OperationCancelled,
    /// Required data not initialized/bootstrapped
    NotInitialized,
    /// Too many concurrent operations
    RateLimited,
    /// Unexpected server error
    InternalError,
}

// =============================================================================
// Progress Types
// =============================================================================

/// Shared progress stages vocabulary
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgressStage {
    /// Operation is queued
    Queued,
    /// Operation is running
    Running,
    /// Downloading data
    Downloading,
    /// Processing data
    Processing,
    /// Finalizing results
    Finalizing,
    /// Operation completed
    Done,
}

// =============================================================================
// System Info Types
// =============================================================================

/// System information response
#[derive(Debug, Clone, Serialize)]
pub struct SystemInfo {
    /// Protocol version
    pub protocol_version: u32,

    /// Server version
    pub server_version: String,

    /// Build information
    pub build: BuildInfo,

    /// Feature flags
    pub features: FeatureFlags,
}

/// Build information
#[derive(Debug, Clone, Serialize)]
pub struct BuildInfo {
    /// Git commit SHA
    pub git_sha: String,

    /// Build timestamp
    pub timestamp: String,
}

/// Feature flags
#[derive(Debug, Clone, Serialize)]
pub struct FeatureFlags {
    /// Whether streaming is supported
    pub streaming: bool,

    /// Whether authentication is required
    pub auth_required: bool,
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            protocol_version: 1,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            build: BuildInfo {
                git_sha: option_env!("GIT_SHA").unwrap_or("unknown").to_string(),
                timestamp: option_env!("BUILD_TIMESTAMP")
                    .unwrap_or("unknown")
                    .to_string(),
            },
            features: FeatureFlags {
                streaming: true,
                auth_required: false,
            },
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_envelope_deserialization() {
        // With id
        let json =
            r#"{"id": "test-1", "method": "time.parse", "params": {"times": ["1234567890"]}}"#;
        let req: RequestEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(req.id, Some("test-1".to_string()));
        assert_eq!(req.method, "time.parse");

        // Without id
        let json = r#"{"method": "time.parse", "params": {}}"#;
        let req: RequestEnvelope = serde_json::from_str(json).unwrap();
        assert!(req.id.is_none());
        assert_eq!(req.method, "time.parse");

        // Without params
        let json = r#"{"method": "time.parse"}"#;
        let req: RequestEnvelope = serde_json::from_str(json).unwrap();
        assert!(req.params.is_null());
    }

    #[test]
    fn test_response_envelope_serialization() {
        let resp =
            ResponseEnvelope::result("test-1".to_string(), serde_json::json!({"foo": "bar"}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"id\":\"test-1\""));
        assert!(json.contains("\"type\":\"result\""));
        assert!(!json.contains("op_id")); // Should be skipped when None

        let resp = ResponseEnvelope::progress(
            "test-1".to_string(),
            "op-1".to_string(),
            serde_json::json!({"stage": "running"}),
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"op_id\":\"op-1\""));
        assert!(json.contains("\"type\":\"progress\""));
    }

    #[test]
    fn test_error_codes_serialization() {
        let error = ErrorData::new(ErrorCode::InvalidRequest, "test error");
        let json = serde_json::to_string(&error).unwrap();
        assert!(json.contains("\"code\":\"INVALID_REQUEST\""));
    }

    #[test]
    fn test_progress_stage_serialization() {
        let stage = ProgressStage::Running;
        let json = serde_json::to_string(&stage).unwrap();
        assert_eq!(json, "\"running\"");
    }
}
