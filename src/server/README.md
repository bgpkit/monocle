# Monocle HTTP/SSE Server

Monocle provides an HTTP API server with SSE (Server-Sent Events) streaming for
BGP search. This replaces the previous WebSocket server architecture.

## Endpoints

### `GET /health`

Returns `OK` (200). Intended for container health checks. Always open, even
when auth is enabled (future).

### `GET /api/v1/system/info`

Returns server metadata as JSON:

```json
{
  "server_version": "1.3.0",
  "api_version": "v1",
  "endpoints": ["/health", "/api/v1/system/info", "/api/v1/search/stream"]
}
```

### `POST /api/v1/search/stream`

Streams BGP search results as Server-Sent Events (`text/event-stream`).

**Request body:**

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
    "dump_type": "updates"
  },
  "batch_size": 100,
  "max_results": 10000
}
```

**SSE events:**

| Event | Data | Description |
|-------|------|-------------|
| `started` | `{batch_size, max_results?, timeout_secs?}` | Stream started |
| `progress` | `SearchProgress` (varies) | Broker query, file start/complete, progress update |
| `elements` | `{total_so_far, collector, elements[]}` | Batch of BGP elements |
| `completed` | `SearchSummary` | Terminal: search completed successfully |
| `cancelled` | (empty) | Terminal: client disconnected |
| `error` | `ApiErrorResponse` | Terminal: search failed |

**Terminal event invariant:** A stream emits at most one terminal event
(`completed`, `cancelled`, or `error`). No events follow a terminal event.

**Cancellation:** Close the HTTP connection to cancel. The server detects the
drop and stops the search worker.

**Backpressure:** The SSE channel is bounded (capacity 32). Element batches
are never dropped. Progress events may be coalesced under backpressure.

**Example:**

```bash
curl -N \
  -H 'Accept: text/event-stream' \
  -H 'Content-Type: application/json' \
  -d @search.json \
  http://127.0.0.1:8080/api/v1/search/stream
```

**Browser (fetch streaming):**

```js
const res = await fetch('/api/v1/search/stream', {
  method: 'POST',
  headers: { 'content-type': 'application/json', 'accept': 'text/event-stream' },
  body: JSON.stringify(request),
});
const reader = res.body.getReader();
// parse SSE frames from chunks
```

## Error handling

**Pre-stream errors** (invalid params, validation failures): HTTP 400 with
`ApiErrorResponse` JSON body. No SSE headers are sent.

**In-stream errors** (broker failure, parse error, timeout): SSE `error` event
with `ApiErrorResponse` as data. HTTP status is already 200.

```json
{
  "code": "INVALID_PARAMS",
  "message": "start-ts is not a valid time string: "
}
```

Error codes: `INVALID_REQUEST`, `INVALID_PARAMS`, `CANCELLED`, `SEARCH_FAILED`,
`NOT_INITIALIZED`, `INTERNAL_ERROR`.

## Configuration

Server settings are part of `MonocleConfig` and configurable via `monocle.toml`
or `MONOCLE_*` environment variables:

```toml
server_address = "127.0.0.1"
server_port = 8080
server_max_search_batch_size = 100
server_max_search_results = 0        # 0 = unlimited
server_search_timeout_secs = 0       # 0 = no timeout
```

```bash
MONOCLE_SERVER_ADDRESS=0.0.0.0
MONOCLE_SERVER_PORT=8080
MONOCLE_SERVER_MAX_SEARCH_BATCH_SIZE=100
MONOCLE_SERVER_MAX_SEARCH_RESULTS=10000
MONOCLE_SERVER_SEARCH_TIMEOUT_SECS=300
```

CLI flags override config values:

```bash
monocle server --address 0.0.0.0 --port 9000 --max-search-batch-size 50
```

## Architecture

```
src/server/
├── mod.rs       — Server state, startup, /health, Axum router
├── http.rs      — REST routes, API error types, /api/v1/system/info
└── search.rs    — SSE search handler, wire DTOs, sequential worker loop
```

The search worker runs in `spawn_blocking` and calls `SearchFilters` utility
methods (`to_broker_items`, `to_parser`) directly. No new lens method is
needed — the lens layer is unchanged. Cancellation uses `Arc<AtomicBool>` set
when the SSE response is dropped.
