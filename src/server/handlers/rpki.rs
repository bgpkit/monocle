//! RPKI handlers for RPKI validation and lookup operations
//!
//! This module provides handlers for RPKI-related methods like `rpki.validate`,
//! `rpki.roas`, and `rpki.aspas`.

use crate::database::{MonocleDatabase, RpkiAspaRecord, RpkiRoaRecord, RpkiValidationState};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// rpki.validate
// =============================================================================

/// Parameters for rpki.validate
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpkiValidateParams {
    /// IP prefix to validate (e.g., "1.1.1.0/24")
    pub prefix: String,

    /// AS number to validate
    pub asn: u32,
}

/// Validation state for a prefix-ASN pair
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationState {
    Valid,
    Invalid,
    NotFound,
}

impl From<crate::database::RpkiValidationState> for ValidationState {
    fn from(state: crate::database::RpkiValidationState) -> Self {
        match state {
            crate::database::RpkiValidationState::Valid => ValidationState::Valid,
            crate::database::RpkiValidationState::Invalid => ValidationState::Invalid,
            crate::database::RpkiValidationState::NotFound => ValidationState::NotFound,
        }
    }
}

/// Validation result details
#[derive(Debug, Clone, Serialize)]
pub struct ValidationDetails {
    /// The validated prefix
    pub prefix: String,

    /// The validated ASN
    pub asn: u32,

    /// Validation state
    pub state: ValidationState,

    /// Human-readable reason
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Covering ROA entry
#[derive(Debug, Clone, Serialize)]
pub struct CoveringRoa {
    /// ROA prefix
    pub prefix: String,

    /// Maximum prefix length
    pub max_length: u8,

    /// Origin ASN
    pub origin_asn: u32,

    /// Trust anchor
    pub ta: String,
}

/// Response for rpki.validate
#[derive(Debug, Clone, Serialize)]
pub struct RpkiValidateResponse {
    /// Validation result
    pub validation: ValidationDetails,

    /// Covering ROAs (if any)
    pub covering_roas: Vec<CoveringRoa>,
}

/// Handler for rpki.validate method
pub struct RpkiValidateHandler;

#[async_trait]
impl WsMethod for RpkiValidateHandler {
    const METHOD: &'static str = "rpki.validate";
    const IS_STREAMING: bool = false;

    type Params = RpkiValidateParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        // Validate prefix format
        params
            .prefix
            .parse::<ipnet::IpNet>()
            .map_err(|_| WsError::invalid_params(format!("Invalid prefix: {}", params.prefix)))?;
        Ok(())
    }

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // NOTE: `MonocleDatabase` / `RpkiRepository<'_>` are not `Send`.
        // `handle()` must produce a `Send` future, so we must not hold any DB-backed
        // values across an `.await`. Do all DB work first, then await only to send.
        let response = {
            // Open the database
            let db = MonocleDatabase::open_in_dir(&ctx.data_dir).map_err(|e| {
                WsError::operation_failed(format!("Failed to open database: {}", e))
            })?;

            let rpki_repo = db.rpki();

            // Check if RPKI data is available
            if rpki_repo.is_empty() {
                return Err(WsError::not_initialized("RPKI"));
            }

            // Perform validation (DB API expects &str)
            let (state, covering) = rpki_repo
                .validate(&params.prefix, params.asn)
                .map_err(|e| WsError::operation_failed(e.to_string()))?;

            // Build response
            let (state, reason) = match state {
                RpkiValidationState::Valid => (
                    ValidationState::Valid,
                    Some("ROA exists with matching ASN and valid prefix length".to_string()),
                ),
                RpkiValidationState::Invalid => {
                    (ValidationState::Invalid, Some("Invalid".to_string()))
                }
                RpkiValidationState::NotFound => (
                    ValidationState::NotFound,
                    Some("No covering ROA found".to_string()),
                ),
            };

            let covering_roas: Vec<CoveringRoa> = covering
                .into_iter()
                .map(|r: RpkiRoaRecord| CoveringRoa {
                    prefix: r.prefix,
                    max_length: r.max_length,
                    origin_asn: r.origin_asn,
                    ta: r.ta,
                })
                .collect();

            RpkiValidateResponse {
                validation: ValidationDetails {
                    prefix: params.prefix,
                    asn: params.asn,
                    state,
                    reason,
                },
                covering_roas,
            }
        };

        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// rpki.roas
// =============================================================================

/// Parameters for rpki.roas
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RpkiRoasParams {
    /// Filter by origin ASN
    #[serde(default)]
    pub asn: Option<u32>,

    /// Filter by prefix
    #[serde(default)]
    pub prefix: Option<String>,

    /// Historical date (YYYY-MM-DD format)
    #[serde(default)]
    pub date: Option<String>,

    /// Data source: cloudflare, ripe, rpkiviews
    #[serde(default)]
    pub source: Option<String>,
}

/// ROA entry in response
#[derive(Debug, Clone, Serialize)]
pub struct RoaEntry {
    /// ROA prefix
    pub prefix: String,

    /// Maximum prefix length
    pub max_length: u8,

    /// Origin ASN
    pub origin_asn: u32,

    /// Trust anchor
    pub ta: String,
}

impl From<RpkiRoaRecord> for RoaEntry {
    fn from(record: RpkiRoaRecord) -> Self {
        Self {
            prefix: record.prefix,
            max_length: record.max_length,
            origin_asn: record.origin_asn,
            ta: record.ta,
        }
    }
}

/// Response for rpki.roas
#[derive(Debug, Clone, Serialize)]
pub struct RpkiRoasResponse {
    /// List of ROAs
    pub roas: Vec<RoaEntry>,

    /// Total count
    pub count: usize,
}

/// Handler for rpki.roas method
pub struct RpkiRoasHandler;

#[async_trait]
impl WsMethod for RpkiRoasHandler {
    const METHOD: &'static str = "rpki.roas";
    const IS_STREAMING: bool = false;

    type Params = RpkiRoasParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        // Validate prefix if provided
        if let Some(ref prefix) = params.prefix {
            prefix
                .parse::<ipnet::IpNet>()
                .map_err(|_| WsError::invalid_params(format!("Invalid prefix: {}", prefix)))?;
        }

        // Validate date if provided
        if let Some(ref date) = params.date {
            NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
                WsError::invalid_params(format!("Invalid date format: {}. Use YYYY-MM-DD", date))
            })?;
        }

        // Validate source if provided
        if let Some(ref source) = params.source {
            match source.to_lowercase().as_str() {
                "cloudflare" | "ripe" | "rpkiviews" => {}
                _ => {
                    return Err(WsError::invalid_params(format!(
                        "Invalid source: {}. Use cloudflare, ripe, or rpkiviews",
                        source
                    )));
                }
            }
        }

        Ok(())
    }

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // NOTE: `MonocleDatabase` / `RpkiRepository<'_>` are not `Send`.
        // Do all DB work before any `.await`.
        let response = {
            // DB-first: query local database only.
            let db = MonocleDatabase::open_in_dir(&ctx.data_dir).map_err(|e| {
                WsError::operation_failed(format!("Failed to open database: {}", e))
            })?;

            let rpki_repo = db.rpki();

            // If the repo is empty, we treat this as not initialized.
            if rpki_repo.is_empty() {
                return Err(WsError::not_initialized("RPKI"));
            }

            // Parse date if provided (currently DB query does not support historical snapshots).
            // We validate earlier; here we fail if a date is explicitly requested to avoid silently
            // lying about historical results.
            if params.date.is_some() {
                return Err(WsError::invalid_params(
                    "Historical date filtering is not supported in DB-first mode yet",
                ));
            }

            // Optional prefix filter
            let prefix_filter: Option<ipnet::IpNet> = match params.prefix.as_deref() {
                Some(p) => Some(
                    p.parse::<ipnet::IpNet>()
                        .map_err(|_| WsError::invalid_params(format!("Invalid prefix: {}", p)))?,
                ),
                None => None,
            };

            // Collect ROAs from DB repo and apply filters locally.
            // Note: this keeps the handler DB-first (no network IO) even if filtering is in-memory.
            let mut roas = rpki_repo
                .get_all_roas()
                .map_err(|e| WsError::operation_failed(e.to_string()))?;

            if let Some(asn) = params.asn {
                roas.retain(|r| r.origin_asn == asn);
            }
            if let Some(prefix) = prefix_filter {
                roas.retain(|r| r.prefix == prefix.to_string());
            }

            let count = roas.len();
            let roa_entries: Vec<RoaEntry> = roas.into_iter().map(RoaEntry::from).collect();

            RpkiRoasResponse {
                roas: roa_entries,
                count,
            }
        };

        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// rpki.aspas
// =============================================================================

/// Parameters for rpki.aspas
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RpkiAspasParams {
    /// Filter by customer ASN
    #[serde(default)]
    pub customer_asn: Option<u32>,

    /// Filter by provider ASN
    #[serde(default)]
    pub provider_asn: Option<u32>,

    /// Historical date (YYYY-MM-DD format)
    #[serde(default)]
    pub date: Option<String>,

    /// Data source: cloudflare, ripe, rpkiviews
    #[serde(default)]
    pub source: Option<String>,
}

/// ASPA entry in response
#[derive(Debug, Clone, Serialize)]
pub struct AspaEntry {
    /// Customer ASN
    pub customer_asn: u32,

    /// Provider ASNs
    pub provider_asns: Vec<u32>,
}

impl From<RpkiAspaRecord> for AspaEntry {
    fn from(record: RpkiAspaRecord) -> Self {
        Self {
            customer_asn: record.customer_asn,
            provider_asns: record.provider_asns,
        }
    }
}

/// Response for rpki.aspas
#[derive(Debug, Clone, Serialize)]
pub struct RpkiAspasResponse {
    /// List of ASPAs
    pub aspas: Vec<AspaEntry>,

    /// Total count
    pub count: usize,
}

/// Handler for rpki.aspas method
pub struct RpkiAspasHandler;

#[async_trait]
impl WsMethod for RpkiAspasHandler {
    const METHOD: &'static str = "rpki.aspas";
    const IS_STREAMING: bool = false;

    type Params = RpkiAspasParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        // Validate date if provided
        if let Some(ref date) = params.date {
            NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| {
                WsError::invalid_params(format!("Invalid date format: {}. Use YYYY-MM-DD", date))
            })?;
        }

        // Validate source if provided
        if let Some(ref source) = params.source {
            match source.to_lowercase().as_str() {
                "cloudflare" | "ripe" | "rpkiviews" => {}
                _ => {
                    return Err(WsError::invalid_params(format!(
                        "Invalid source: {}. Use cloudflare, ripe, or rpkiviews",
                        source
                    )));
                }
            }
        }

        Ok(())
    }

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // NOTE: `MonocleDatabase` / `RpkiRepository<'_>` are not `Send`.
        // Do all DB work before any `.await`.
        let response = {
            // DB-first: query local database only.
            let db = MonocleDatabase::open_in_dir(&ctx.data_dir).map_err(|e| {
                WsError::operation_failed(format!("Failed to open database: {}", e))
            })?;

            let rpki_repo = db.rpki();

            // If the repo is empty, we treat this as not initialized.
            if rpki_repo.is_empty() {
                return Err(WsError::not_initialized("RPKI"));
            }

            // Parse date if provided (currently DB query does not support historical snapshots).
            // We validate earlier; here we fail if a date is explicitly requested to avoid silently
            // lying about historical results.
            if params.date.is_some() {
                return Err(WsError::invalid_params(
                    "Historical date filtering is not supported in DB-first mode yet",
                ));
            }

            // Collect ASPAs from DB repo and apply filters locally.
            let mut aspas = rpki_repo
                .get_all_aspas()
                .map_err(|e| WsError::operation_failed(e.to_string()))?;

            if let Some(customer) = params.customer_asn {
                aspas.retain(|a| a.customer_asn == customer);
            }
            if let Some(provider) = params.provider_asn {
                aspas.retain(|a| a.provider_asns.contains(&provider));
            }

            let count = aspas.len();
            let aspa_entries: Vec<AspaEntry> = aspas.into_iter().map(AspaEntry::from).collect();

            RpkiAspasResponse {
                aspas: aspa_entries,
                count,
            }
        };

        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpki_validate_params_deserialization() {
        let json = r#"{"prefix": "1.1.1.0/24", "asn": 13335}"#;
        let params: RpkiValidateParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.prefix, "1.1.1.0/24");
        assert_eq!(params.asn, 13335);
    }

    #[test]
    fn test_rpki_validate_params_validation() {
        // Valid params
        let params = RpkiValidateParams {
            prefix: "1.1.1.0/24".to_string(),
            asn: 13335,
        };
        assert!(RpkiValidateHandler::validate(&params).is_ok());

        // Invalid prefix
        let params = RpkiValidateParams {
            prefix: "not-a-prefix".to_string(),
            asn: 13335,
        };
        assert!(RpkiValidateHandler::validate(&params).is_err());
    }

    #[test]
    fn test_rpki_roas_params_default() {
        let params = RpkiRoasParams::default();
        assert!(params.asn.is_none());
        assert!(params.prefix.is_none());
        assert!(params.date.is_none());
        assert!(params.source.is_none());
    }

    #[test]
    fn test_rpki_roas_params_deserialization() {
        let json = r#"{"asn": 13335, "source": "cloudflare"}"#;
        let params: RpkiRoasParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.asn, Some(13335));
        assert_eq!(params.source, Some("cloudflare".to_string()));
    }

    #[test]
    fn test_rpki_roas_params_validation() {
        // Valid params
        let params = RpkiRoasParams {
            asn: Some(13335),
            prefix: Some("1.1.1.0/24".to_string()),
            date: Some("2024-01-01".to_string()),
            source: Some("cloudflare".to_string()),
        };
        assert!(RpkiRoasHandler::validate(&params).is_ok());

        // Invalid prefix
        let params = RpkiRoasParams {
            prefix: Some("invalid".to_string()),
            ..Default::default()
        };
        assert!(RpkiRoasHandler::validate(&params).is_err());

        // Invalid date
        let params = RpkiRoasParams {
            date: Some("not-a-date".to_string()),
            ..Default::default()
        };
        assert!(RpkiRoasHandler::validate(&params).is_err());

        // Invalid source
        let params = RpkiRoasParams {
            source: Some("invalid-source".to_string()),
            ..Default::default()
        };
        assert!(RpkiRoasHandler::validate(&params).is_err());
    }

    #[test]
    fn test_rpki_aspas_params_default() {
        let params = RpkiAspasParams::default();
        assert!(params.customer_asn.is_none());
        assert!(params.provider_asn.is_none());
    }

    #[test]
    fn test_rpki_aspas_params_deserialization() {
        let json = r#"{"customer_asn": 13335}"#;
        let params: RpkiAspasParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.customer_asn, Some(13335));
        assert!(params.provider_asn.is_none());
    }

    #[test]
    fn test_validation_state_serialization() {
        let state = ValidationState::Valid;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"valid\"");

        let state = ValidationState::Invalid;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"invalid\"");

        let state = ValidationState::NotFound;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"notfound\"");
    }

    #[test]
    fn test_rpki_validate_response_serialization() {
        let response = RpkiValidateResponse {
            validation: ValidationDetails {
                prefix: "1.1.1.0/24".to_string(),
                asn: 13335,
                state: ValidationState::Valid,
                reason: Some("ROA exists".to_string()),
            },
            covering_roas: vec![CoveringRoa {
                prefix: "1.1.1.0/24".to_string(),
                max_length: 24,
                origin_asn: 13335,
                ta: "APNIC".to_string(),
            }],
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"state\":\"valid\""));
        assert!(json.contains("\"prefix\":\"1.1.1.0/24\""));
        assert!(json.contains("\"asn\":13335"));
    }
}
