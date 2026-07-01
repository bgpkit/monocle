//! `POST /api/v1/ip/lookup` and `GET /api/v1/ip/public` — IP information.

use std::net::IpAddr;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;

use crate::lens::ip::{IpInfo, IpLens, IpLookupArgs};
use crate::server::http::ApiError;
use crate::server::ServerState;

#[derive(Debug, Clone, Deserialize)]
pub struct IpLookupRequest {
    /// IP address to look up.
    pub ip: String,
    /// Return simplified response.
    #[serde(default)]
    pub simple: bool,
}

pub async fn ip_lookup(
    State(_state): State<ServerState>,
    Json(req): Json<IpLookupRequest>,
) -> Result<Json<IpInfo>, ApiError> {
    let ip: IpAddr = req
        .ip
        .parse()
        .map_err(|e| ApiError::invalid_params(format!("Invalid IP address: {}", e)))?;

    let simple = req.simple;

    let info = tokio::task::spawn_blocking(move || -> anyhow::Result<IpInfo> {
        let lens = IpLens::new();
        let args = IpLookupArgs::new(ip).with_simple(simple);
        lens.lookup(&args)
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(info))
}

pub async fn ip_public(State(_state): State<ServerState>) -> Result<Json<IpInfo>, ApiError> {
    let info = tokio::task::spawn_blocking(move || -> anyhow::Result<IpInfo> {
        let lens = IpLens::new();
        let args = IpLookupArgs::public_ip();
        lens.lookup(&args)
    })
    .await
    .map_err(|e| ApiError::internal(format!("Task join error: {}", e)))?
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(info))
}
