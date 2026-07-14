# Monocle HTTP/SSE Server

Monocle provides an HTTP API server with SSE (Server-Sent Events) streaming for
BGP search and REST endpoints for all other Monocle capabilities.

## Starting the Server

```bash
monocle server
# Starting HTTP server on 127.0.0.1:8080 (auth: false)
```

With custom address/port and auth:

```bash
monocle server --address 0.0.0.0 --port 3000 --auth-enabled true --auth-token my-secret
```

## Endpoints

### `GET /health`

Returns `OK` (200). Intended for container health checks. Always open, even
when auth is enabled.

### `GET /api/v1/system/info`

Returns server metadata as JSON:

```json
{
  "server_version": "1.3.0",
  "api_version": "v1",
  "endpoints": ["/health", "/api/v1/system/info", "/api/v1/search/stream", ...]
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

**Filter fields:**

| Field | Type | Description |
|-------|------|-------------|
| `prefix` | `Vec<String>` | Prefixes to match (CIDR) |
| `include_super` | `bool` | Include super-prefixes |
| `include_sub` | `bool` | Include sub-prefixes |
| `origin_asn` | `Vec<String>` | Filter by origin ASN |
| `peer_asn` | `Vec<String>` | Filter by peer ASN |
| `peer_ip` | `Vec<String>` | Filter by peer IP |
| `communities` | `Vec<String>` | Filter by BGP communities |
| `as_path` | `Option<String>` | AS path regex |
| `start_ts` | `String` | Start timestamp (required) |
| `end_ts` | `String` | End timestamp (required) |
| `collector` | `Option<String>` | Collector ID (e.g., `rrc00`) |
| `project` | `Option<String>` | Project (`riperis` or `routeviews`) |
| `dump_type` | `Option<String>` | `updates`, `rib`, or `rib_updates` |

**SSE events:**

| Event | Data | Description |
|-------|------|-------------|
| `started` | `{batch_size, max_results?, timeout_secs?}` | Stream started |
| `progress` | `SearchProgress` (varies) | Broker query, file start/complete, progress update |
| `elements` | `{total_so_far, collector, elements[]}` | Batch of BGP elements |
| `completed` | `SearchStreamResult` | Final: search completed or reached `max_results` |
| `cancelled` | `SearchStreamResult` | Final: client disconnected, with partial stats |
| `error` | `SearchStreamResult` | Final: search failed or timed out, with partial stats |

**Final event invariant:** A stream emits at most one final event
(`completed`, `cancelled`, or `error`). No events follow a final event.

`SearchStreamResult.stats.matched_elements` counts filtered BGP elements emitted
by the stream, not raw BGP messages. `source_bytes_compressed` is the
broker-advertised compressed size of selected files; `source_bytes_exact` is
false when any file required its rough-size fallback. `matching_collectors` and
`matching_files` list only sources that emitted at least one matched element.

**Cancellation:** Close the HTTP connection to cancel. The server detects the
drop and stops the search worker via an `Arc<AtomicBool>` flag.

**Backpressure:** The SSE channel is bounded (capacity 32). Element batches
are never dropped. Progress events may be coalesced under backpressure.

**Example:**

```bash
curl -N \
  -H 'Accept: text/event-stream' \
  -H 'Content-Type: application/json' \
  -d '{"filters":{"start_ts":"2024-01-01T00:00:00Z","end_ts":"2024-01-01T00:01:00Z","collector":"rrc00","project":"riperis","dump_type":"updates"},"batch_size":100}' \
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

### Stateless REST endpoints

#### `POST /api/v1/time/parse`

```bash
curl -s -X POST http://localhost:8080/api/v1/time/parse \
  -H 'Content-Type: application/json' \
  -d '{"times":["2024-01-01T00:00:00Z","1704067200"]}'
```

#### `POST /api/v1/country/lookup`

```bash
curl -s -X POST http://localhost:8080/api/v1/country/lookup \
  -H 'Content-Type: application/json' \
  -d '{"query":"United"}'
```

#### `POST /api/v1/ip/lookup`

```bash
curl -s -X POST http://localhost:8080/api/v1/ip/lookup \
  -H 'Content-Type: application/json' \
  -d '{"ip":"1.1.1.1","simple":true}'
```

#### `GET /api/v1/ip/public`

```bash
curl -s http://localhost:8080/api/v1/ip/public
```

### Database-backed REST endpoints

These require local data to be loaded. If data is missing, they return
`503 NOT_INITIALIZED`. Refresh via the `/refresh` endpoints first.

#### `GET /api/v1/database/status`

```bash
curl -s http://localhost:8080/api/v1/database/status
```

#### `POST /api/v1/database/refresh`

```bash
# Refresh a single source
curl -s -X POST http://localhost:8080/api/v1/database/refresh \
  -H 'Content-Type: application/json' \
  -d '{"source":"rpki"}'

# Refresh all sources
curl -s -X POST http://localhost:8080/api/v1/database/refresh \
  -H 'Content-Type: application/json' \
  -d '{"source":"all"}'
```

Sources: `rpki`, `as2rel`, `asinfo`, `all` (`pfx2as` not yet supported via API).

#### `GET /api/v1/rpki/roa/lookup`

```bash
# All ROAs
curl -s http://localhost:8080/api/v1/rpki/roa/lookup

# By ASN
curl -s "http://localhost:8080/api/v1/rpki/roa/lookup?asn=13335"
```

#### `GET /api/v1/rpki/aspa/lookup`

```bash
# By customer ASN
curl -s "http://localhost:8080/api/v1/rpki/aspa/lookup?customer_asn=13335"
```

#### `POST /api/v1/rpki/roa/validate`

```bash
curl -s -X POST http://localhost:8080/api/v1/rpki/roa/validate \
  -H 'Content-Type: application/json' \
  -d '{"prefix":"1.1.1.0/24","asn":13335}'
```

#### `GET /api/v1/pfx2as/lookup`

```bash
curl -s "http://localhost:8080/api/v1/pfx2as/lookup?prefix=1.1.1.0/24&mode=longest"
```

#### `GET /api/v1/as2rel/relationship`

```bash
curl -s "http://localhost:8080/api/v1/as2rel/relationship?asn1=13335&asn2=1299"
```

#### `POST /api/v1/as2rel/search`

```bash
curl -s -X POST http://localhost:8080/api/v1/as2rel/search \
  -H 'Content-Type: application/json' \
  -d '{"asns":[13335]}'
```

#### `POST /api/v1/as2rel/refresh`

```bash
curl -s -X POST http://localhost:8080/api/v1/as2rel/refresh
```

#### `POST /api/v1/inspect/query`

```bash
curl -s -X POST http://localhost:8080/api/v1/inspect/query \
  -H 'Content-Type: application/json' \
  -d '{"queries":["13335","1.1.1.0/24"]}'
```

#### `POST /api/v1/inspect/refresh`

```bash
curl -s -X POST http://localhost:8080/api/v1/inspect/refresh
```

## Error Handling

**Pre-stream errors** (invalid params, validation failures): HTTP 400 or 503
with `ApiErrorResponse` JSON body. No SSE headers are sent.

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

## Auth

When `server_auth_enabled` is true, `/api/v1/*` requires:

```
Authorization: Bearer <token>
```

`/health` stays open for container health checks. The server refuses to start
if auth is enabled but the token is empty.

```bash
# Config
server_auth_enabled = true
server_auth_token = "my-secret"

# Env
MONOCLE_SERVER_AUTH_ENABLED=true
MONOCLE_SERVER_AUTH_TOKEN=my-secret

# CLI
monocle server --auth-enabled true --auth-token my-secret
```

## Configuration

Server settings are part of `MonocleConfig` and configurable via `monocle.toml`
or `MONOCLE_*` environment variables. See `monocle.toml.example` for all options.

```toml
search_concurrency = 0               # 0 = auto/rayon default
server_address = "127.0.0.1"
server_port = 8080
server_max_search_batch_size = 100
server_max_search_results = 0        # 0 = unlimited
server_search_timeout_secs = 0       # 0 = no timeout
server_max_concurrent_searches = 3   # 0 = unlimited; excess requests get 429
server_auth_enabled = false
server_auth_token = ""
```

CLI flags override config values:

```bash
monocle server --address 0.0.0.0 --port 9000 --max-search-batch-size 50 --concurrency 4
```

## Docker Deployment

```bash
# Build and run with docker compose
docker compose up -d

# Health check
curl http://localhost:8080/health

# Search
curl -N -H 'Accept: text/event-stream' \
  -H 'Content-Type: application/json' \
  -d '{"filters":{"start_ts":"2024-01-01T00:00:00Z","end_ts":"2024-01-01T00:01:00Z","collector":"rrc00","dump_type":"updates"}}' \
  http://localhost:8080/api/v1/search/stream
```

Volumes:
- `/data/monocle` — SQLite database (RPKI, AS2Rel, Pfx2as, ASInfo)
- `/cache/monocle` — MRT file cache

See `docker-compose.yml` for full configuration including auth, cache TTLs,
and health checks.

## CLI Remote Search

The CLI can search against a remote Monocle service instead of locally:

```bash
monocle search -t 2024-01-01T00:00:00Z -T 2024-01-01T00:01:00Z \
  -c rrc00 \
  --remote-url http://monocle.example.net:8080/api/v1/search/stream \
  --remote-token my-secret
```

Output is formatted using the same formatters as local search (`--format`,
`--json`, etc.).

## Architecture

```
src/server/
├── mod.rs       — Server state, startup, /health, Axum router, auth wiring
├── auth.rs      — Token-based auth middleware
├── http.rs      — REST router, API error types, /api/v1/system/info
├── search.rs    — SSE search handler, wire DTOs, sequential worker loop
└── rest/
    ├── mod.rs       — Module declarations
    ├── time.rs      — Time parsing
    ├── country.rs   — Country lookup
    ├── ip.rs        — IP information lookup
    ├── rpki.rs      — RPKI ROA/ASPA lookup and ROA validation
    ├── pfx2as.rs    — Prefix-to-ASN lookup
    ├── as2rel.rs    — AS relationship search/lookup/refresh
    ├── inspect.rs   — Unified AS/prefix inspection
    └── database.rs  — Database status and refresh
```

The search worker runs in `spawn_blocking` and calls `SearchFilters` utility
methods (`to_broker_items`, `to_parser`) directly. No new lens method is
needed — the lens layer is unchanged. Cancellation uses `Arc<AtomicBool>` set
when the SSE response is dropped.

All DB-backed REST handlers open `MonocleDatabase` per-request in
`spawn_blocking` (the database connection is `!Send`). All DB-backed endpoints
return `503 NOT_INITIALIZED` if required data is missing — there is no
auto-refresh (users refresh via explicit `/refresh` endpoints).
