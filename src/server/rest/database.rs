//! Database endpoints:
//! - `GET  /api/v1/database/status` — report database state (no side effects)
//! - `POST /api/v1/database/refresh` — refresh a data source
//! - `POST /api/v1/inspect/refresh` — refresh all inspect data sources

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::config::{get_data_source_info, get_sqlite_info, DataSourceInfo, SqliteDatabaseInfo};
use crate::database::MonocleDatabase;
use crate::lens::inspect::InspectLens;
use crate::lens::rpki::RpkiLens;
use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};
use crate::server::ServerState;

// =============================================================================
// Database Status
// =============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseStatusResponse {
    pub sqlite: SqliteDatabaseInfo,
    pub sources: Vec<DataSourceInfo>,
}

pub async fn database_status(
    State(state): State<ServerState>,
) -> Result<Json<DatabaseStatusResponse>, ApiError> {
    let config = state.config.as_ref().clone();
    let response =
        tokio::task::spawn_blocking(move || -> anyhow::Result<DatabaseStatusResponse> {
            Ok(DatabaseStatusResponse {
                sqlite: get_sqlite_info(&config),
                sources: get_data_source_info(&config),
            })
        })
        .await
        .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(response))
}

// =============================================================================
// Database Refresh
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseRefreshRequest {
    /// Data source to refresh: "rpki", "as2rel", "pfx2as", "asinfo", or "all".
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseRefreshResponse {
    pub source: String,
    pub message: String,
}

pub async fn database_refresh(
    State(state): State<ServerState>,
    Json(req): Json<DatabaseRefreshRequest>,
) -> Result<Json<DatabaseRefreshResponse>, ApiError> {
    let source = req.source.to_lowercase();
    match source.as_str() {
        "rpki" | "as2rel" | "pfx2as" | "asinfo" | "all" => {}
        other => {
            return Err(ApiError::invalid_params(format!(
                "Invalid source '{}': use 'rpki', 'as2rel', 'pfx2as', 'asinfo', or 'all'",
                other
            )))
        }
    }

    // pfx2as refresh is not yet implemented via the HTTP API — return 501 so
    // automation can detect that the operation did not run.
    if source == "pfx2as" {
        return Err(ApiError::new(
            axum::http::StatusCode::NOT_IMPLEMENTED,
            ApiErrorResponse::new(
                ApiErrorCode::InvalidRequest,
                "pfx2as refresh via API is not yet implemented; use CLI 'monocle config update --pfx2as'",
            ),
        ));
    }

    let data_dir = state.config.data_dir.clone();
    let source_clone = source.clone();

    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let db = MonocleDatabase::open_in_dir(&data_dir)?;
            let message = match source_clone.as_str() {
                "rpki" => {
                    let lens = RpkiLens::new(&db);
                    let (roas, aspas) = lens.refresh()?;
                    format!("Refreshed RPKI: {} ROAs, {} ASPAs", roas, aspas)
                }
                "as2rel" => {
                    let count = db.refresh_as2rel()?;
                    format!("Refreshed AS2REL: {} entries", count)
                }
                "asinfo" => {
                    let counts = db.refresh_asinfo()?;
                    format!(
                        "Refreshed ASInfo: {} core, {} as2org, {} peeringdb, {} hegemony, {} population",
                        counts.core, counts.as2org, counts.peeringdb, counts.hegemony, counts.population
                    )
                }
                "all" => {
                    let mut messages = Vec::new();

                    let counts = db.refresh_asinfo()?;
                    messages.push(format!(
                        "ASInfo: {} core, {} as2org",
                        counts.core, counts.as2org
                    ));

                    let count = db.refresh_as2rel()?;
                    messages.push(format!("AS2REL: {} entries", count));

                    let lens = RpkiLens::new(&db);
                    let (roas, aspas) = lens.refresh()?;
                    messages.push(format!("RPKI: {} ROAs, {} ASPAs", roas, aspas));

                    format!("Refreshed all sources: {}", messages.join("; "))
                }
                _ => unreachable!(),
            };
            Ok(message)
        })
        .await
        .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(DatabaseRefreshResponse {
        source,
        message: result,
    }))
}

// =============================================================================
// Inspect Refresh
// =============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct InspectRefreshResponse {
    pub message: String,
}

pub async fn inspect_refresh(
    State(state): State<ServerState>,
) -> Result<Json<InspectRefreshResponse>, ApiError> {
    let config = state.config.as_ref().clone();

    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<InspectRefreshResponse> {
            let db = MonocleDatabase::open_in_dir(&config.data_dir)?;
            let lens = InspectLens::new(&db, &config);
            let counts = lens.refresh()?;
            Ok(InspectRefreshResponse {
                message: format!(
                    "Inspect data refreshed: {} core, {} as2org, {} peeringdb, {} hegemony, {} population",
                    counts.core, counts.as2org, counts.peeringdb, counts.hegemony, counts.population
                ),
            })
        })
        .await
        .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(result))
}
