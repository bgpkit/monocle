//! Database handlers for database management operations
//!
//! This module provides handlers for database-related methods like `database.status`
//! and `database.refresh`.

use crate::config::DataSourceStatus;
use crate::database::{MonocleDatabase, Pfx2asDbRecord};
use crate::lens::pfx2as::Pfx2asEntry;
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

// =============================================================================
// database.status
// =============================================================================

/// Parameters for database.status (empty)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DatabaseStatusParams {}

/// SQLite database info in response
#[derive(Debug, Clone, Serialize)]
pub struct SqliteInfo {
    /// Database path
    pub path: String,

    /// Whether the database file exists
    pub exists: bool,

    /// Database file size in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,

    /// AS2Rel record count
    pub as2rel_count: u64,

    /// RPKI ROA record count
    pub rpki_roa_count: u64,

    /// Pfx2as record count
    pub pfx2as_count: u64,
}

/// Source status info
#[derive(Debug, Clone, Serialize)]
pub struct SourceInfo {
    /// Current state
    pub state: String,

    /// Last updated timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,

    /// Next refresh after timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_refresh_after: Option<String>,
}

/// Sources status
#[derive(Debug, Clone, Serialize)]
pub struct SourcesInfo {
    /// RPKI source status
    pub rpki: SourceInfo,

    /// AS2Rel source status
    pub as2rel: SourceInfo,

    /// Pfx2as source status
    pub pfx2as: SourceInfo,
}

/// Cache info
#[derive(Debug, Clone, Serialize)]
pub struct CacheInfoResponse {
    /// Cache directory
    pub directory: String,

    /// Pfx2as cache entry count
    pub pfx2as_cache_count: usize,
}

/// Response for database.status
#[derive(Debug, Clone, Serialize)]
pub struct DatabaseStatusResponse {
    /// SQLite database info
    pub sqlite: SqliteInfo,

    /// Data sources status
    pub sources: SourcesInfo,

    /// Cache info
    pub cache: CacheInfoResponse,
}

/// Handler for database.status method
pub struct DatabaseStatusHandler;

#[async_trait]
impl WsMethod for DatabaseStatusHandler {
    const METHOD: &'static str = "database.status";
    const IS_STREAMING: bool = false;

    type Params = DatabaseStatusParams;

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        _params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // Build paths
        let sqlite_path = format!("{}/monocle.db", ctx.data_dir);
        let cache_dir = format!("{}/cache", ctx.data_dir);
        let sqlite_exists = Path::new(&sqlite_path).exists();
        let cache_exists = Path::new(&cache_dir).exists();

        // Get SQLite size if exists
        let sqlite_size = if sqlite_exists {
            std::fs::metadata(&sqlite_path).ok().map(|m| m.len())
        } else {
            None
        };

        // Open database to get counts
        let (as2rel_count, rpki_roa_count, pfx2as_count, as2rel_status, rpki_status, pfx2as_status) =
            if sqlite_exists {
                match MonocleDatabase::open_in_dir(&ctx.data_dir) {
                    Ok(db) => {
                        let as2rel = db.as2rel().count().unwrap_or(0);
                        let rpki_roa = db.rpki().roa_count().unwrap_or(0);
                        let pfx2as = db.pfx2as().record_count().unwrap_or(0);

                        let as2rel_status = if as2rel > 0 {
                            DataSourceStatus::Ready
                        } else {
                            DataSourceStatus::Empty
                        };
                        let rpki_status = if rpki_roa > 0 {
                            DataSourceStatus::Ready
                        } else {
                            DataSourceStatus::Empty
                        };
                        let pfx2as_status = if pfx2as > 0 {
                            DataSourceStatus::Ready
                        } else {
                            DataSourceStatus::Empty
                        };

                        (
                            as2rel,
                            rpki_roa,
                            pfx2as,
                            as2rel_status,
                            rpki_status,
                            pfx2as_status,
                        )
                    }
                    Err(_) => (
                        0,
                        0,
                        0,
                        DataSourceStatus::NotInitialized,
                        DataSourceStatus::NotInitialized,
                        DataSourceStatus::NotInitialized,
                    ),
                }
            } else {
                (
                    0,
                    0,
                    0,
                    DataSourceStatus::NotInitialized,
                    DataSourceStatus::NotInitialized,
                    DataSourceStatus::NotInitialized,
                )
            };

        // Count pfx2as cache files
        let pfx2as_cache_count = if cache_exists {
            std::fs::read_dir(&cache_dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_name().to_string_lossy().starts_with("pfx2as_"))
                        .count()
                })
                .unwrap_or(0)
        } else {
            0
        };

        let status_to_string = |status: &DataSourceStatus| -> String {
            match status {
                DataSourceStatus::Ready => "ready".to_string(),
                DataSourceStatus::Empty => "empty".to_string(),
                DataSourceStatus::NotInitialized => "absent".to_string(),
            }
        };

        let response = DatabaseStatusResponse {
            sqlite: SqliteInfo {
                path: sqlite_path,
                exists: sqlite_exists,
                size_bytes: sqlite_size,
                as2rel_count,
                rpki_roa_count,
                pfx2as_count,
            },
            sources: SourcesInfo {
                rpki: SourceInfo {
                    state: status_to_string(&rpki_status),
                    last_updated: None,
                    next_refresh_after: None,
                },
                as2rel: SourceInfo {
                    state: status_to_string(&as2rel_status),
                    last_updated: None,
                    next_refresh_after: None,
                },
                pfx2as: SourceInfo {
                    state: status_to_string(&pfx2as_status),
                    last_updated: None,
                    next_refresh_after: None,
                },
            },
            cache: CacheInfoResponse {
                directory: cache_dir,
                pfx2as_cache_count,
            },
        };

        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// database.refresh
// =============================================================================

/// Parameters for database.refresh
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseRefreshParams {
    /// Source to refresh: "rpki", "as2org", "as2rel", or "all"
    pub source: String,

    /// Force refresh even if data is fresh
    #[serde(default)]
    pub force: Option<bool>,
}

/// Response for database.refresh
#[derive(Debug, Clone, Serialize)]
pub struct DatabaseRefreshResponse {
    /// Whether refresh was performed
    pub refreshed: bool,

    /// Source that was refreshed
    pub source: String,

    /// Message
    pub message: String,

    /// Number of records (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
}

/// Handler for database.refresh method
pub struct DatabaseRefreshHandler;

#[async_trait]
impl WsMethod for DatabaseRefreshHandler {
    const METHOD: &'static str = "database.refresh";
    const IS_STREAMING: bool = false; // Could be streaming for progress

    type Params = DatabaseRefreshParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        match params.source.to_lowercase().as_str() {
            "rpki" | "as2rel" | "pfx2as" | "all" => Ok(()),
            _ => Err(WsError::invalid_params(format!(
                "Invalid source: {}. Use 'rpki', 'as2rel', 'pfx2as', or 'all'",
                params.source
            ))),
        }
    }

    async fn handle(
        ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        let source = params.source.to_lowercase();
        let _force = params.force.unwrap_or(false);

        // Open the database
        let db = MonocleDatabase::open_in_dir(&ctx.data_dir)
            .map_err(|e| WsError::operation_failed(format!("Failed to open database: {}", e)))?;

        let (message, count) = match source.as_str() {
            "as2rel" => {
                let count = db.update_as2rel().map_err(|e| {
                    WsError::operation_failed(format!("AS2Rel refresh failed: {}", e))
                })?;
                (
                    format!("Successfully refreshed AS2Rel data with {} entries", count),
                    Some(count),
                )
            }
            "rpki" => {
                // RPKI refresh would need to use the RPKI repository
                // For now, return a placeholder
                let rpki_repo = db.rpki();
                let count = rpki_repo.roa_count().unwrap_or(0);
                let count_usize = usize::try_from(count).unwrap_or(usize::MAX);
                (
                    format!(
                        "RPKI data has {} ROA entries (use bgpkit-commons for refresh)",
                        count
                    ),
                    Some(count_usize),
                )
            }
            "pfx2as" => {
                // Fetch pfx2as data from BGPKIT and store in SQLite
                let url = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";

                let entries: Vec<Pfx2asEntry> = oneio::read_json_struct(url).map_err(|e| {
                    WsError::operation_failed(format!("Failed to fetch pfx2as data: {}", e))
                })?;

                // Convert to database records
                let records: Vec<Pfx2asDbRecord> = entries
                    .into_iter()
                    .map(|e| Pfx2asDbRecord {
                        prefix: e.prefix,
                        origin_asn: e.asn,
                        validation: "unknown".to_string(),
                    })
                    .collect();

                let count = records.len();

                // Store in SQLite
                db.pfx2as().store(&records, url).map_err(|e| {
                    WsError::operation_failed(format!("Failed to store pfx2as data: {}", e))
                })?;

                (
                    format!("Successfully refreshed pfx2as data with {} records", count),
                    Some(count),
                )
            }
            "all" => {
                // Refresh all sources
                let mut messages = Vec::new();

                // AS2Rel
                match db.update_as2rel() {
                    Ok(count) => messages.push(format!("AS2Rel: {} entries", count)),
                    Err(e) => messages.push(format!("AS2Rel: failed - {}", e)),
                }

                // Pfx2as
                let pfx2as_url = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";
                match oneio::read_json_struct::<Vec<Pfx2asEntry>>(pfx2as_url) {
                    Ok(entries) => {
                        let records: Vec<Pfx2asDbRecord> = entries
                            .into_iter()
                            .map(|e| Pfx2asDbRecord {
                                prefix: e.prefix,
                                origin_asn: e.asn,
                                validation: "unknown".to_string(),
                            })
                            .collect();
                        let count = records.len();
                        match db.pfx2as().store(&records, pfx2as_url) {
                            Ok(()) => messages.push(format!("Pfx2as: {} entries", count)),
                            Err(e) => messages.push(format!("Pfx2as: store failed - {}", e)),
                        }
                    }
                    Err(e) => messages.push(format!("Pfx2as: fetch failed - {}", e)),
                }

                (messages.join("; "), None)
            }
            _ => {
                return Err(WsError::invalid_params(format!(
                    "Unknown source: {}",
                    source
                )));
            }
        };

        let response = DatabaseRefreshResponse {
            refreshed: true,
            source: params.source,
            message,
            count,
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
    fn test_database_status_params_default() {
        let params = DatabaseStatusParams::default();
        let json = serde_json::to_string(&params).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_database_refresh_params_deserialization() {
        let json = r#"{"source": "rpki"}"#;
        let params: DatabaseRefreshParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.source, "rpki");
        assert!(params.force.is_none());

        let json = r#"{"source": "as2rel", "force": true}"#;
        let params: DatabaseRefreshParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.source, "as2rel");
        assert_eq!(params.force, Some(true));
    }

    #[test]
    fn test_database_refresh_params_validation() {
        // Valid sources
        for source in &["rpki", "as2rel", "pfx2as", "all"] {
            let params = DatabaseRefreshParams {
                source: source.to_string(),
                force: None,
            };
            assert!(DatabaseRefreshHandler::validate(&params).is_ok());
        }

        // Invalid source
        let params = DatabaseRefreshParams {
            source: "invalid".to_string(),
            force: None,
        };
        assert!(DatabaseRefreshHandler::validate(&params).is_err());
    }

    #[test]
    fn test_database_status_response_serialization() {
        let response = DatabaseStatusResponse {
            sqlite: SqliteInfo {
                path: "/path/to/monocle.db".to_string(),
                exists: true,
                size_bytes: Some(1024),
                as2rel_count: 200,
                rpki_roa_count: 300,
                pfx2as_count: 400,
            },
            sources: SourcesInfo {
                rpki: SourceInfo {
                    state: "ready".to_string(),
                    last_updated: Some("2024-01-01T00:00:00Z".to_string()),
                    next_refresh_after: None,
                },
                as2rel: SourceInfo {
                    state: "empty".to_string(),
                    last_updated: None,
                    next_refresh_after: None,
                },
                pfx2as: SourceInfo {
                    state: "ready".to_string(),
                    last_updated: Some("2024-01-01T00:00:00Z".to_string()),
                    next_refresh_after: None,
                },
            },
            cache: CacheInfoResponse {
                directory: "/path/to/cache".to_string(),
                pfx2as_cache_count: 5,
            },
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"exists\":true"));
        assert!(json.contains("\"as2rel_count\":200"));
        assert!(json.contains("\"state\":\"ready\""));
    }

    #[test]
    fn test_database_refresh_response_serialization() {
        let response = DatabaseRefreshResponse {
            refreshed: true,
            source: "as2rel".to_string(),
            message: "Successfully refreshed".to_string(),
            count: Some(100),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"refreshed\":true"));
        assert!(json.contains("\"source\":\"as2rel\""));
        assert!(json.contains("\"count\":100"));

        // Without count
        let response = DatabaseRefreshResponse {
            refreshed: true,
            source: "all".to_string(),
            message: "Refreshed all sources".to_string(),
            count: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("count")); // Should be skipped
    }
}
