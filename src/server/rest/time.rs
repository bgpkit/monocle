//! `POST /api/v1/time/parse` — parse time strings.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::lens::time::{TimeBgpTime, TimeLens, TimeParseArgs};
use crate::server::http::ApiError;
use crate::server::ServerState;

#[derive(Debug, Clone, Deserialize)]
pub struct TimeParseRequest {
    /// Time strings to parse (Unix timestamp, RFC3339, or human-readable).
    /// Empty = current time.
    #[serde(default)]
    pub times: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeParseResponse {
    pub results: Vec<TimeBgpTime>,
}

pub async fn time_parse(
    State(_state): State<ServerState>,
    Json(req): Json<TimeParseRequest>,
) -> Result<Json<TimeParseResponse>, ApiError> {
    let lens = TimeLens::new();
    let args = TimeParseArgs::new(req.times);
    let results = lens
        .parse(&args)
        .map_err(|e| ApiError::invalid_params(e.to_string()))?;
    Ok(Json(TimeParseResponse { results }))
}
