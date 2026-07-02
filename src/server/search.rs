//! SSE search streaming handler.
//!
//! `POST /api/v1/search/stream` accepts a JSON request body and returns a
//! `text/event-stream` response. The server runs a sequential, cancellable
//! parallel search executor in a blocking task, streaming progress and element
//! batch events to the client. Cancellation is triggered by closing the HTTP
//! connection — when the SSE response is dropped, the cancellation flag is
//! set and the worker stops.
//!
//! See `SSE_SERVICE_OVERHAUL_DESIGN.md` Section 6 for the algorithm.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::response::sse::{Event as SseEvent, Sse};
use axum::Json;
use bgpkit_parser::BgpElem;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::lens::parse::ParseElemType;
use crate::lens::search::{
    SearchControl, SearchElementBatch, SearchExecutionOptions, SearchExitReason, SearchFilters,
    SearchLens, SearchProgress, SearchSink, SearchSummary,
};
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
                .map(|s| {
                    s.parse()
                        .map_err(|e| anyhow::anyhow!("invalid peer_ip '{}': {}", s, e))
                })
                .collect::<anyhow::Result<Vec<_>>>()?,
            peer_asn: f.peer_asn,
            communities: f.communities,
            elem_type: match f.elem_type.as_deref() {
                None => None,
                Some("A") | Some("a") => Some(ParseElemType::A),
                Some("W") | Some("w") => Some(ParseElemType::W),
                Some(other) => anyhow::bail!(
                    "invalid elem_type '{}': expected 'A' (announce) or 'W' (withdrawal)",
                    other
                ),
            },
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
    pub file_url: String,
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

    let requested_max_results = request.max_results.filter(|max| *max > 0);
    let max_results = match (requested_max_results, config.server_max_search_results) {
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

    // 4. Enforce server-side concurrent search limit. Requests are rejected
    // immediately instead of queued so clients get deterministic feedback.
    let search_permit = match state.search_permits.as_ref() {
        Some(search_permits) => Some(
            search_permits
                .clone()
                .try_acquire_owned()
                .map_err(|_| ApiError::too_many_requests("too many concurrent search requests"))?,
        ),
        None => None,
    };

    let concurrency = if config.search_concurrency > 0 {
        Some(config.search_concurrency)
    } else {
        None
    };
    let search_pool = state.search_pool.clone();

    // 5. Create bounded channel and cancellation flag
    let (tx, rx) = mpsc::channel::<SearchStreamEvent>(32);
    let cancel_flag = Arc::new(AtomicBool::new(false));

    // 6. Spawn the search worker in a blocking task
    let worker_cancel_flag = cancel_flag.clone();
    let worker_tx = tx.clone();

    tokio::task::spawn_blocking(move || {
        let _search_permit = search_permit;
        run_search_worker(SearchWorkerConfig {
            filters,
            batch_size,
            max_results,
            timeout_secs,
            concurrency,
            search_pool,
            cancel_flag: worker_cancel_flag,
            event_tx: worker_tx,
        });
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

struct SearchWorkerConfig {
    filters: SearchFilters,
    batch_size: usize,
    max_results: Option<u64>,
    timeout_secs: Option<u64>,
    concurrency: Option<usize>,
    search_pool: Option<Arc<rayon::ThreadPool>>,
    cancel_flag: Arc<AtomicBool>,
    event_tx: mpsc::Sender<SearchStreamEvent>,
}

fn run_search_worker(config: SearchWorkerConfig) {
    let SearchWorkerConfig {
        filters,
        batch_size,
        max_results,
        timeout_secs,
        concurrency,
        search_pool,
        cancel_flag,
        event_tx,
    } = config;

    let _ = send_event(
        &event_tx,
        SearchStreamEvent::Started(SearchStarted {
            batch_size,
            max_results,
            timeout_secs,
        }),
    );

    let sink = Arc::new(SseSearchSink::new(event_tx.clone(), cancel_flag.clone()));
    let options = SearchExecutionOptions {
        concurrency,
        thread_pool: search_pool,
        max_results,
        timeout: timeout_secs.map(Duration::from_secs),
        cancel_flag: Some(cancel_flag),
        batch_size,
    };

    let lens = SearchLens::new();
    let outcome = match lens.search_with_options(&filters, options, sink) {
        Ok(outcome) => outcome,
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

    match outcome.exit_reason {
        SearchExitReason::Cancelled => {
            let _ = send_event(&event_tx, SearchStreamEvent::Cancelled);
        }
        SearchExitReason::Timeout => {
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
        }
        SearchExitReason::Completed | SearchExitReason::MaxResultsReached => {
            let _ = send_event(&event_tx, SearchStreamEvent::Completed(outcome.summary));
        }
    }
}

struct SseSearchState {
    total_so_far: u64,
}

struct SseSearchSink {
    state: Mutex<SseSearchState>,
    event_tx: mpsc::Sender<SearchStreamEvent>,
    cancel_flag: Arc<AtomicBool>,
}

impl SseSearchSink {
    fn new(event_tx: mpsc::Sender<SearchStreamEvent>, cancel_flag: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::new(SseSearchState { total_so_far: 0 }),
            event_tx,
            cancel_flag,
        }
    }
}

impl SearchSink for SseSearchSink {
    fn on_progress(&self, progress: SearchProgress) {
        if send_event(&self.event_tx, SearchStreamEvent::Progress(progress)).is_err() {
            self.cancel_flag.store(true, Ordering::Relaxed);
        }
    }

    fn on_elements(&self, batch: SearchElementBatch) -> SearchControl {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => {
                self.cancel_flag.store(true, Ordering::Relaxed);
                return SearchControl::Stop;
            }
        };

        state.total_so_far += batch.elements.len() as u64;
        let event = SearchStreamEvent::Elements(ElementsBatch {
            total_so_far: state.total_so_far,
            collector: Some(batch.collector),
            file_url: batch.file_url,
            elements: batch.elements,
        });

        if send_event(&self.event_tx, event).is_err() {
            self.cancel_flag.store(true, Ordering::Relaxed);
            SearchControl::Stop
        } else {
            SearchControl::Continue
        }
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
