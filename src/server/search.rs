//! SSE search streaming handler.
//!
//! `POST /api/v1/search/stream` accepts a JSON request body and returns a
//! `text/event-stream` response. The server runs a sequential, cancellable
//! search loop in a blocking task, streaming progress and element batch
//! events to the client. Cancellation is triggered by closing the HTTP
//! connection — when the SSE response is dropped, the cancellation flag is
//! set and the worker stops.
//!
//! See `SSE_SERVICE_OVERHAUL_DESIGN.md` Section 6 for the algorithm.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::response::sse::{Event as SseEvent, Sse};
use axum::Json;
use bgpkit_parser::BgpElem;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::lens::search::{SearchFilters, SearchProgress, SearchSummary};
use crate::server::http::{ApiError, ApiErrorCode, ApiErrorResponse};
use crate::server::ServerState;

// =============================================================================
// Wire DTOs
// =============================================================================

/// Request body for `POST /api/v1/search/stream`.
///
/// This is an independent wire DTO — not `monocle::lens::search::SearchFilters`.
/// It maps to `SearchFilters` internally so internal refactoring does not break
/// the API contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchStreamRequest {
    pub filters: SearchStreamFilters,
    /// Elements per SSE batch (clamped to server max)
    #[serde(default)]
    pub batch_size: Option<usize>,
    /// Maximum total results (0 or None = unlimited, clamped to server max)
    #[serde(default)]
    pub max_results: Option<u64>,
}

/// Wire-level filters mirroring `SearchFilters` + `ParseFilters` field names.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchStreamFilters {
    #[serde(default)]
    pub prefix: Vec<String>,
    #[serde(default)]
    pub include_super: bool,
    #[serde(default)]
    pub include_sub: bool,
    #[serde(default)]
    pub origin_asn: Vec<String>,
    #[serde(default)]
    pub peer_asn: Vec<String>,
    #[serde(default)]
    pub peer_ip: Vec<String>,
    #[serde(default)]
    pub communities: Vec<String>,
    #[serde(default)]
    pub elem_type: Option<String>,
    #[serde(default)]
    pub as_path: Option<String>,
    /// Start timestamp (unix or human-readable). Required.
    pub start_ts: String,
    /// End timestamp (unix or human-readable). Required.
    pub end_ts: String,
    #[serde(default)]
    pub collector: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub dump_type: Option<String>,
}

impl TryFrom<SearchStreamFilters> for SearchFilters {
    type Error = anyhow::Error;

    fn try_from(f: SearchStreamFilters) -> Result<Self, Self::Error> {
        use crate::lens::parse::ParseFilters;
        use crate::lens::search::SearchDumpType;

        let dump_type = match f.dump_type.as_deref() {
            None | Some("updates") => SearchDumpType::Updates,
            Some("rib") => SearchDumpType::Rib,
            Some("rib_updates") | Some("all") => SearchDumpType::RibUpdates,
            Some(other) => {
                anyhow::bail!(
                    "invalid dump_type '{}': expected 'updates', 'rib', or 'rib_updates'",
                    other
                )
            }
        };

        let parse_filters = ParseFilters {
            origin_asn: f.origin_asn,
            prefix: f.prefix,
            include_super: f.include_super,
            include_sub: f.include_sub,
            peer_ip: f
                .peer_ip
                .into_iter()
                .filter_map(|s| s.parse().ok())
                .collect(),
            peer_asn: f.peer_asn,
            communities: f.communities,
            elem_type: None, // TODO: parse elem_type string → ParseElemType
            start_ts: Some(f.start_ts),
            end_ts: Some(f.end_ts),
            duration: None,
            as_path: f.as_path,
        };

        Ok(SearchFilters {
            parse_filters,
            collector: f.collector,
            project: f.project,
            dump_type,
        })
    }
}

// =============================================================================
// SSE Event Types
// =============================================================================

/// Metadata sent in the `started` event.
#[derive(Debug, Clone, Serialize)]
pub struct SearchStarted {
    pub batch_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

/// A batch of elements sent in an `elements` event.
///
/// Uses `bgpkit_parser::BgpElem` directly — it already derives `Serialize`
/// with the `serde` feature. A dedicated `ApiBgpElem` can be introduced later
/// if the wire contract needs to diverge from the parser's model.
#[derive(Debug, Clone, Serialize)]
pub struct ElementsBatch {
    pub total_so_far: u64,
    pub collector: Option<String>,
    pub elements: Vec<BgpElem>,
}

/// Internal enum representing SSE events. Each variant maps to an SSE `event:`
/// name; the `data:` field is the variant's payload serialized as JSON.
enum SearchStreamEvent {
    Started(SearchStarted),
    Progress(SearchProgress),
    Elements(ElementsBatch),
    Completed(SearchSummary),
    Cancelled,
    Error(ApiErrorResponse),
}

impl SearchStreamEvent {
    /// Map to an Axum SSE `Event` with the correct `event:` name and JSON data.
    fn to_sse(&self) -> Option<SseEvent> {
        let (event_name, json_data) = match self {
            SearchStreamEvent::Started(data) => ("started", serde_json::to_value(data).ok()?),
            SearchStreamEvent::Progress(data) => ("progress", serde_json::to_value(data).ok()?),
            SearchStreamEvent::Elements(data) => ("elements", serde_json::to_value(data).ok()?),
            SearchStreamEvent::Completed(data) => ("completed", serde_json::to_value(data).ok()?),
            SearchStreamEvent::Cancelled => ("cancelled", serde_json::Value::Null),
            SearchStreamEvent::Error(data) => ("error", serde_json::to_value(data).ok()?),
        };

        SseEvent::default()
            .event(event_name)
            .data(json_data.to_string())
            .into()
    }
}

// =============================================================================
// Handler
// =============================================================================

/// `POST /api/v1/search/stream`
///
/// Validates the request, converts the wire DTO to `SearchFilters`, spawns a
/// blocking sequential search worker, and returns an SSE stream. When the
/// HTTP response is dropped (client disconnect), the cancellation flag is set
/// and the worker stops.
pub async fn stream_search(
    State(state): State<ServerState>,
    Json(request): Json<SearchStreamRequest>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>>, ApiError> {
    // 1. Convert wire DTO to internal SearchFilters
    let filters: SearchFilters = request
        .filters
        .try_into()
        .map_err(|e: anyhow::Error| ApiError::invalid_params(e.to_string()))?;

    // 2. Validate filters (time range parse, etc.)
    filters
        .validate()
        .map_err(|e| ApiError::invalid_params(e.to_string()))?;

    // 3. Clamp batch_size and max_results to server-configured limits
    let config = &state.config;
    let batch_size = request
        .batch_size
        .unwrap_or(config.server_max_search_batch_size)
        .min(config.server_max_search_batch_size)
        .max(1);

    let max_results = match (request.max_results, config.server_max_search_results) {
        (Some(r), 0) => Some(r),                // server unlimited
        (Some(r), limit) => Some(r.min(limit)), // clamp to server limit
        (None, 0) => None,                      // both unlimited
        (None, limit) => Some(limit),           // server limit
    };

    let timeout_secs = if config.server_search_timeout_secs > 0 {
        Some(config.server_search_timeout_secs)
    } else {
        None
    };

    // 4. Create bounded channel and cancellation flag
    let (tx, rx) = mpsc::channel::<SearchStreamEvent>(32);
    let cancel_flag = Arc::new(AtomicBool::new(false));

    // 5. Spawn the search worker in a blocking task
    let worker_cancel_flag = cancel_flag.clone();
    let worker_tx = tx.clone();

    tokio::task::spawn_blocking(move || {
        run_search_worker(
            filters,
            batch_size,
            max_results,
            timeout_secs,
            worker_cancel_flag,
            worker_tx,
        );
    });

    // 6. Build SSE stream from the channel receiver.
    // When the client disconnects, the Sse response is dropped, which drops
    // `rx`, which causes `tx.send()` to fail in the worker, which sets
    // `cancel_flag`. The worker checks `cancel_flag` and stops.
    drop(tx); // drop the original sender; worker has its own clone

    let stream = ReceiverStream::new(rx).map(|event| {
        let sse_event = event.to_sse().unwrap_or_else(|| {
            SseEvent::default().event("error").data(
                serde_json::to_string(&ApiErrorResponse::new(
                    ApiErrorCode::InternalError,
                    "failed to serialize event",
                ))
                .unwrap_or_else(|_| "{}".to_string()),
            )
        });
        Ok::<_, std::convert::Infallible>(sse_event)
    });

    // Wrap in CancellableStream so that dropping the response sets cancel_flag
    let stream = CancellableStream::new(stream, cancel_flag);

    Ok(Sse::new(stream))
}

// =============================================================================
// Search Worker (runs in spawn_blocking)
// =============================================================================

fn run_search_worker(
    filters: SearchFilters,
    batch_size: usize,
    max_results: Option<u64>,
    timeout_secs: Option<u64>,
    cancel_flag: Arc<AtomicBool>,
    event_tx: mpsc::Sender<SearchStreamEvent>,
) {
    // Send Started event
    let _ = send_event(
        &event_tx,
        SearchStreamEvent::Started(SearchStarted {
            batch_size,
            max_results,
            timeout_secs,
        }),
    );

    let start_time = Instant::now();
    let deadline = timeout_secs.map(|s| start_time + Duration::from_secs(s));

    let is_cancelled = || cancel_flag.load(Ordering::Relaxed);

    // 1. Query broker for files
    let _ = send_event(
        &event_tx,
        SearchStreamEvent::Progress(SearchProgress::QueryingBroker),
    );

    let items = match filters.to_broker_items() {
        Ok(items) => items,
        Err(e) => {
            let _ = send_event(
                &event_tx,
                SearchStreamEvent::Error(ApiErrorResponse::new(
                    ApiErrorCode::SearchFailed,
                    e.to_string(),
                )),
            );
            return;
        }
    };

    let total_files = items.len();
    let _ = send_event(
        &event_tx,
        SearchStreamEvent::Progress(SearchProgress::FilesFound { count: total_files }),
    );

    if total_files == 0 {
        let _ = send_event(
            &event_tx,
            SearchStreamEvent::Completed(SearchSummary {
                total_files: 0,
                successful_files: 0,
                failed_files: 0,
                total_messages: 0,
                duration_secs: start_time.elapsed().as_secs_f64(),
            }),
        );
        return;
    }

    let mut successful_files: usize = 0;
    let mut failed_files: usize = 0;
    let mut total_messages: u64 = 0;
    let mut total_elements_sent: u64 = 0;

    // Process files sequentially
    for (index, item) in items.into_iter().enumerate() {
        if is_cancelled() {
            break;
        }

        // Check timeout
        if let Some(dl) = deadline {
            if Instant::now() >= dl {
                let _ = send_event(
                    &event_tx,
                    SearchStreamEvent::Error(ApiErrorResponse::new(
                        ApiErrorCode::SearchFailed,
                        format!(
                            "Search timed out after {} seconds",
                            timeout_secs.unwrap_or(0)
                        ),
                    )),
                );
                return;
            }
        }

        let url = &item.url;
        let collector = item.collector_id.clone();

        let _ = send_event(
            &event_tx,
            SearchStreamEvent::Progress(SearchProgress::FileStarted {
                file_index: index,
                total_files,
                file_url: url.clone(),
                collector: collector.clone(),
            }),
        );

        let parser = match filters.to_parser(url) {
            Ok(p) => p,
            Err(e) => {
                failed_files += 1;
                let _ = send_event(
                    &event_tx,
                    SearchStreamEvent::Progress(SearchProgress::FileCompleted {
                        file_index: index,
                        total_files,
                        messages_found: 0,
                        success: false,
                        error: Some(e.to_string()),
                    }),
                );
                continue;
            }
        };

        let mut file_messages: u64 = 0;
        let mut batch: Vec<BgpElem> = Vec::with_capacity(batch_size);
        let mut max_reached = false;

        for elem in parser {
            if is_cancelled() {
                break;
            }

            file_messages += 1;
            total_elements_sent += 1;
            batch.push(elem);

            if batch.len() >= batch_size {
                let batch_event = SearchStreamEvent::Elements(ElementsBatch {
                    total_so_far: total_elements_sent,
                    collector: Some(collector.clone()),
                    elements: std::mem::take(&mut batch),
                });

                if send_event(&event_tx, batch_event).is_err() {
                    // Channel closed (client gone) — cancel
                    cancel_flag.store(true, Ordering::Relaxed);
                    break;
                }
            }

            // Check max_results
            if let Some(max) = max_results {
                if total_elements_sent >= max {
                    max_reached = true;
                    break;
                }
            }
        }

        // Flush remaining partial batch for this file
        if !batch.is_empty() {
            let _ = send_event(
                &event_tx,
                SearchStreamEvent::Elements(ElementsBatch {
                    total_so_far: total_elements_sent,
                    collector: Some(collector.clone()),
                    elements: std::mem::take(&mut batch),
                }),
            );
        }

        if max_reached {
            successful_files += 1;
            total_messages += file_messages;
            let _ = send_event(
                &event_tx,
                SearchStreamEvent::Completed(SearchSummary {
                    total_files,
                    successful_files,
                    failed_files,
                    total_messages,
                    duration_secs: start_time.elapsed().as_secs_f64(),
                }),
            );
            return;
        }

        if is_cancelled() {
            break;
        }

        if file_messages > 0 {
            successful_files += 1;
        }
        total_messages += file_messages;

        let _ = send_event(
            &event_tx,
            SearchStreamEvent::Progress(SearchProgress::FileCompleted {
                file_index: index,
                total_files,
                messages_found: file_messages,
                success: true,
                error: None,
            }),
        );

        let completed = index + 1;
        let elapsed = start_time.elapsed().as_secs_f64();
        let percent = completed as f64 / total_files as f64 * 100.0;
        let eta = if completed < total_files && percent < 100.0 {
            let rate = elapsed / completed as f64;
            Some(rate * (total_files - completed) as f64)
        } else {
            None
        };

        let _ = send_event(
            &event_tx,
            SearchStreamEvent::Progress(SearchProgress::ProgressUpdate {
                files_completed: completed,
                total_files,
                total_messages,
                percent_complete: percent,
                elapsed_secs: elapsed,
                eta_secs: eta,
            }),
        );
    }

    // Send terminal event
    if is_cancelled() {
        let _ = send_event(&event_tx, SearchStreamEvent::Cancelled);
    } else {
        let _ = send_event(
            &event_tx,
            SearchStreamEvent::Completed(SearchSummary {
                total_files,
                successful_files,
                failed_files,
                total_messages,
                duration_secs: start_time.elapsed().as_secs_f64(),
            }),
        );
    }
}

/// Send an event on the channel, handling backpressure.
///
/// For progress events, use `try_send` and skip if the channel is full
/// (progress is informational and can be coalesced). For all other events
/// (started, elements, completed, cancelled, error), block until the receiver
/// is ready — these are contractual and must not be dropped.
fn send_event(tx: &mpsc::Sender<SearchStreamEvent>, event: SearchStreamEvent) -> Result<(), ()> {
    match &event {
        SearchStreamEvent::Progress(_) => match tx.try_send(event) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                // Skip progress under backpressure
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(()),
        },
        // For all other events, block until receiver is ready
        _ => tx.blocking_send(event).map_err(|_| ()),
    }
}

// =============================================================================
// CancellableStream wrapper
// =============================================================================

/// Wraps an SSE stream so that when it is dropped, the cancellation flag is
/// set, signalling the worker to stop.
struct CancellableStream<S> {
    inner: S,
    cancel_flag: Arc<AtomicBool>,
}

impl<S> CancellableStream<S> {
    fn new(inner: S, cancel_flag: Arc<AtomicBool>) -> Self {
        Self { inner, cancel_flag }
    }
}

impl<S: Stream + Unpin> Stream for CancellableStream<S> {
    type Item = S::Item;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        std::pin::Pin::new(&mut this.inner).poll_next(cx)
    }
}

impl<S> Drop for CancellableStream<S> {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_stream_filters_conversion() {
        let wire = SearchStreamFilters {
            prefix: vec!["1.1.1.0/24".to_string()],
            start_ts: "2024-01-01T00:00:00Z".to_string(),
            end_ts: "2024-01-01T00:10:00Z".to_string(),
            collector: Some("rrc00".to_string()),
            dump_type: Some("updates".to_string()),
            ..Default::default()
        };

        let filters: SearchFilters = wire.try_into().expect("conversion should succeed");
        assert_eq!(filters.collector, Some("rrc00".to_string()));
        assert_eq!(filters.parse_filters.prefix, vec!["1.1.1.0/24"]);
    }

    #[test]
    fn test_invalid_dump_type() {
        let wire = SearchStreamFilters {
            start_ts: "2024-01-01T00:00:00Z".to_string(),
            end_ts: "2024-01-01T00:10:00Z".to_string(),
            dump_type: Some("invalid".to_string()),
            ..Default::default()
        };

        let result: Result<SearchFilters, _> = wire.try_into();
        assert!(result.is_err());
    }

    #[test]
    fn test_search_stream_event_to_sse() {
        let event = SearchStreamEvent::Started(SearchStarted {
            batch_size: 100,
            max_results: Some(1000),
            timeout_secs: None,
        });
        let sse = event.to_sse();
        assert!(sse.is_some());
    }
}
