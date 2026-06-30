//! Token-based auth middleware for `/api/v1/*` endpoints.
//!
//! When `server_auth_enabled` is true, requests must include
//! `Authorization: Bearer <token>`. The `/health` endpoint is always open.
//!
//! ```text
//! Authorization: Bearer my-secret-token
//! ```

use axum::extract::{Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::sync::Arc;

use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};

/// The expected bearer token, passed as middleware state.
#[derive(Clone, Debug)]
pub struct AuthState {
    pub expected_token: Arc<String>,
}

/// Middleware function: reject requests without a valid Bearer token.
pub async fn require_token(
    State(auth): State<AuthState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string());

    match token {
        Some(t) if t == *auth.expected_token => Ok(next.run(req).await),
        _ => Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            ApiErrorResponse::new(
                ApiErrorCode::InvalidRequest,
                "Missing or invalid Authorization header. Expected: Bearer <token>",
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_clone() {
        let state = AuthState {
            expected_token: Arc::new("secret".to_string()),
        };
        let _cloned = state.clone();
    }
}
