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

pub mod auth;
pub mod http;
pub mod rest;
pub mod search;

use axum::middleware::from_fn_with_state;
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

    let auth_enabled = config.server_auth_enabled;
    let auth_token = config.server_auth_token.clone();

    let state = ServerState {
        config: Arc::new(config),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_router = if auth_enabled {
        if auth_token.is_empty() {
            anyhow::bail!(
                "server_auth_enabled is true but server_auth_token is empty. \
                 Set MONOCLE_SERVER_AUTH_TOKEN or server_auth_token in config."
            );
        }
        let auth_state = auth::AuthState {
            expected_token: Arc::new(auth_token),
        };
        http::router(state).layer(from_fn_with_state(auth_state, auth::require_token))
    } else {
        http::router(state)
    };

    let app = AxumRouter::new()
        .route("/health", get(health_handler))
        .nest("/api/v1", api_router)
        .layer(cors);

    tracing::info!(
        "Starting HTTP server on {} (auth: {})",
        bind_address,
        auth_enabled
    );

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
