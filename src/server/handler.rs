//! Handler trait and context module for WebSocket methods
//!
//! This module defines the `WsMethod` trait which all WebSocket method handlers
//! must implement, along with the `WsContext` which provides access to shared
//! resources like database handles and configuration.

use crate::server::op_sink::WsOpSink;
use crate::server::protocol::{ErrorCode, ErrorData, RequestEnvelope};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::sync::Arc;

// =============================================================================
// Context
// =============================================================================

use crate::config::MonocleConfig;

/// WebSocket context providing access to shared resources
///
/// This context is passed to all handlers and provides access to:
/// - Database handles (MonocleDatabase)
/// - Configuration settings
/// - Operation registry for cancellation
/// - Rate limiting state
#[derive(Clone)]
pub struct WsContext {
    /// Monocle configuration (includes data_dir and cache TTLs)
    pub config: MonocleConfig,
}

impl WsContext {
    /// Create a new WebSocket context from MonocleConfig
    pub fn from_config(config: MonocleConfig) -> Self {
        Self { config }
    }

    /// Get the data directory path
    pub fn data_dir(&self) -> &str {
        &self.config.data_dir
    }
}

impl Default for WsContext {
    fn default() -> Self {
        Self::from_config(MonocleConfig::default())
    }
}

// =============================================================================
// Request
// =============================================================================

/// Processed WebSocket request with guaranteed ID
#[derive(Debug, Clone)]
pub struct WsRequest {
    /// Request correlation ID (client-provided or server-generated)
    pub id: String,

    /// Server-generated operation identifier (present for streaming/long operations)
    pub op_id: Option<String>,

    /// Method name
    pub method: String,

    /// Raw parameters
    pub params: Value,
}

impl WsRequest {
    /// Create a new request from an envelope, generating an ID if not provided.
    ///
    /// Note: `op_id` is assigned by the dispatcher/router for streaming/long operations.
    pub fn from_envelope(envelope: RequestEnvelope) -> Self {
        let id = envelope
            .id
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Self {
            id,
            op_id: None,
            method: envelope.method,
            params: envelope.params,
        }
    }
}

// =============================================================================
// Handler Trait
// =============================================================================

/// Result type for WebSocket handlers
pub type WsResult<T> = Result<T, WsError>;

/// Error type for WebSocket handlers
#[derive(Debug, Clone)]
pub struct WsError {
    /// Error code
    pub code: ErrorCode,
    /// Error message
    pub message: String,
    /// Optional details
    pub details: Option<Value>,
}

impl WsError {
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

    /// Create an invalid params error
    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidParams, message)
    }

    /// Create an operation failed error
    pub fn operation_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::OperationFailed, message)
    }

    /// Create a not initialized error
    pub fn not_initialized(resource: &str) -> Self {
        Self::new(
            ErrorCode::NotInitialized,
            format!("{} data not initialized", resource),
        )
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }

    /// Convert to ErrorData
    pub fn to_error_data(&self) -> ErrorData {
        match &self.details {
            Some(details) => {
                ErrorData::with_details(self.code, self.message.clone(), details.clone())
            }
            None => ErrorData::new(self.code, self.message.clone()),
        }
    }
}

impl std::fmt::Display for WsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for WsError {}

impl From<anyhow::Error> for WsError {
    fn from(err: anyhow::Error) -> Self {
        Self::operation_failed(err.to_string())
    }
}

impl From<serde_json::Error> for WsError {
    fn from(err: serde_json::Error) -> Self {
        Self::invalid_params(err.to_string())
    }
}

/// Trait for WebSocket method handlers
///
/// Each method handler implements this trait to define:
/// - The method name (e.g., "rpki.validate")
/// - Whether it's a streaming method
/// - How to parse and validate parameters
/// - How to execute the method
#[async_trait]
pub trait WsMethod: Send + Sync + 'static {
    /// Fully qualified method name, e.g., "rpki.validate"
    const METHOD: &'static str;

    /// Whether this method is streaming (returns progress/stream messages)
    const IS_STREAMING: bool = false;

    /// Parameter type for this method
    type Params: DeserializeOwned + Send;

    /// Validate parameters after parsing
    ///
    /// Override this to perform additional validation beyond JSON deserialization.
    fn validate(_params: &Self::Params) -> WsResult<()> {
        Ok(())
    }

    /// Execute the method
    ///
    /// For non-streaming methods, this should send a single result via the sink.
    /// For streaming methods, this may send progress/stream messages followed by a result.
    async fn handle(
        ctx: Arc<WsContext>,
        req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()>;
}

// =============================================================================
// Handler Registration
// =============================================================================

/// Type-erased handler function
pub type DynHandler = Box<
    dyn Fn(Arc<WsContext>, WsRequest, WsOpSink) -> futures::future::BoxFuture<'static, WsResult<()>>
        + Send
        + Sync,
>;

/// Create a type-erased handler from a WsMethod implementation
pub fn make_handler<M: WsMethod>() -> DynHandler {
    Box::new(move |ctx, req, sink| {
        Box::pin(async move {
            // Parse parameters
            let params: M::Params = serde_json::from_value(req.params.clone())?;

            // Validate parameters
            M::validate(&params)?;

            // Execute handler
            M::handle(ctx, req, params, sink).await
        })
    })
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_context_default() {
        let ctx = WsContext::default();
        assert!(ctx.data_dir().contains("monocle"));
    }

    #[test]
    fn test_ws_context_from_config() {
        let config = MonocleConfig::default();
        let ctx = WsContext::from_config(config.clone());
        assert_eq!(ctx.data_dir(), &config.data_dir);
    }

    #[test]
    fn test_ws_request_from_envelope() {
        // With ID
        let envelope = RequestEnvelope {
            id: Some("test-id".to_string()),
            method: "time.parse".to_string(),
            params: serde_json::json!({}),
        };
        let req = WsRequest::from_envelope(envelope);
        assert_eq!(req.id, "test-id");
        assert_eq!(req.op_id, None);
        assert_eq!(req.method, "time.parse");

        // Without ID (should generate UUID)
        let envelope = RequestEnvelope {
            id: None,
            method: "time.parse".to_string(),
            params: serde_json::json!({}),
        };
        let req = WsRequest::from_envelope(envelope);
        assert!(!req.id.is_empty());
        assert_ne!(req.id, "test-id"); // Should be different
    }

    #[test]
    fn test_ws_error_conversion() {
        let err = WsError::invalid_params("missing field");
        assert_eq!(err.code, ErrorCode::InvalidParams);
        assert!(err.message.contains("missing field"));

        let error_data = err.to_error_data();
        assert_eq!(error_data.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn test_ws_error_from_anyhow() {
        let anyhow_err = anyhow::anyhow!("something went wrong");
        let ws_err: WsError = anyhow_err.into();
        assert_eq!(ws_err.code, ErrorCode::OperationFailed);
        assert!(ws_err.message.contains("something went wrong"));
    }
}
