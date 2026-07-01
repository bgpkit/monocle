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

/// Constant-time comparison of two byte slices to reduce timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Middleware function: reject requests without a valid Bearer token.
pub async fn require_token(
    State(auth): State<AuthState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    // The HTTP auth scheme token ("Bearer") is case-insensitive per RFC 7235.
    // Split on the first space and compare the scheme case-insensitively.
    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            let (scheme, rest) = v.split_once(' ')?;
            if scheme.eq_ignore_ascii_case("Bearer") {
                Some(rest.trim())
            } else {
                None
            }
        });

    match token {
        Some(t) if constant_time_eq(t.as_bytes(), auth.expected_token.as_bytes()) => {
            Ok(next.run(req).await)
        }
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

    #[test]
    fn test_constant_time_eq_same() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn test_constant_time_eq_diff_len() {
        assert!(!constant_time_eq(b"hello", b"hello!"));
    }

    #[test]
    fn test_constant_time_eq_diff_content() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn test_constant_time_eq_empty() {
        assert!(constant_time_eq(b"", b""));
    }
}
