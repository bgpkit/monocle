//! RPKI endpoints:
//! - `GET  /api/v1/rpki/roa/lookup` — list ROAs from local cache
//! - `GET  /api/v1/rpki/aspa/lookup` — list ASPAs from local cache
//! - `POST /api/v1/rpki/roa/validate` — validate prefix+ASN against ROAs
//! - `POST /api/v1/rpki/aspa/validate` — check if provider is authorized by customer

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::database::MonocleDatabase;
use crate::lens::rpki::{RpkiLens, RpkiValidationResult};
use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};
use crate::server::ServerState;

// =============================================================================
// ROA Lookup
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct RoaLookupQuery {
    /// Filter by prefix.
    pub prefix: Option<String>,
    /// Filter by origin ASN.
    pub asn: Option<u32>,
}

pub async fn roa_lookup(
    State(state): State<ServerState>,
    Query(query): Query<RoaLookupQuery>,
) -> Result<Json<Vec<RpkiRoaEntryResponse>>, ApiError> {
    let data_dir = state.config.data_dir.clone();
    let prefix = query.prefix.clone();
    let asn = query.asn;

    let results =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<RpkiRoaEntryResponse>> {
            let db = MonocleDatabase::open_in_dir(&data_dir)?;
            let rpki = db.rpki();

            if rpki.is_empty() {
                anyhow::bail!("NOT_INITIALIZED:RPKI");
            }

            let records: Vec<RpkiRoaEntryResponse> = if let Some(asn) = asn {
                rpki.get_roas_by_asn(asn)?
                    .into_iter()
                    .map(|r| RpkiRoaEntryResponse {
                        prefix: r.prefix,
                        max_length: r.max_length,
                        origin_asn: r.origin_asn,
                        ta: r.ta,
                    })
                    .collect()
            } else if let Some(_prefix) = prefix {
                // For prefix filtering, use the lens which handles this
                let mut lens = RpkiLens::new(&db);
                use crate::lens::rpki::{RpkiDataSource, RpkiOutputFormat, RpkiRoaLookupArgs};
                let args = RpkiRoaLookupArgs {
                    prefix: query.prefix,
                    asn: None,
                    date: None,
                    source: RpkiDataSource::default(),
                    collector: None,
                    format: RpkiOutputFormat::default(),
                };
                lens.get_roas(&args)?
                    .into_iter()
                    .map(|r| RpkiRoaEntryResponse {
                        prefix: r.prefix,
                        max_length: r.max_length,
                        origin_asn: r.origin_asn,
                        ta: r.ta,
                    })
                    .collect()
            } else {
                rpki.get_all_roas()?
                    .into_iter()
                    .map(|r| RpkiRoaEntryResponse {
                        prefix: r.prefix,
                        max_length: r.max_length,
                        origin_asn: r.origin_asn,
                        ta: r.ta,
                    })
                    .collect()
            };

            Ok(records)
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
                        "RPKI data not initialized. Run database/refresh first.",
                    ),
                ))
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RpkiRoaEntryResponse {
    pub prefix: String,
    pub max_length: u8,
    pub origin_asn: u32,
    pub ta: String,
}

// =============================================================================
// ASPA Lookup
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct AspaLookupQuery {
    /// Filter by customer ASN.
    pub customer_asn: Option<u32>,
    /// Filter by provider ASN.
    pub provider_asn: Option<u32>,
}

pub async fn aspa_lookup(
    State(state): State<ServerState>,
    Query(query): Query<AspaLookupQuery>,
) -> Result<Json<Vec<RpkiAspaEntryResponse>>, ApiError> {
    let data_dir = state.config.data_dir.clone();
    let customer_asn = query.customer_asn;
    let provider_asn = query.provider_asn;

    let results =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<RpkiAspaEntryResponse>> {
            let db = MonocleDatabase::open_in_dir(&data_dir)?;
            let rpki = db.rpki();

            if rpki.is_empty() {
                anyhow::bail!("NOT_INITIALIZED:RPKI");
            }

            let records = if let Some(customer) = customer_asn {
                rpki.get_aspas_by_customer_enriched(customer)?
            } else if let Some(provider) = provider_asn {
                rpki.get_aspas_by_provider_enriched(provider)?
            } else {
                rpki.get_all_aspas_enriched()?
            };

            Ok(records
                .into_iter()
                .map(|r| RpkiAspaEntryResponse {
                    customer_asn: r.customer_asn,
                    customer_name: r.customer_name,
                    customer_country: r.customer_country,
                    providers: r
                        .providers
                        .into_iter()
                        .map(|p| RpkiAspaProviderResponse {
                            asn: p.asn,
                            name: p.name,
                        })
                        .collect(),
                })
                .collect())
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
                        "RPKI data not initialized. Run database/refresh first.",
                    ),
                ))
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RpkiAspaEntryResponse {
    pub customer_asn: u32,
    pub customer_name: Option<String>,
    pub customer_country: Option<String>,
    pub providers: Vec<RpkiAspaProviderResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RpkiAspaProviderResponse {
    pub asn: u32,
    pub name: Option<String>,
}

// =============================================================================
// ROA Validation
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
pub struct RoaValidateRequest {
    /// Prefix to validate (e.g., "1.1.1.0/24").
    pub prefix: String,
    /// Origin ASN to validate.
    pub asn: u32,
}

pub async fn roa_validate(
    State(state): State<ServerState>,
    Json(req): Json<RoaValidateRequest>,
) -> Result<Json<RpkiValidationResult>, ApiError> {
    // Validate prefix format
    req.prefix
        .parse::<ipnet::IpNet>()
        .map_err(|e| ApiError::invalid_params(format!("Invalid prefix: {}", e)))?;

    let data_dir = state.config.data_dir.clone();
    let prefix = req.prefix.clone();
    let asn = req.asn;

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<RpkiValidationResult> {
        let db = MonocleDatabase::open_in_dir(&data_dir)?;
        let rpki = db.rpki();

        if rpki.is_empty() {
            anyhow::bail!("NOT_INITIALIZED:RPKI");
        }

        let lens = RpkiLens::new(&db);
        let result = lens.validate(&prefix, asn)?;
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
                        "RPKI data not initialized. Run database/refresh first.",
                    ),
                ))
            } else {
                Err(ApiError::internal(msg))
            }
        }
    }
}

// =============================================================================
// ASPA Validation (TODO — not exposed)
// =============================================================================
// Proper ASPA validation requires checking full AS paths against ASPA records
// combined with AS relationship inference data (as2rel). This is deferred.
// The simple membership-check approach is insufficient and has been removed.
//
// See: RFC 8481 and ASPA Internet-Drafts for the full validation algorithm.

// pub async fn aspa_validate(...) — TODO: implement with full AS path validation
//   using ASPA records + as2rel relationship inference data.
