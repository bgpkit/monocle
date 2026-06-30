//! AS2REL endpoints:
//! - `POST /api/v1/as2rel/search` — search AS relationships
//! - `GET  /api/v1/as2rel/relationship` — get relationship between two ASNs
//! - `POST /api/v1/as2rel/refresh` — refresh AS2REL data

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::database::MonocleDatabase;
use crate::lens::as2rel::{As2relLens, As2relSearchArgs, As2relSearchResult, As2relSortOrder};
use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};
use crate::server::ServerState;

// =============================================================================
// Search
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct As2relSearchRequest {
    /// One or more ASNs to query relationships for.
    #[serde(default)]
    pub asns: Vec<u32>,
    /// Sort by ASN2 ascending instead of connected percentage descending.
    #[serde(default)]
    pub sort_by_asn: bool,
    /// Show organization name for ASN2.
    #[serde(default)]
    pub show_name: bool,
    /// Minimum visibility percentage (0-100).
    #[serde(default)]
    pub min_visibility: Option<f32>,
    /// Only show ASNs that are single-homed to the queried ASN.
    #[serde(default)]
    pub single_homed: bool,
    /// Filter to only upstream relationships.
    #[serde(default)]
    pub is_upstream: bool,
    /// Filter to only downstream relationships.
    #[serde(default)]
    pub is_downstream: bool,
    /// Filter to only peer relationships.
    #[serde(default)]
    pub is_peer: bool,
}

pub async fn as2rel_search(
    State(state): State<ServerState>,
    Json(req): Json<As2relSearchRequest>,
) -> Result<Json<Vec<As2relSearchResult>>, ApiError> {
    if req.asns.is_empty() {
        return Err(ApiError::invalid_params("At least one ASN is required"));
    }

    let data_dir = state.config.data_dir.clone();
    let args = As2relSearchArgs {
        asns: req.asns,
        sort_by_asn: req.sort_by_asn,
        show_name: req.show_name,
        no_explain: true,
        min_visibility: req.min_visibility,
        single_homed: req.single_homed,
        is_upstream: req.is_upstream,
        is_downstream: req.is_downstream,
        is_peer: req.is_peer,
    };

    let results =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<As2relSearchResult>> {
            let db = MonocleDatabase::open_in_dir(&data_dir)?;
            let lens = As2relLens::new(&db);

            if !lens.is_data_available() {
                anyhow::bail!("NOT_INITIALIZED:AS2REL");
            }

            let mut results = lens.search(&args)?;
            lens.sort_results(&mut results, &As2relSortOrder::default());
            Ok(results)
        })
        .await
        .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?;

    match results {
        Ok(r) => Ok(Json(r)),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("NOT_INITIALIZED") {
                Err(ApiError::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    ApiErrorResponse::new(
                        ApiErrorCode::NotInitialized,
                        "AS2REL data not initialized. Run as2rel/refresh first.",
                    ),
                ))
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}

// =============================================================================
// Relationship
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct As2relRelationshipQuery {
    pub asn1: u32,
    pub asn2: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct As2relRelationshipResponse {
    pub asn1: u32,
    pub asn2: u32,
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<As2relSearchResult>,
}

pub async fn as2rel_relationship(
    State(state): State<ServerState>,
    Query(query): Query<As2relRelationshipQuery>,
) -> Result<Json<As2relRelationshipResponse>, ApiError> {
    let data_dir = state.config.data_dir.clone();
    let asn1 = query.asn1;
    let asn2 = query.asn2;

    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<As2relRelationshipResponse> {
            let db = MonocleDatabase::open_in_dir(&data_dir)?;
            let lens = As2relLens::new(&db);

            if !lens.is_data_available() {
                anyhow::bail!("NOT_INITIALIZED:AS2REL");
            }

            let args = As2relSearchArgs {
                asns: vec![asn1],
                sort_by_asn: false,
                show_name: false,
                no_explain: true,
                min_visibility: None,
                single_homed: false,
                is_upstream: false,
                is_downstream: false,
                is_peer: false,
            };

            let results = lens.search(&args)?;
            let found = results.iter().find(|r| r.asn2 == asn2).cloned();

            Ok(As2relRelationshipResponse {
                asn1,
                asn2,
                found: found.is_some(),
                relationship: found,
            })
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
                        "AS2REL data not initialized. Run as2rel/refresh first.",
                    ),
                ))
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}

// =============================================================================
// Refresh
// =============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct As2relRefreshResponse {
    pub updated_records: usize,
}

pub async fn as2rel_refresh(
    State(state): State<ServerState>,
) -> Result<Json<As2relRefreshResponse>, ApiError> {
    let data_dir = state.config.data_dir.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<As2relRefreshResponse> {
        let db = MonocleDatabase::open_in_dir(&data_dir)?;
        let lens = As2relLens::new(&db);
        let count = lens.update()?;
        Ok(As2relRefreshResponse {
            updated_records: count,
        })
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(result))
}
