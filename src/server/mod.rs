//! WebSocket server module for Monocle
//!
//! This module provides a WebSocket API server for Monocle, enabling real-time
//! communication with clients for BGP data operations.
//!
//! # Architecture
//!
//! The server is organized into several submodules:
//!
//! - `protocol` - Protocol types (request/response envelopes, error codes)
//! - `query` - Non-core protocol helper types (pagination/filters) used by query/streaming methods
//! - `handler` - Handler trait and context for method implementations
//! - `sink` - WebSocket sink abstraction for typed envelope writing (transport-level)
//! - `op_sink` - Operation-scoped sink enforcing streaming terminal semantics (protocol-level)
//! - `router` - Registry-based method routing
//! - `operations` - Operation registry for streaming operations and cancellation
//! - `handlers` - Individual method handler implementations
//!
//! # Connection lifecycle
//!
//! The WebSocket connection loop enforces:
//! - max message size (`ServerConfig.max_message_size`)
//! - periodic ping keepalive (`ServerConfig.ping_interval_secs`)
//! - idle timeout (`ServerConfig.connection_timeout_secs`)
//!
//! # Usage
//!
//! ```rust,ignore
//! use monocle::server::{create_router, WsContext, ServerConfig};
//!
//! // Create the router with all handlers registered
//! let router = create_router();
//!
//! // Create context
//! let context = WsContext::new("~/.monocle".to_string());
//!
//! // Start the server
//! let config = ServerConfig::default();
//! start_server(router, context, config).await?;
//! ```

pub mod handler;
pub mod handlers;
pub mod op_sink;
pub mod operations;
pub mod protocol;
pub mod query;
pub mod router;
pub mod sink;

// Re-export commonly used types
pub use handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
pub use op_sink::{WsOpSink, WsOpSinkError};
pub use operations::{OperationRegistry, OperationStatus};
pub use protocol::{
    ErrorCode, ErrorData, ProgressStage, RequestEnvelope, ResponseEnvelope, ResponseType,
    SystemInfo,
};
pub use router::{Dispatcher, Router};
pub use sink::{WsSink, WsSinkError};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::get,
    Router as AxumRouter,
};
use futures::StreamExt;
use std::sync::Arc;
use tokio::time::{Duration, Instant};
use tower_http::cors::{Any, CorsLayer};

// =============================================================================
// Server Configuration
// =============================================================================

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Address to bind to
    pub address: String,

    /// Port to listen on
    pub port: u16,

    /// Maximum concurrent operations per connection
    pub max_concurrent_ops: usize,

    /// Maximum message size in bytes
    pub max_message_size: usize,

    /// Connection timeout in seconds
    pub connection_timeout_secs: u64,

    /// Ping interval in seconds
    pub ping_interval_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1".to_string(),
            port: 8080,
            max_concurrent_ops: 10,
            max_message_size: 1024 * 1024, // 1MB
            connection_timeout_secs: 300,  // 5 minutes
            ping_interval_secs: 30,
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the address
    pub fn with_address(mut self, address: impl Into<String>) -> Self {
        self.address = address.into();
        self
    }

    /// Set the port
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Get the full bind address
    pub fn bind_address(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }
}

// =============================================================================
// Router Creation
// =============================================================================

/// Create a router with all handlers registered
pub fn create_router() -> Router {
    use handlers::*;

    let mut router = Router::new();

    // System handlers
    router.register::<SystemInfoHandler>();

    // Time handlers
    router.register::<TimeParseHandler>();

    // Country handlers
    router.register::<CountryLookupHandler>();

    // IP handlers
    router.register::<IpLookupHandler>();
    router.register::<IpPublicHandler>();

    // RPKI handlers
    router.register::<RpkiValidateHandler>();
    router.register::<RpkiRoasHandler>();
    router.register::<RpkiAspasHandler>();

    // AS2Org handlers
    router.register::<As2orgSearchHandler>();
    router.register::<As2orgBootstrapHandler>();

    // AS2Rel handlers
    router.register::<As2relSearchHandler>();
    router.register::<As2relRelationshipHandler>();
    router.register::<As2relUpdateHandler>();

    // Pfx2as handlers
    router.register::<Pfx2asLookupHandler>();

    // Database handlers
    router.register::<DatabaseStatusHandler>();
    router.register::<DatabaseRefreshHandler>();

    router
}

// =============================================================================
// Server State
// =============================================================================

/// Shared server state
#[derive(Clone)]
pub struct ServerState {
    /// Dispatcher for routing messages
    pub dispatcher: Arc<Dispatcher>,

    /// Server configuration
    pub config: Arc<ServerConfig>,
}

// =============================================================================
// Axum Router Creation
// =============================================================================

/// Create the Axum router for the WebSocket server
pub fn create_axum_router(state: ServerState) -> AxumRouter {
    // Configure CORS
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    AxumRouter::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(health_handler))
        .layer(cors)
        .with_state(state)
}

/// Health check handler
async fn health_handler() -> &'static str {
    "OK"
}

/// WebSocket upgrade handler
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<ServerState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle a WebSocket connection
async fn handle_socket(socket: WebSocket, state: ServerState) {
    let (sender, mut receiver) = socket.split();
    let sink = WsSink::new(sender);

    tracing::info!("WebSocket connection established");

    let max_message_size = state.config.max_message_size;
    let ping_interval = Duration::from_secs(state.config.ping_interval_secs.max(1));
    let idle_timeout = Duration::from_secs(state.config.connection_timeout_secs.max(1));

    let mut last_activity = Instant::now();
    let mut next_ping = Instant::now() + ping_interval;

    // Connection loop: enforce max message size, periodic ping keepalive, and idle timeout.
    loop {
        tokio::select! {
            maybe_msg = receiver.next() => {
                let Some(msg) = maybe_msg else {
                    break;
                };

                match msg {
                    Ok(Message::Text(text)) => {
                        if text.len() > max_message_size {
                            tracing::warn!(
                                "Closing connection: text message too large ({} > {} bytes)",
                                text.len(),
                                max_message_size
                            );
                            let _ = sink.send_message_raw(Message::Close(None)).await;
                            break;
                        }
                        last_activity = Instant::now();
                        tracing::debug!("Received message: {}", text);
                        state.dispatcher.dispatch(&text, sink.clone()).await;
                    }
                    Ok(Message::Binary(data)) => {
                        if data.len() > max_message_size {
                            tracing::warn!(
                                "Closing connection: binary message too large ({} > {} bytes)",
                                data.len(),
                                max_message_size
                            );
                            let _ = sink.send_message_raw(Message::Close(None)).await;
                            break;
                        }
                        last_activity = Instant::now();

                        // Try to parse binary as UTF-8 text
                        match String::from_utf8(data) {
                            Ok(text) => {
                                tracing::debug!("Received binary message as text: {}", text);
                                state.dispatcher.dispatch(&text, sink.clone()).await;
                            }
                            Err(_) => {
                                tracing::warn!("Received non-UTF8 binary message, ignoring");
                            }
                        }
                    }
                    Ok(Message::Ping(data)) => {
                        last_activity = Instant::now();
                        // Respond with pong
                        if let Err(e) = sink.send_message_raw(Message::Pong(data)).await {
                            tracing::warn!("Failed to send pong: {}", e);
                            break;
                        }
                    }
                    Ok(Message::Pong(_)) => {
                        last_activity = Instant::now();
                        // Ignore pong responses
                    }
                    Ok(Message::Close(_)) => {
                        tracing::info!("WebSocket connection closed by client");
                        break;
                    }
                    Err(e) => {
                        tracing::error!("WebSocket error: {}", e);
                        break;
                    }
                }
            }

            _ = tokio::time::sleep_until(next_ping) => {
                // Idle timeout check
                if last_activity.elapsed() > idle_timeout {
                    tracing::info!(
                        "Closing connection due to idle timeout (>{}s)",
                        idle_timeout.as_secs()
                    );
                    let _ = sink.send_message_raw(Message::Close(None)).await;
                    break;
                }

                // Periodic ping keepalive
                if let Err(e) = sink.send_message_raw(Message::Ping(Vec::new())).await {
                    tracing::warn!("Failed to send ping: {}", e);
                    break;
                }

                next_ping = Instant::now() + ping_interval;
            }
        }
    }

    tracing::info!("WebSocket connection closed");
}

// =============================================================================
// Server Startup
// =============================================================================

/// Start the WebSocket server
pub async fn start_server(
    router: Router,
    context: WsContext,
    config: ServerConfig,
) -> anyhow::Result<()> {
    let operations = OperationRegistry::with_max_concurrent(config.max_concurrent_ops);
    let dispatcher = Dispatcher::new(router, context, operations);

    let state = ServerState {
        dispatcher: Arc::new(dispatcher),
        config: Arc::new(config.clone()),
    };

    let app = create_axum_router(state);

    let bind_address = config.bind_address();
    tracing::info!("Starting WebSocket server on {}", bind_address);

    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.address, "127.0.0.1");
        assert_eq!(config.port, 8080);
        assert_eq!(config.max_concurrent_ops, 10);
    }

    #[test]
    fn test_server_config_builder() {
        let config = ServerConfig::new().with_address("0.0.0.0").with_port(9000);

        assert_eq!(config.address, "0.0.0.0");
        assert_eq!(config.port, 9000);
        assert_eq!(config.bind_address(), "0.0.0.0:9000");
    }

    #[test]
    fn test_create_router() {
        let router = create_router();

        // Check that key methods are registered
        assert!(router.has_method("system.info"));
        assert!(router.has_method("time.parse"));
        assert!(router.has_method("country.lookup"));
        assert!(router.has_method("ip.lookup"));
        assert!(router.has_method("ip.public"));
        assert!(router.has_method("rpki.validate"));
        assert!(router.has_method("rpki.roas"));
        assert!(router.has_method("rpki.aspas"));
        assert!(router.has_method("as2org.search"));
        assert!(router.has_method("as2org.bootstrap"));
        assert!(router.has_method("as2rel.search"));
        assert!(router.has_method("as2rel.relationship"));
        assert!(router.has_method("as2rel.update"));
        assert!(router.has_method("pfx2as.lookup"));
        assert!(router.has_method("database.status"));
        assert!(router.has_method("database.refresh"));

        // Check that unknown methods return false
        assert!(!router.has_method("unknown.method"));
    }

    #[test]
    fn test_router_streaming_flags() {
        let router = create_router();

        // Non-streaming methods
        assert!(!router.is_streaming("system.info"));
        assert!(!router.is_streaming("time.parse"));
        assert!(!router.is_streaming("rpki.validate"));

        // Unknown methods should return false
        assert!(!router.is_streaming("unknown.method"));
    }
}
