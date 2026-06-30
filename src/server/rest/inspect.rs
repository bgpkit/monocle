//! `POST /api/v1/inspect/query` — unified AS/prefix/country lookup.

use std::collections::HashSet;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::lens::inspect::{InspectDataSection, InspectLens, InspectQueryOptions, InspectResult};
use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};
use crate::server::ServerState;

#[derive(Debug, Clone, Deserialize)]
pub struct InspectQueryRequest {
    /// One or more queries (ASN, prefix, or AS name).
    #[serde(default)]
    pub queries: Vec<String>,
    /// Data sections to include. None = defaults based on query type.
    #[serde(default)]
    pub select: Option<Vec<String>>,
    /// Maximum ROAs to return (0 = unlimited).
    #[serde(default)]
    pub max_roas: Option<usize>,
    /// Maximum prefixes to return (0 = unlimited).
    #[serde(default)]
    pub max_prefixes: Option<usize>,
    /// Maximum neighbors per category (0 = unlimited).
    #[serde(default)]
    pub max_neighbors: Option<usize>,
}

pub async fn inspect_query(
    State(state): State<ServerState>,
    Json(req): Json<InspectQueryRequest>,
) -> Result<Json<InspectResult>, ApiError> {
    if req.queries.is_empty() {
        return Err(ApiError::invalid_params("At least one query is required"));
    }

    let data_dir = state.config.data_dir.clone();
    let config = state.config.as_ref().clone();

    let select = req.select.map(|s| {
        s.into_iter()
            .filter_map(|name| match name.as_str() {
                "basic" => Some(InspectDataSection::Basic),
                "prefixes" => Some(InspectDataSection::Prefixes),
                "connectivity" => Some(InspectDataSection::Connectivity),
                "rpki" => Some(InspectDataSection::Rpki),
                _ => None,
            })
            .collect::<HashSet<_>>()
    });

    let options = InspectQueryOptions {
        select,
        max_roas: req.max_roas.unwrap_or(0),
        max_prefixes: req.max_prefixes.unwrap_or(0),
        max_neighbors: req.max_neighbors.unwrap_or(0),
        max_search_results: 0,
    };

    let queries = req.queries.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<InspectResult> {
        let db = crate::database::MonocleDatabase::open_in_dir(&data_dir)?;
        let lens = InspectLens::new(&db, &config);

        if !lens.is_data_available() {
            anyhow::bail!("NOT_INITIALIZED:INSPECT");
        }

        let result = lens.query(&queries, &options)?;
        Ok(result)
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?;

    match result {
        Ok(r) => Ok(Json(r)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("NOT_INITIALIZED") {
                Err(ApiError::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    ApiErrorResponse::new(
                        ApiErrorCode::NotInitialized,
                        "Inspect data not initialized. Run inspect/refresh first.",
                    ),
                ))
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}
