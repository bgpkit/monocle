//! `GET /api/v1/pfx2as/lookup` — prefix-to-ASN mapping lookup.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::database::MonocleDatabase;
use crate::lens::pfx2as::{
    Pfx2asDetailedResult, Pfx2asLens, Pfx2asLookupArgs, Pfx2asLookupMode, Pfx2asOutputFormat,
};
use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};
use crate::server::ServerState;

#[derive(Debug, Clone, Deserialize)]
pub struct Pfx2asLookupQuery {
    /// Prefix to look up (e.g., "1.1.1.0/24").
    pub prefix: String,
    /// Lookup mode: "exact", "longest", "covering", "covered" (default: longest).
    #[serde(default)]
    pub mode: Option<String>,
}

pub async fn pfx2as_lookup(
    State(state): State<ServerState>,
    Query(query): Query<Pfx2asLookupQuery>,
) -> Result<Json<Vec<Pfx2asDetailedResult>>, ApiError> {
    let data_dir = state.config.data_dir.clone();
    let prefix = query.prefix.clone();
    let mode_str = query.mode.clone().unwrap_or_else(|| "longest".to_string());

    let results =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Pfx2asDetailedResult>> {
            let db = MonocleDatabase::open_in_dir(&data_dir)?;
            let lens = Pfx2asLens::new(&db);

            let count = lens.record_count()?;
            if count == 0 {
                anyhow::bail!("NOT_INITIALIZED:PFX2AS");
            }

            let mode = match mode_str.as_str() {
                "exact" => Pfx2asLookupMode::Exact,
                "longest" => Pfx2asLookupMode::Longest,
                "covering" => Pfx2asLookupMode::Covering,
                "covered" => Pfx2asLookupMode::Covered,
                other => anyhow::bail!(
                    "Invalid mode '{}': expected exact, longest, covering, or covered",
                    other
                ),
            };
            let args = Pfx2asLookupArgs {
                prefix: prefix.clone(),
                mode,
                format: Pfx2asOutputFormat::default(),
            };

            let results = lens.lookup(&args)?;
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
                        "Pfx2as data not initialized. Run database/refresh first.",
                    ),
                ))
            } else {
                Err(ApiError::invalid_params(msg))
            }
        }
    }
}
