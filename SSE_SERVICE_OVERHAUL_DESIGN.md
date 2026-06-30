# Monocle HTTP + Direct SSE Service Overhaul Design

## 1. Overview

This design replaces Monocle's WebSocket-oriented server with a simpler HTTP
service: direct streaming for `search` and regular REST endpoints for
non-streaming commands. For the first version, `search` does **not** use
background jobs. A client sends one HTTP request, the server keeps that
response open as `text/event-stream`, and cancellation is simply closing the
HTTP connection.

The hardest implementation work is not Axum or SSE — it is making the existing
Rayon-based search pipeline safely cancellable and streamable. This design
addresses that boundary explicitly (Section 6).

### MVP scope

```text
MVP includes:
- HTTP server routing (Axum)
- GET /health
- GET /api/v1/system/info
- POST /api/v1/search/stream (SSE)
- minimal server config (address, port, batch size, max results, timeout)
- independent wire DTOs (not internal types)
- bounded channel with backpressure
- cancellation on disconnect
- terminal event invariant
- tests for validation, streaming, batching, cancellation, limits
- minimal Dockerfile smoke test

MVP excludes (separate follow-up designs):
- auth
- CLI remote mode
- full REST API coverage (RPKI, AS2Rel, Pfx2As, Inspect, etc.)
- database refresh policy
- Docker Compose / full deployment config
- WebSocket removal
- job registry / replay / reconnect
```

Each excluded item is listed in Section 12 (Future designs) with a brief note
on when it should be addressed.

## 2. Motivation and Use Cases

- Run Monocle as a deployable HTTP service behind ordinary reverse proxies.
- Stream `search` progress/results without WebSocket protocol state.
- Use simple REST endpoints for non-streaming commands (added after MVP).
- Package Monocle as a Docker image with mounted data/cache directories.
- Keep search behavior close to the existing CLI/lens logic.

| Aspect | Current WebSocket Server | MVP HTTP + Direct SSE |
|--------|--------------------------|------------------------|
| Streaming | `/ws` JSON envelopes | `POST /api/v1/search/stream` returns `text/event-stream` |
| Cancellation | WebSocket cancel message + `op_id` | Client closes HTTP connection |
| Operation identity | `id` + `op_id` | No job ID in MVP |
| Non-streaming commands | JSON-RPC-style WebSocket methods | Normal REST endpoints (post-MVP) |
| Deployment | WebSocket-aware proxying | Standard long-lived HTTP response |

## 3. Current Implementation Review

- `src/server/mod.rs` exposes `/ws` and `/health`, owns WebSocket upgrades, idle timeout, pings, CORS, and startup.
- `src/server/protocol.rs`, `handler.rs`, `sink.rs`, `op_sink.rs`, and `router.rs` are WebSocket-specific.
- Existing `src/server/handlers/*` files already contain useful parameter/response structs and validation logic for non-streaming commands, but these are coupled to `WsOpSink` and `ResponseEnvelope`. Extracting shared logic for REST reuse requires a refactor that is **not** part of the MVP.
- `src/server/operations.rs` tracks streaming operations and cancellation — not needed for direct SSE MVP.
- `src/server/README.md` lists `search.start` and `parse.start` as implemented, but the code has no registered server-side search/parse handlers.
- `src/lens/search/mod.rs` has `SearchFilters`, `SearchProgress`, and `SearchLens::search_with_progress`, which are the right building blocks for `search` streaming.
- `search_with_progress` uses `items.into_par_iter().for_each(...)` — a blocking, parallel, **non-cancellable** loop. This is the core challenge for SSE streaming (Section 6).
- `src/bin/commands/search.rs` contains richer CLI behavior such as broker cache, MRT cache, retries, pagination, and output handling. The service MVP starts with lens-level search; shared cache/retry code can be extracted later.

## 4. Design Decisions

> **Use direct request/response SSE for search, not jobs.** A job registry adds status tracking, cancellation endpoints, retention, cleanup, and reconnect semantics. For MVP, a single streaming request is enough.

> **Cancellation is connection close.** If the client disconnects, the server cancels the search. This avoids `job_id`, `DELETE /jobs/{id}`, and operation tracking.

> **Use `POST /api/v1/search/stream` for JSON request bodies.** Native `EventSource` only supports `GET`, but JSON search filters are too complex for query strings. Browser clients use `fetch()` with a streaming response. A limited `GET` variant can be added later.

> **SSE event names are the type discriminator.** The SSE `event:` field carries the event type; the `data:` field carries JSON without a redundant `type` tag. This avoids double-tagging (e.g., `event: progress\ndata: {"type":"progress",...}`).

> **Use independent wire DTOs, not internal types.** `SearchStreamRequest` defines its own `SearchStreamFilters` DTO rather than embedding `monocle::lens::search::SearchFilters`. Similarly, element batches use a dedicated `ApiBgpElem` rather than exposing `bgpkit_parser::BgpElem` directly. This isolates the wire contract from internal refactoring.

> **Keep the lens layer synchronous and cancellation-agnostic.** The lens does not depend on `tokio` or `tokio-util`. Cancellation is communicated via a simple `Arc<AtomicBool>` or `Box<dyn Fn() -> bool + Send + Sync>` callback. The server layer wraps this with its own `CancellationToken`. Batching is a server-layer concern, not a lens concern.

> **Process search sequentially for MVP.** The existing `par_iter` loop has no early-exit and cannot be cancelled mid-iteration. The SSE path uses a sequential file loop for clean cancellation and ordered batches. Parallelism can be reintroduced later with `try_for_each` + per-element cancellation checks, but that optimization is deferred.

> **Bounded channel with explicit backpressure.** The SSE channel is bounded (e.g., capacity 32). Element batches are never dropped. If the receiver is gone or full, the search is cancelled. Progress events may be coalesced or skipped under backpressure.

> **Terminal event invariant.** A stream emits at most one terminal event: `completed`, `cancelled`, or `error`. No events are emitted after a terminal event. If the client disconnects, the terminal event may not be delivered.

> **Defer replay, job history, and multi-client subscription.** These are useful later, but not needed to validate Monocle-as-a-service.

> **Defer auth, CLI remote mode, refresh policy, and full REST coverage.** Each is a separate follow-up design (Section 12). Deployments should bind to localhost or use a reverse proxy for access control until auth is implemented.

## 5. Data Structures

### API error types

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    pub code: ApiErrorCode,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    InvalidRequest,
    InvalidParams,
    Cancelled,
    SearchFailed,
    NotInitialized,
    InternalError,
}
```

Error codes are search-oriented, not job/operation-oriented. `RateLimited` is
omitted — rate limiting is not in scope for MVP.

### Search stream request (wire DTO)

```rust
/// Independent wire DTO — not `monocle::lens::search::SearchFilters`.
/// Maps to `SearchFilters` internally so internal refactoring does not
/// break the API contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchStreamRequest {
    pub filters: SearchStreamFilters,
    pub batch_size: Option<usize>,
    pub max_results: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchStreamFilters {
    pub prefix: Vec<String>,
    pub include_super: bool,
    pub include_sub: bool,
    pub origin_asn: Vec<String>,
    pub peer_asn: Vec<String>,
    pub peer_ip: Vec<String>,
    pub communities: Vec<String>,
    pub elem_type: Option<String>,
    pub as_path: Option<String>,
    pub start_ts: String,
    pub end_ts: String,
    pub collector: Option<String>,
    pub project: Option<String>,
    pub dump_type: Option<String>,
}
```

The exact field set should mirror current `SearchFilters` + `ParseFilters` but
is owned by the server module. A `TryFrom<SearchStreamFilters>` for
`SearchFilters` performs validation and conversion.

### Search stream events

The Rust enum is internal. On the wire, each variant maps to an SSE event name
and its data is serialized directly (no `type` tag in JSON):

```rust
/// Internal event enum. The server maps each variant to an SSE `event:` name.
pub enum SearchStreamEvent {
    Started(SearchStarted),
    Progress(SearchProgress),        // reuse lens type, serialized directly
    Elements(ElementsBatch),
    Completed(SearchSummary),        // reuse lens type
    Cancelled,
    Error(ApiErrorResponse),
}

pub struct SearchStarted {
    pub batch_size: usize,
    pub max_results: Option<u64>,
    pub timeout_secs: Option<u64>,
}

/// Wire DTO for elements — uses ApiBgpElem, not bgpkit_parser::BgpElem.
pub struct ElementsBatch {
    pub total_so_far: u64,
    pub collector: Option<String>,
    pub elements: Vec<ApiBgpElem>,
}
```

Note: `batch_index` is intentionally omitted. With sequential file processing,
batches arrive in order and `total_so_far` is sufficient for client-side
accounting. If parallel processing is added later, `batch_index` remains
meaningless without a global ordering authority, so it is not part of the
contract.

`ApiBgpElem` is a dedicated wire type that `From<BgpElem>` converts into. This
prevents `bgpkit_parser` serialization changes from becoming breaking API
changes. For MVP, `ApiBgpElem` can be a thin struct mirroring the current
`BgpElem` fields, but it is owned by the server module.

### Per-request runtime state (server layer only)

```rust
struct SearchStreamState {
    cancel_flag: Arc<AtomicBool>,
    event_tx: mpsc::Sender<SearchStreamEvent>,
    batch_size: usize,
    max_results: Option<u64>,
}
```

No `CancellationToken` or `tokio` types leak into the lens layer. The server
owns the sequential search loop directly — no new lens method is needed (see
Section 6).

## 6. Search Execution and Cancellation

This is the highest-risk part of the design. The existing
`search_with_progress` uses `items.into_par_iter().for_each(...)`, which:

- Has no early-exit mechanism — you cannot cancel mid-iteration.
- Occupies the Rayon thread pool; running it inside `spawn_blocking` can
  starve the tokio blocking pool under concurrent requests.
- Produces elements from multiple files concurrently with no global ordering,
  making `batch_index` and `total_so_far` racy.

### MVP strategy: sequential processing, no new lens method

The SSE path uses a **sequential** file loop that the server owns directly.
No new method is added to `SearchLens`. The server calls the existing public
utility methods on `SearchFilters`:

- `SearchFilters::validate()` — validation
- `SearchFilters::to_broker_items()` — broker query → `Vec<BrokerItem>`
- `SearchFilters::to_parser(url)` — build a `BgpkitParser` for one file

These are already public, already used by the CLI search command, and already
composable. The server's sequential loop is:

```rust
let items = filters.to_broker_items()?;  // sends Progress::QueryingBroker, FilesFound
for (index, item) in items.into_iter().enumerate() {
    if cancel_flag.load(Relaxed) { break; }
    let parser = match filters.to_parser(&item.url) {
        Ok(p) => p,
        Err(e) => { /* record failure, send Progress::FileCompleted, continue */ continue; }
    };
    let mut file_messages = 0u64;
    for elem in parser {
        if cancel_flag.load(Relaxed) { break; }
        // convert to ApiBgpElem, append to batch, flush when full
        // check max_results, break if reached
        file_messages += 1;
    }
    // update totals, send Progress::FileCompleted + ProgressUpdate
}
// flush final partial batch, send Completed or Cancelled
```

This is slower than Rayon but provides:

- Clean cancellation (check `AtomicBool` before each file and each element).
- Natural batch ordering (no `batch_index` needed).
- Deterministic `max_results` enforcement.
- No Rayon/tokio thread pool contention.
- Full visibility: batching, SSE event generation, and cancellation are all
  in one place (`src/server/search.rs`), not split across a callback protocol.

### Why no new lens method

`SearchLens::search_with_progress` is never called outside its own doc
examples — the CLI search command already calls `SearchFilters` utilities
directly and owns its own iteration loop. Adding a `search_cancellable`
method would:

- Duplicate the existing `search_with_progress` loop with minor changes.
- Introduce a callback protocol (`should_cancel`, `progress_callback`,
  `element_handler`) between server and lens, just to re-convert those
  callbacks back into SSE events on the server side.
- Create two similar methods to maintain.

The existing `SearchFilters` utility methods are the right abstraction
boundary: they handle broker query and parser construction (the parts that
need domain knowledge), and the server owns the iteration/streaming logic
(the parts that are transport-specific).

`search_with_progress` and `search_and_collect` remain for CLI backward
compatibility but are not used by the SSE path.

### Minor cleanup: slim down `SearchLens` (optional, not blocking)

`SearchLens` is the only lens where the "Lens struct as orchestrator"
pattern doesn't carry its weight. Its `search_with_progress` and
`search_and_collect` methods are never called outside doc examples. These
can be deprecated or removed in a future cleanup. The useful parts of the
search lens — `SearchFilters`, `SearchProgress`, `SearchSummary`, and the
utility methods on `SearchFilters` — stay. This is a minor cleanup, not
part of the SSE MVP.

### Backpressure policy

```rust
let (tx, rx) = mpsc::channel::<SearchStreamEvent>(32);
```

- **Element batches are never dropped.** If `send()` fails (receiver gone or
  channel full and timed out), treat as cancellation.
- **Progress events may be coalesced or skipped** if the channel is full.
  Progress is informational; element delivery is contractual.
- **Slow client vs. timeout:** if the channel remains full and the search
  timeout expires, cancel the search and send `error` with `SearchFailed`.
- The search worker checks `cancel_flag` after each `send().await`, not just
  before each file.

### Cancellation latency

With sequential processing and an `AtomicBool` check per element, cancellation
latency is bounded by the time to process one element (microseconds to low
milliseconds). This is acceptable for MVP.

### Future optimization (deferred)

If throughput becomes a problem, reintroduce parallelism via:

```rust
items.into_par_iter().try_for_each(|item| {
    if should_cancel() { return Err(Cancelled); }
    // ...
    Ok(())
})
```

This requires per-element atomic loads across N threads and careful batch
ordering. It is explicitly deferred — the MVP prioritizes correctness and
cancellation over throughput.

## 7. API Shape

### SSE encoding model

Each event uses the SSE `event:` field as the type discriminator. The `data:`
field contains JSON without a redundant `type` tag:

```text
event: started
data: {"batch_size":100,"max_results":10000,"timeout_secs":300}

event: progress
data: {"FilesFound":{"count":5}}

event: elements
data: {"total_so_far":2,"collector":"rrc00","elements":[...]}

event: completed
data: {"total_files":5,"successful_files":5,"failed_files":0,"total_messages":275,"duration_secs":8.42}
```

### Error handling: pre-stream vs. in-stream

- **Before the stream starts** (request parsing, validation, broker query
  failure before first event): return an HTTP error status (400 or 500) with
  an `ApiErrorResponse` JSON body. No SSE headers are sent.
- **After the stream starts** (broker query failure after `started`, parse
  errors mid-stream): send an `error` SSE event with `ApiErrorResponse` as
  data, then close the stream. HTTP status is already 200.

Clients must handle both paths.

### Terminal event invariant

A stream emits **at most one terminal event**: `completed`, `cancelled`, or
`error`. No events are emitted after a terminal event. If the client
disconnects before the terminal event is delivered, the server cancels the
search and the terminal event may be lost — this is expected and documented.

### MVP endpoints

```http
GET  /health
GET  /api/v1/system/info
POST /api/v1/search/stream
```

### Search streaming request

```http
POST /api/v1/search/stream
Content-Type: application/json
Accept: text/event-stream
```

```json
{
  "filters": {
    "prefix": ["1.1.1.0/24"],
    "include_super": true,
    "include_sub": false,
    "start_ts": "2024-01-01T00:00:00Z",
    "end_ts": "2024-01-01T00:10:00Z",
    "collector": "rrc00",
    "project": "riperis",
    "dump_type": "Updates"
  },
  "batch_size": 100,
  "max_results": 10000
}
```

### Example client

```bash
curl -N \
  -H 'Accept: text/event-stream' \
  -H 'Content-Type: application/json' \
  -d @search.json \
  http://127.0.0.1:8080/api/v1/search/stream
```

Browser MVP uses `fetch()` streaming, not `EventSource`:

```js
const res = await fetch('/api/v1/search/stream', {
  method: 'POST',
  headers: { 'content-type': 'application/json', 'accept': 'text/event-stream' },
  body: JSON.stringify(request),
});
const reader = res.body.getReader();
// parse SSE frames from chunks
```

### Phase 2 REST endpoints

All non-streaming REST endpoints, organized by tier:

```http
# Tier 1: Stateless (no database)
POST /api/v1/time/parse
POST /api/v1/country/lookup
POST /api/v1/ip/lookup
GET  /api/v1/ip/public

# Tier 2: Database read-only (cache-only; return NOT_INITIALIZED if missing)
GET  /api/v1/database/status
GET  /api/v1/rpki/roa/lookup
GET  /api/v1/rpki/aspa/lookup
GET  /api/v1/pfx2as/lookup
GET  /api/v1/as2rel/relationship
POST /api/v1/as2rel/search

# Tier 3: Database refresh (explicit operations, no progress streaming)
POST /api/v1/database/refresh
POST /api/v1/inspect/refresh
POST /api/v1/as2rel/refresh

# Tier 4: Composite query (cache-only for MVP)
POST /api/v1/rpki/roa/validate
POST /api/v1/rpki/aspa/validate
POST /api/v1/inspect/query
```

All endpoints work in cache-only mode for MVP — no `auto_refresh` /
`force_refresh` knobs. If required local data is missing, return
`NOT_INITIALIZED`. Users refresh via the explicit `/refresh` endpoints.

## 8. Configuration

### MVP config fields only

All config goes through the existing `MonocleConfig` in `src/config.rs` (reads
`monocle.toml` from XDG config path or `--config`, then overlays `MONOCLE_*`
env variables). No separate server settings loader.

```toml
# HTTP service — MVP fields only
server_address = "0.0.0.0"
server_port = 8080
server_max_search_batch_size = 100
server_max_search_results = 10000
server_search_timeout_secs = 300
```

```bash
MONOCLE_SERVER_ADDRESS=0.0.0.0
MONOCLE_SERVER_PORT=8080
MONOCLE_SERVER_MAX_SEARCH_BATCH_SIZE=100
MONOCLE_SERVER_MAX_SEARCH_RESULTS=10000
MONOCLE_SERVER_SEARCH_TIMEOUT_SECS=300
```

Auth, remote-client, and refresh-policy config fields are **not** added until
their respective follow-up designs are implemented. This prevents config from
becoming a dumping ground for speculative features.

### CLI flags as overrides

```rust
/// Maximum number of elements per SSE batch
#[clap(long)]
max_search_batch_size: Option<usize>,

/// Maximum search results per request
#[clap(long)]
max_search_results: Option<u64>,
```

CLI flags override config/env values; config/env is the normal service
configuration path.

## 9. File Changes

### `src/server/mod.rs`

Change from WebSocket-first routing to HTTP service routing:

```rust
AxumRouter::new()
    .route("/health", get(health_handler))
    .nest("/api/v1", http::router())
    .layer(cors)
    .with_state(state)
```

The WebSocket `/ws` route is **removed** in this redesign (see Section 11:
WebSocket removal). The WebSocket modules (`protocol.rs`, `handler.rs`,
`sink.rs`, `op_sink.rs`, `router.rs`, `operations.rs`) are deleted. Handler
parameter/response structs in `src/server/handlers/*` that are useful for
future REST conversion are preserved but not wired into any router for MVP.

### `src/server/http.rs` (new)

Defines MVP REST routes:

```rust
pub fn router() -> AxumRouter<ServerState> {
    AxumRouter::new()
        .route("/system/info", get(system_info))
        .route("/search/stream", post(search::stream_search))
}
```

### `src/server/search.rs` (new)

Implements `SearchStreamRequest`, `SearchStreamFilters`, `ApiBgpElem`,
`ElementsBatch`, `SearchStreamEvent`, and `stream_search`.

```rust
pub async fn stream_search(
    State(state): State<ServerState>,
    Json(request): Json<SearchStreamRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    // validate, convert DTO to SearchFilters
    // create bounded channel + AtomicBool cancel flag
    // spawn_blocking the sequential search worker
    // return Sse stream; drop cancels the flag
}
```

### `src/lens/search/mod.rs`

**No changes required for MVP.** The server calls the existing public
utility methods on `SearchFilters` (`validate`, `to_broker_items`,
`to_parser`) directly. `search_with_progress` and `search_and_collect`
remain unchanged for CLI backward compatibility.

Optional future cleanup: deprecate `search_with_progress` and
`search_and_collect` since neither is called outside doc examples. This is
not blocking for the SSE MVP.

### `src/bin/monocle.rs`

Update help text:

```rust
/// Start the Monocle HTTP service (REST: /api/v1, search stream: /api/v1/search/stream)
Server(ServerArgs),
```

Add MVP safety limit CLI flags (Section 8).

### `src/server/README.md`

Replace the WebSocket API design with the HTTP/SSE design. Remove the
implementation table entries that claim `search.start` and `parse.start` are
implemented.

### `Dockerfile` (new, minimal)

```dockerfile
FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --features cli --bin monocle

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/monocle /usr/local/bin/monocle
ENV MONOCLE_SERVER_ADDRESS=0.0.0.0 \
    MONOCLE_SERVER_PORT=8080
EXPOSE 8080
ENTRYPOINT ["monocle", "server", "--address", "0.0.0.0", "--port", "8080"]
```

No volume mounts or cache dirs for MVP — search streaming does not require
local state. Volume layout is part of the future deployment design.

## 10. Tests

### Search stream

- Valid `POST /api/v1/search/stream` returns `text/event-stream`.
- Invalid search params return `400 INVALID_PARAMS` before stream starts (HTTP error, not SSE).
- Search stream emits `started` then terminal `completed` for an empty/no-match range.
- Batch size of 2 with 5 elements emits 3 element batches (2 + 2 + 1).
- `max_results` stops streaming after the configured limit; `completed` reflects the actual count.
- Client disconnect triggers `AtomicBool` cancellation; worker exits promptly.
- Exactly one terminal event per stream (completed | cancelled | error).
- Progress events may be coalesced under backpressure; element batches are never dropped.
- `ApiBgpElem` serialization shape is stable and documented.

### REST

- `GET /health` returns `OK`.
- `GET /api/v1/system/info` returns JSON without WebSocket envelope.

### Docker

- Docker image smoke test: `/health` returns `OK`.

## 11. WebSocket Removal

The WebSocket server is **removed** in this redesign, not kept as a
coexistence option. Rationale:

- The existing handlers are coupled to `WsOpSink` and `ResponseEnvelope`;
  maintaining two transport architectures doubles maintenance.
- "Temporarily" keeping WebSocket avoids the removal decision and creates
  ambiguity about which API is primary.
- No external clients are known to depend on the WebSocket API (the README
  overstates implementation status).

The WebSocket modules (`protocol.rs`, `handler.rs`, `sink.rs`, `op_sink.rs`,
`router.rs`, `operations.rs`) are deleted. Useful handler structs in
`src/server/handlers/*` are preserved for future REST conversion but are not
wired into any router for MVP.

If a future need arises for WebSocket (e.g., bidirectional streaming), it
should be added as a thin adapter over the HTTP handlers, not as a parallel
architecture.

## 12. Future Designs (Out of MVP Scope)

Each item below is a separate follow-up design. They are listed here to make
the MVP boundary explicit.

### Database refresh policy

Endpoints backed by local datasets (RPKI, AS2Rel, Pfx2AS, Inspect) need a
refresh policy. When designed, use a single enum instead of two booleans to
avoid invalid states like `auto_refresh=false, force_refresh=true`:

```rust
#[serde(rename_all = "snake_case")]
pub enum RefreshPolicy {
    CacheOnly,  // default
    Auto,
    Force,
}
```

`CacheOnly` should be the default — return an error if data is missing/stale
rather than silently triggering slow network refreshes. Explicit refresh
endpoints (`POST /api/v1/database/refresh`, `POST /api/v1/inspect/refresh`)
cover the manual refresh case until the policy is designed.

### Full REST API coverage

Convert remaining handlers to REST. Requires decoupling handler logic from
`WsOpSink`/`ResponseEnvelope`. This is a refactor with its own scope.

### Authentication

Token-only auth (`Authorization: Bearer <token>`) via config-gated middleware.
Basic auth is deferred unless a specific reverse-proxy setup requires it.
`/health` stays open for container health checks.

### CLI remote search mode

Let `monocle search` run against a remote Monocle deployment. Requires SSE
client parsing, output formatting bridge, remote/local fallback, and auth
token handling. This is a separate product feature, not part of the service
overhaul MVP.

### Docker Compose and deployment config

Full deployment with volume mounts (`/data/monocle`, `/cache/monocle`),
`docker-compose.yml`, env-based config, and deployment documentation. Only a
minimal Dockerfile is part of MVP.

### Job-based / replayable search

If clients later need reconnect, result replay, or long-running searches
independent of client connections, evolve the direct SSE design into a
job-based API. This is explicitly out of scope for MVP.

## 13. Implementation Plan

The plan is a DAG, not a linear sequence. Phases 2–4 can proceed in parallel
after Phase 1.

```text
Phase 0 (config) ──► Phase 1 (HTTP shell + search SSE) ──┬──► Phase 2 (full REST)
                                                          ├──► Phase 3 (auth)
                                                          ├──► Phase 4 (CLI remote)
                                                          └──► Phase 5 (Docker Compose)
                                                                    │
                                                                    ▼
                                                              Phase 6 (wrap-up)
```

### Phase 0: Minimal config

Goal: add only the config fields the MVP needs.

1. Extend `MonocleConfig` with: `server_address`, `server_port`,
   `server_max_search_batch_size`, `server_max_search_results`,
   `server_search_timeout_secs`.
2. Update `EMPTY_CONFIG`, `Default`, `new()`, `summary()`, and tests.
3. Support `MONOCLE_SERVER_*` env variables through the existing loader.
4. No auth, remote, or refresh-policy fields.

### Phase 1: HTTP shell + search SSE MVP

Goal: ship the core — search streaming over HTTP.

1. Add `ApiErrorResponse` / `ApiErrorCode` in `src/server/http.rs`.
2. Add `GET /health` and `GET /api/v1/system/info`.
3. Add `src/server/search.rs` with `SearchStreamRequest`,
   `SearchStreamFilters`, `ApiBgpElem`, `ElementsBatch`, `SearchStreamEvent`.
4. Implement `stream_search`: bounded channel (capacity 32), sequential
   search worker in `spawn_blocking` calling `SearchFilters::to_broker_items`
   and `SearchFilters::to_parser` directly (no new lens method),
   cancellation on disconnect via `Arc<AtomicBool>`, batching, max-results,
   timeout, terminal event invariant.
5. Remove WebSocket modules and `/ws` route.
6. Test with small time ranges and one collector (e.g., `rrc00`, 5–10 min
   update windows).
7. Add `curl -N` and `fetch()` streaming examples.

### Phase 2: Full REST API coverage (parallel with 3–4)

Goal: add non-streaming REST endpoints for all Monocle capabilities.

1. Add `MonocleDatabase` to `ServerState` (shared `Arc<MonocleDatabase>`
   opened at startup).
2. Tier 1 — stateless: `time/parse`, `country/lookup`, `ip/lookup`, `ip/public`.
3. Tier 2 — DB read-only (cache-only): `database/status`, `rpki/roa/lookup`,
   `rpki/aspa/lookup`, `pfx2as/lookup`, `as2rel/relationship`, `as2rel/search`.
4. Tier 3 — DB refresh: `database/refresh`, `inspect/refresh`, `as2rel/refresh`.
   Return JSON summary on completion; no progress streaming.
5. Tier 4 — composite query (cache-only): `rpki/roa/validate`,
   `rpki/aspa/validate`, `inspect/query`.
6. All DB-backed endpoints return `NOT_INITIALIZED` if required data is missing.
7. No refresh policy knobs (`auto_refresh`/`force_refresh`) — deferred.
8. Add endpoint tests.

### Phase 3: Authentication (parallel with 2, 4)

Goal: protect remote deployments.

1. Token-only auth middleware (`Authorization: Bearer <token>`).
2. Config: `server_auth_enabled`, `server_auth_token`.
3. `/health` stays open.
4. Tests for open mode, valid token, rejected requests.

### Phase 4: CLI remote search mode (parallel with 2, 3)

Goal: let CLI users search against a remote deployment.

1. Config: `remote_server_url`, `remote_auth_token`.
2. Map `SearchArgs` → `SearchStreamRequest`.
3. Consume SSE, format with existing CLI output formatters.
4. Preserve local search as default unless remote is configured.

### Phase 5: Docker Compose and deployment (after 3)

Goal: production-ready deployment.

1. `Dockerfile` with volume mounts (`/data/monocle`, `/cache/monocle`).
2. `docker-compose.yml` with env-based config.
3. Complete example `monocle.toml` for service deployment.
4. Docker smoke test for `/health` and a small search stream.

### Phase 6: Wrap-up

Goal: finalize documentation and tooling.

1. Update `src/server/README.md` to describe only HTTP/SSE.
2. Update top-level README and CLI help.
3. Add examples for Rust, JavaScript/fetch, curl, and Docker Compose.
4. Write a usage tutorial (can later become a blog post).
5. `cargo fmt`, `cargo clippy --all-features -- -D warnings`,
   `cargo test --all-features`.

## 14. Notes and Caveats

- The MVP intentionally avoids job IDs, job status endpoints, cancellation
  endpoints, replay, and multi-client subscriptions.
- Direct SSE is less resilient than jobs, but much easier to ship and
  validate.
- Sequential search processing is slower than Rayon but provides clean
  cancellation and ordered batches. Parallelism can be reintroduced later
  with `try_for_each` + per-element cancellation checks.
- The lens layer is unchanged. The server calls existing `SearchFilters`
  utility methods directly — no new lens method, no `tokio` dependency in the
  lens, no callback protocol between layers.
- Wire DTOs (`SearchStreamFilters`, `ApiBgpElem`) isolate the API contract
  from internal type refactoring.
- If clients later need reconnect, result replay, or long-running searches
  independent of clients, evolve this design into a job-based API.
