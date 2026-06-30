//! HTTP + SSE server module for Monocle.
//!
//! This module provides an HTTP API server with:
//! - `GET /health` — health check for container orchestration
//! - `GET /api/v1/system/info` — server metadata and endpoint list
//! - `POST /api/v1/search/stream` — SSE streaming BGP search
//!
//! # Architecture
//!
//! - `http` — REST routes, API error types, system info handler
//! - `search` — SSE search streaming handler, wire DTOs, worker loop
//!
//! # Usage
//!
//! ```rust,ignore
//! use monocle::config::MonocleConfig;
//! use monocle::server::start_server;
//!
//! let config = MonocleConfig::new(&None)?;
//! start_server(config).await?;
//! ```

pub mod http;
pub mod rest;
pub mod search;

use axum::routing::get;
use axum::Router as AxumRouter;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use crate::config::MonocleConfig;

// =============================================================================
// Server State
// =============================================================================

/// Shared server state, cloned across handlers.
#[derive(Clone)]
pub struct ServerState {
    pub config: Arc<MonocleConfig>,
}

// =============================================================================
// Server Startup
// =============================================================================

/// Start the HTTP server. Blocks until the server shuts down.
pub async fn start_server(config: MonocleConfig) -> anyhow::Result<()> {
    let bind_address = format!("{}:{}", config.server_address, config.server_port);
    let state = ServerState {
        config: Arc::new(config),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = AxumRouter::new()
        .route("/health", get(health_handler))
        .nest("/api/v1", http::router(state))
        .layer(cors);

    tracing::info!("Starting HTTP server on {}", bind_address);

    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// `GET /health` — returns `OK` for container health checks.
async fn health_handler() -> &'static str {
    "OK"
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_state_clone() {
        let config = MonocleConfig::default();
        let state = ServerState {
            config: Arc::new(config),
        };
        let _cloned = state.clone();
    }
}
