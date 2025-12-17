//! Router module for registry-based method dispatch
//!
//! This module provides the `Router` which maintains a registry of method handlers
//! and dispatches incoming requests to the appropriate handler.
//!
//! Protocol invariants enforced here:
//! - Streaming methods: server generates `op_id` and it MUST be present in all
//!   progress/stream/terminal responses.
//! - Non-streaming methods: `op_id` MUST be absent in all responses.

use crate::server::handler::{make_handler, DynHandler, WsContext, WsMethod, WsRequest};
use crate::server::op_sink::WsOpSink;
use crate::server::operations::OperationRegistry;
use crate::server::protocol::{ErrorData, RequestEnvelope};
use crate::server::sink::WsSink;
use std::collections::HashMap;
use std::sync::Arc;

// =============================================================================
// Router
// =============================================================================

/// Router for dispatching WebSocket requests to handlers
///
/// The router maintains a registry of method handlers and dispatches incoming
/// requests to the appropriate handler based on the method name.
pub struct Router {
    /// Map from method name to handler
    handlers: HashMap<&'static str, DynHandler>,

    /// Whether the handler is streaming
    streaming_methods: HashMap<&'static str, bool>,
}

impl Router {
    /// Create a new empty router
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            streaming_methods: HashMap::new(),
        }
    }

    /// Register a method handler
    pub fn register<M: WsMethod>(&mut self) -> &mut Self {
        let handler = make_handler::<M>();
        self.handlers.insert(M::METHOD, handler);
        self.streaming_methods.insert(M::METHOD, M::IS_STREAMING);
        self
    }

    /// Check if a method is registered
    pub fn has_method(&self, method: &str) -> bool {
        self.handlers.contains_key(method)
    }

    /// Check if a method is streaming
    pub fn is_streaming(&self, method: &str) -> bool {
        self.streaming_methods.get(method).copied().unwrap_or(false)
    }

    /// Get all registered method names
    pub fn method_names(&self) -> Vec<&'static str> {
        self.handlers.keys().copied().collect()
    }

    /// Get the handler for a method
    pub fn get_handler(&self, method: &str) -> Option<&DynHandler> {
        self.handlers.get(method)
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Message Dispatcher
// =============================================================================

/// Dispatcher for routing and executing WebSocket messages
///
/// This struct combines the router with context and operation registry
/// to provide a complete message handling system.
pub struct Dispatcher {
    /// The router with registered handlers
    router: Arc<Router>,

    /// Shared context for all handlers
    context: Arc<WsContext>,

    /// Operation registry for streaming operations
    operations: Arc<OperationRegistry>,
}

impl Dispatcher {
    /// Create a new dispatcher
    pub fn new(router: Router, context: WsContext, operations: OperationRegistry) -> Self {
        Self {
            router: Arc::new(router),
            context: Arc::new(context),
            operations: Arc::new(operations),
        }
    }

    /// Create a new dispatcher with default settings
    pub fn with_router(router: Router) -> Self {
        Self::new(router, WsContext::default(), OperationRegistry::new())
    }

    /// Get a reference to the context
    pub fn context(&self) -> &Arc<WsContext> {
        &self.context
    }

    /// Get a reference to the operation registry
    pub fn operations(&self) -> &Arc<OperationRegistry> {
        &self.operations
    }

    /// Get a reference to the router
    pub fn router(&self) -> &Arc<Router> {
        &self.router
    }

    /// Dispatch a message to the appropriate handler
    ///
    /// This method:
    /// 1. Parses the request envelope
    /// 2. Validates the method exists
    /// 3. For streaming methods, registers the operation
    /// 4. Executes the handler
    /// 5. Ensures a terminal response is sent
    pub async fn dispatch(&self, message: &str, sink: WsSink) {
        // Parse request envelope
        let envelope: RequestEnvelope = match serde_json::from_str(message) {
            Ok(env) => env,
            Err(e) => {
                // Generate an ID for the error response
                let id = uuid::Uuid::new_v4().to_string();
                let op_sink = WsOpSink::new(sink.clone(), id.clone(), None);
                let _ = op_sink
                    .send_error(ErrorData::invalid_request(format!(
                        "Failed to parse request: {}",
                        e
                    )))
                    .await;
                return;
            }
        };

        // Convert to WsRequest (generates ID if not provided)
        let mut request = WsRequest::from_envelope(envelope);
        let id = request.id.clone();
        let method = request.method.clone();

        // Check if method exists
        let handler = match self.router.get_handler(&method) {
            Some(h) => h,
            None => {
                let op_sink = WsOpSink::new(sink.clone(), id.clone(), None);
                let _ = op_sink.send_error(ErrorData::unknown_method(&method)).await;
                return;
            }
        };

        let is_streaming = self.router.is_streaming(&method);

        // Strict op_id presence policy:
        // - streaming methods: server generates and attaches op_id
        // - non-streaming methods: op_id must be absent (always None)
        let op_id = if is_streaming {
            match self.operations.register(id.clone(), method.clone()).await {
                Ok((op_id, _cancel_token)) => {
                    request.op_id = Some(op_id.clone());
                    Some(op_id)
                }
                Err(_) => {
                    // Not a streaming op yet; respond without op_id.
                    let op_sink = WsOpSink::new(sink.clone(), id.clone(), None);
                    let _ = op_sink.send_error(ErrorData::rate_limited()).await;
                    return;
                }
            }
        } else {
            // Enforce: non-streaming requests never carry an op_id.
            request.op_id = None;
            None
        };

        // Create an operation-scoped sink and pass it into the handler
        let op_sink = WsOpSink::new(sink.clone(), id.clone(), op_id.clone());

        // Execute handler (handlers now receive WsOpSink)
        let ctx = Arc::clone(&self.context);
        let result = handler(ctx, request, op_sink.clone()).await;

        // Handle errors (terminal guarded). For streaming ops, handler error should mark FAIL.
        if let Err(e) = result {
            let _ = op_sink.send_error(e.to_error_data()).await;

            if let Some(ref op_id) = op_id {
                let _ = self.operations.fail_and_remove(op_id).await;
            }
            return;
        }

        // Successful completion. For streaming ops, mark COMPLETE.
        if let Some(ref op_id) = op_id {
            let _ = self.operations.complete_and_remove(op_id).await;
        }
    }

    /// Cancel an operation by op_id
    pub async fn cancel(&self, op_id: &str, request_id: String, sink: WsSink) {
        // Cancel responses are non-streaming; do not attach an op_id to the envelope.
        let op_sink = WsOpSink::new(sink.clone(), request_id.clone(), None);

        // Prefer cancel+remove to avoid registry growth.
        match self.operations.cancel_and_remove(op_id).await {
            Ok(()) => {
                let _ = op_sink
                    .send_result(serde_json::json!({
                        "cancelled": true,
                        "op_id": op_id
                    }))
                    .await;
            }
            Err(crate::server::operations::RegistryError::OperationNotFound) => {
                let _ = op_sink
                    .send_error(ErrorData::invalid_params(format!(
                        "Unknown operation: {}",
                        op_id
                    )))
                    .await;
            }
            Err(crate::server::operations::RegistryError::OperationNotRunning) => {
                let _ = op_sink
                    .send_error(ErrorData::invalid_params(format!(
                        "Operation {} is not running",
                        op_id
                    )))
                    .await;
            }
            Err(e) => {
                let _ = op_sink
                    .send_error(ErrorData::operation_failed(e.to_string()))
                    .await;
            }
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
    fn test_router_new() {
        let router = Router::new();
        assert!(router.method_names().is_empty());
    }

    #[test]
    fn test_router_has_method() {
        let router = Router::new();
        assert!(!router.has_method("time.parse"));
    }

    #[test]
    fn test_router_is_streaming_unknown() {
        let router = Router::new();
        assert!(!router.is_streaming("unknown.method"));
    }
}
