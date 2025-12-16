//! Pfx2as handlers for prefix-to-ASN lookup operations
//!
//! This module provides handlers for Pfx2as-related methods like `pfx2as.lookup`.
//!
//! The handler uses the SQLite-based Pfx2as repository for efficient prefix queries,
//! with fallback to the file-based cache for backward compatibility.

use crate::database::{MonocleDatabase, Pfx2asFileCache, Pfx2asRecord};
use crate::lens::pfx2as::{Pfx2asLens, Pfx2asLookupArgs, Pfx2asLookupMode};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// =============================================================================
// pfx2as.lookup
// =============================================================================

/// Parameters for pfx2as.lookup
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Pfx2asLookupParams {
    /// IP prefix to look up
    pub prefix: String,

    /// Lookup mode: "exact", "longest", "covering", or "covered" (default: longest)
    #[serde(default)]
    pub mode: Option<String>,
}

/// Response for pfx2as.lookup
#[derive(Debug, Clone, Serialize)]
pub struct Pfx2asLookupResponse {
    /// The queried prefix (or matched prefix for single-result modes)
    pub prefix: String,

    /// Origin ASNs for the prefix
    pub asns: Vec<u32>,

    /// Match type (exact, longest, covering, covered)
    pub match_type: String,

    /// Additional results for covering/covered modes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Vec<Pfx2asMatchResult>>,
}

/// A single match result for covering/covered queries
#[derive(Debug, Clone, Serialize)]
pub struct Pfx2asMatchResult {
    /// The matched prefix
    pub prefix: String,
    /// Origin ASNs for this prefix
    pub asns: Vec<u32>,
}

/// Handler for pfx2as.lookup method
pub struct Pfx2asLookupHandler;

#[async_trait]
impl WsMethod for Pfx2asLookupHandler {
    const METHOD: &'static str = "pfx2as.lookup";
    const IS_STREAMING: bool = false;

    type Params = Pfx2asLookupParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        // Validate prefix format
        params
            .prefix
            .parse::<ipnet::IpNet>()
            .map_err(|_| WsError::invalid_params(format!("Invalid prefix: {}", params.prefix)))?;

        // Validate mode if provided
        if let Some(ref mode) = params.mode {
            match mode.to_lowercase().as_str() {
                "exact" | "longest" | "covering" | "covered" => {}
                _ => {
                    return Err(WsError::invalid_params(format!(
                        "Invalid mode: {}. Use 'exact', 'longest', 'covering', or 'covered'",
                        mode
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
        // DB-first policy: Try SQLite repository first, then fall back to file cache
        let mode_str = params.mode.as_deref().unwrap_or("longest").to_lowercase();

        // Try SQLite-based repository first (do all DB work before any await)
        let sqlite_response: Option<Pfx2asLookupResponse> = {
            let db_result = MonocleDatabase::open_in_dir(&ctx.data_dir);

            if let Ok(db) = db_result {
                let repo = db.pfx2as();

                // Check if SQLite has data
                if !repo.is_empty() {
                    let response = match mode_str.as_str() {
                        "exact" => {
                            let asns = repo
                                .lookup_exact(&params.prefix)
                                .map_err(|e| WsError::operation_failed(e.to_string()))?;

                            Some(Pfx2asLookupResponse {
                                prefix: params.prefix.clone(),
                                asns,
                                match_type: "exact".to_string(),
                                results: None,
                            })
                        }
                        "longest" => {
                            let result = repo
                                .lookup_longest(&params.prefix)
                                .map_err(|e| WsError::operation_failed(e.to_string()))?;

                            Some(Pfx2asLookupResponse {
                                prefix: result.prefix,
                                asns: result.origin_asns,
                                match_type: "longest".to_string(),
                                results: None,
                            })
                        }
                        "covering" => {
                            let results = repo
                                .lookup_covering(&params.prefix)
                                .map_err(|e| WsError::operation_failed(e.to_string()))?;

                            let match_results: Vec<Pfx2asMatchResult> = results
                                .into_iter()
                                .map(|r| Pfx2asMatchResult {
                                    prefix: r.prefix,
                                    asns: r.origin_asns,
                                })
                                .collect();

                            Some(Pfx2asLookupResponse {
                                prefix: params.prefix.clone(),
                                asns: vec![],
                                match_type: "covering".to_string(),
                                results: Some(match_results),
                            })
                        }
                        "covered" => {
                            let results = repo
                                .lookup_covered(&params.prefix)
                                .map_err(|e| WsError::operation_failed(e.to_string()))?;

                            let match_results: Vec<Pfx2asMatchResult> = results
                                .into_iter()
                                .map(|r| Pfx2asMatchResult {
                                    prefix: r.prefix,
                                    asns: r.origin_asns,
                                })
                                .collect();

                            Some(Pfx2asLookupResponse {
                                prefix: params.prefix.clone(),
                                asns: vec![],
                                match_type: "covered".to_string(),
                                results: Some(match_results),
                            })
                        }
                        _ => {
                            return Err(WsError::invalid_params(format!(
                                "Unknown mode: {}",
                                mode_str
                            )));
                        }
                    };
                    response
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Send result if we got one from SQLite (await is now safe - no DB references held)
        if let Some(response) = sqlite_response {
            sink.send_result(response)
                .await
                .map_err(|e| WsError::internal(e.to_string()))?;

            return Ok(());
        }

        // Fall back to file-based cache for backward compatibility
        let cache = Pfx2asFileCache::new(&ctx.data_dir)
            .map_err(|e| WsError::internal(format!("Failed to access pfx2as cache: {}", e)))?;

        // Default BGPKIT source string used by CLI and lens docs.
        let source = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";

        let cached = match cache.load(source) {
            Ok(data) => data,
            Err(_) => {
                return Err(WsError::not_initialized(
                    "pfx2as cache (run database.refresh source=pfx2as-cache first)",
                ));
            }
        };

        // Convert cached records into the lens's record type
        let mut prefix_map: HashMap<String, HashSet<u32>> = HashMap::new();
        for rec in cached.records {
            prefix_map
                .entry(rec.prefix.clone())
                .or_default()
                .extend(rec.origin_asns.into_iter());
        }

        let records: Vec<Pfx2asRecord> = prefix_map
            .into_iter()
            .map(|(prefix, asns)| Pfx2asRecord {
                prefix,
                origin_asns: asns.into_iter().collect(),
            })
            .collect();

        let lens = Pfx2asLens::from_records(records)
            .map_err(|e| WsError::internal(format!("Failed to build pfx2as trie: {}", e)))?;

        // Parse mode (file cache only supports exact and longest)
        let mode = match mode_str.as_str() {
            "exact" => Pfx2asLookupMode::Exact,
            "longest" => Pfx2asLookupMode::Longest,
            "covering" | "covered" => {
                return Err(WsError::invalid_params(
                    "covering/covered modes require SQLite cache. Run database.refresh source=pfx2as-cache to populate it.",
                ));
            }
            _ => Pfx2asLookupMode::Longest,
        };

        // Build lookup args
        let args = Pfx2asLookupArgs::new(&params.prefix).with_mode(mode.clone());

        // Perform the lookup
        let asns = lens
            .lookup(&args)
            .map_err(|e| WsError::operation_failed(e.to_string()))?;

        // Build response
        let response = Pfx2asLookupResponse {
            prefix: params.prefix,
            asns,
            match_type: match mode {
                Pfx2asLookupMode::Exact => "exact".to_string(),
                Pfx2asLookupMode::Longest => "longest".to_string(),
            },
            results: None,
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
    fn test_pfx2as_lookup_params_deserialization() {
        let json = r#"{"prefix": "1.1.1.0/24"}"#;
        let params: Pfx2asLookupParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.prefix, "1.1.1.0/24");
        assert!(params.mode.is_none());

        let json = r#"{"prefix": "8.8.8.0/24", "mode": "exact"}"#;
        let params: Pfx2asLookupParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.prefix, "8.8.8.0/24");
        assert_eq!(params.mode, Some("exact".to_string()));
    }

    #[test]
    fn test_pfx2as_lookup_params_validation() {
        // Valid params
        let params = Pfx2asLookupParams {
            prefix: "1.1.1.0/24".to_string(),
            mode: None,
        };
        assert!(Pfx2asLookupHandler::validate(&params).is_ok());

        // Valid with mode
        let params = Pfx2asLookupParams {
            prefix: "1.1.1.0/24".to_string(),
            mode: Some("exact".to_string()),
        };
        assert!(Pfx2asLookupHandler::validate(&params).is_ok());

        let params = Pfx2asLookupParams {
            prefix: "1.1.1.0/24".to_string(),
            mode: Some("longest".to_string()),
        };
        assert!(Pfx2asLookupHandler::validate(&params).is_ok());

        // Valid with covering/covered modes
        let params = Pfx2asLookupParams {
            prefix: "1.1.1.0/24".to_string(),
            mode: Some("covering".to_string()),
        };
        assert!(Pfx2asLookupHandler::validate(&params).is_ok());

        let params = Pfx2asLookupParams {
            prefix: "1.1.1.0/24".to_string(),
            mode: Some("covered".to_string()),
        };
        assert!(Pfx2asLookupHandler::validate(&params).is_ok());

        // Invalid prefix
        let params = Pfx2asLookupParams {
            prefix: "not-a-prefix".to_string(),
            mode: None,
        };
        assert!(Pfx2asLookupHandler::validate(&params).is_err());

        // Invalid mode
        let params = Pfx2asLookupParams {
            prefix: "1.1.1.0/24".to_string(),
            mode: Some("invalid".to_string()),
        };
        assert!(Pfx2asLookupHandler::validate(&params).is_err());
    }

    #[test]
    fn test_pfx2as_lookup_response_serialization() {
        let response = Pfx2asLookupResponse {
            prefix: "1.1.1.0/24".to_string(),
            asns: vec![13335],
            match_type: "exact".to_string(),
            results: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"prefix\":\"1.1.1.0/24\""));
        assert!(json.contains("\"asns\":[13335]"));
        assert!(json.contains("\"match_type\":\"exact\""));
        // results should not appear when None
        assert!(!json.contains("\"results\""));
    }

    #[test]
    fn test_pfx2as_lookup_response_multiple_asns() {
        let response = Pfx2asLookupResponse {
            prefix: "192.0.2.0/24".to_string(),
            asns: vec![64496, 64497, 64498],
            match_type: "longest".to_string(),
            results: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("[64496,64497,64498]"));
    }

    #[test]
    fn test_pfx2as_lookup_response_empty_asns() {
        let response = Pfx2asLookupResponse {
            prefix: "10.0.0.0/8".to_string(),
            asns: vec![],
            match_type: "exact".to_string(),
            results: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"asns\":[]"));
    }

    #[test]
    fn test_pfx2as_lookup_response_with_results() {
        let response = Pfx2asLookupResponse {
            prefix: "1.0.0.0/8".to_string(),
            asns: vec![],
            match_type: "covering".to_string(),
            results: Some(vec![
                Pfx2asMatchResult {
                    prefix: "1.0.0.0/8".to_string(),
                    asns: vec![1000],
                },
                Pfx2asMatchResult {
                    prefix: "1.1.0.0/16".to_string(),
                    asns: vec![1100],
                },
            ]),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"results\""));
        assert!(json.contains("\"1.0.0.0/8\""));
        assert!(json.contains("\"1.1.0.0/16\""));
    }
}
