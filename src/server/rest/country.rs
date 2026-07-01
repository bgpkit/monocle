//! `POST /api/v1/country/lookup` — country code/name lookup.

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::lens::country::{CountryEntry, CountryLens, CountryLookupArgs};
use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};
use crate::server::ServerState;

#[derive(Debug, Clone, Deserialize)]
pub struct CountryLookupRequest {
    /// Search query: country code (e.g., "US") or partial name (e.g., "united").
    pub query: Option<String>,
    /// List all countries.
    #[serde(default)]
    pub all: bool,
}

pub async fn country_lookup(
    State(_state): State<ServerState>,
    Json(req): Json<CountryLookupRequest>,
) -> Result<Json<Vec<CountryEntry>>, ApiError> {
    if req.query.is_none() && !req.all {
        return Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            ApiErrorResponse::new(
                ApiErrorCode::InvalidParams,
                "Either 'query' or 'all: true' is required",
            ),
        ));
    }

    let query = req.query;
    let all = req.all;

    let results = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<CountryEntry>> {
        let lens = CountryLens::new();
        let args = if all {
            CountryLookupArgs::all_countries()
        } else {
            CountryLookupArgs::new(query.unwrap_or_default())
        };
        lens.search(&args)
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(results))
}
