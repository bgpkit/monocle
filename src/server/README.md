# Monocle WebSocket API Design

This document specifies a unified WebSocket interface for third-party applications (web/native UIs, services) to interact with Monocle.

## Overview

Why WebSocket:

- Streaming results for long-running operations (parse/search)
- Real-time progress updates
- Single persistent connection with cancellation

### Design Goals (Keep It Lean)

- **Small protocol surface**: one envelope, fixed response types, consistent semantics.
- **UI-friendly**: streaming/progress + stable operation identifiers.
- **DB-first queries**: query methods must be network-neutral; refresh is explicit and deduplicated.
- **Maintainable**: consistent handler contract across lenses; avoid a growing router match.

## Architecture

Components and responsibilities:

- Clients (Web UI, native UI, CLI, services)
  - maintain one WebSocket connection
  - send requests (`method`, optional `id`, `params`)
  - receive `progress` / `stream` / terminal `result` or `error`
- Monocle server
  - WebSocket endpoint (Axum): connection management, request parsing/validation, routing
  - Lens layer: implements operations (time/ip/rpki/inspect/as2rel/pfx2as/country/parse/search)
  - Data layer: SQLite DB (authoritative local store) + file caches (as applicable)

## Common Types (Referenced by Methods)

To keep the API surface consistent and the spec compact, methods reference these shared types instead of redefining the same shape repeatedly.

### `RequestEnvelope`

```json
{
  "id": "optional-client-request-id",
  "method": "namespace.operation",
  "params": { ... }
}
```

- `id` is optional. If provided, it must be unique among in-flight requests on the same connection.
- The server always echoes `id` in responses (client-provided or server-generated).
- Long-running/streaming operations always return a server-generated `op_id`.

### `ResponseEnvelope`

```json
{
  "id": "request-id",
  "op_id": "server-operation-id",
  "type": "result" | "progress" | "error" | "stream",
  "data": { ... }
}
```

- `op_id` is present for operations that can be cancelled or produce incremental output (streaming/long-running; including refresh).
- Terminal message: exactly one `result` or `error`.

### `Pagination` (for list/query methods)

```json
{
  "limit": 100,
  "offset": 0
}
```

- `limit` (optional): clamp on the server to a safe maximum.
- `offset` (optional): non-negative.

### `QueryFilters` (shared by `parse.start` and `search.start`)

```json
{
  "origin_asn": 13335,
  "prefix": "1.1.1.0/24",
  "include_super": false,
  "include_sub": false,
  "peer_ip": [],
  "peer_asn": null,
  "elem_type": null,
  "start_ts": null,
  "end_ts": null,
  "as_path": null
}
```

Notes:
- `peer_ip` is a list of strings; empty means no filter.
- `start_ts` / `end_ts` accept either RFC3339 strings or `null` (server normalizes internally).
- `include_super` and `include_sub` define prefix match behavior when `prefix` is set.

### `ProgressStage`

To avoid UI drift, stages should use this shared vocabulary:

- `queued`, `running`, `downloading`, `processing`, `finalizing`, `done`

Method-specific details belong in additional fields (e.g., counters, ETA, filenames), not new stage strings.

## Message Protocol

### `op_id` Presence Policy (Strict)
To keep streaming state machine simple and reduce client ambiguity:

- **Non-streaming methods**:
  - `op_id` MUST be absent in all server responses (`result` / `error`).
  - Clients MUST NOT include `op_id` in requests (requests do not have an `op_id` field).
- **Streaming methods**:
  - Server MUST generate an `op_id` and include it in **every** server envelope for that operation:
    - all `progress` messages
    - all `stream` messages
    - the final terminal `result` or `error`
  - Streaming messages without `op_id` are invalid.

This document treats `op_id` as the single, stable identity for a streaming operation across all emitted messages. `id` remains the request correlation identifier.

All messages are JSON-encoded and follow a consistent envelope structure.

### Client → Server (Request)

```json
{
  "id": "optional-client-request-id",
  "method": "namespace.operation",
  "params": { ... }
}
```

| Field    | Type   | Required | Description                                                |
|----------|--------|----------|------------------------------------------------------------|
| `id`     | string | No       | Optional request correlation ID (echoed in responses)       |
| `method` | string | Yes      | Operation to perform (e.g., `rpki.validate`)                |
| `params` | object | No       | Operation-specific parameters                               |

#### Request Semantics

- If `id` is provided, it must be unique among in-flight requests on the same connection.
- Long-running operations return a server-generated `op_id` for cancellation and UI tracking.

### Server → Client (Response)

```json
{
  "id": "request-id",
  "op_id": "server-operation-id",
  "type": "result" | "progress" | "error" | "stream",
  "data": { ... }
}
```

| Field   | Type   | Description                                                                 |
|---------|--------|-----------------------------------------------------------------------------|
| `id`    | string | Request correlation ID (client-provided or server-generated)                |
| `op_id` | string | Server-generated operation identifier (present for streaming/long operations) |
| `type`  | string | Response type (see below)                                                   |
| `data`  | object | Response payload                                                            |

#### Response Types

- **`result`**: Final successful response for the operation (exactly once)
- **`progress`**: Intermediate progress update (0..N times)
- **`stream`**: Streaming data batches (0..N times)
- **`error`**: Error response (terminal; ends the operation)

#### Streaming Contract (UI-Friendly)
For streaming methods (`*.start` that stream), the server follows this exact contract:

- 0..N `progress` messages (each includes `id` and `op_id`)
- 0..N `stream` messages (each includes `id` and `op_id`)
- then **exactly one** terminal message:
  - `result` (includes `id` and `op_id`) OR
  - `error` (includes `id` and `op_id`)

After a terminal message, the operation is finished and no further messages for that `op_id` will be sent.

- For a given request `id` / operation `op_id`, the server may emit:
  - `progress` messages (optional),
  - `stream` messages (optional),
  - and then exactly one terminal message: either `result` or `error`.
- Clients should treat `result`/`error` as completion and release UI resources for that `op_id`.

### Error Response

```json
{
  "id": "request-id",
  "op_id": "server-operation-id",
  "type": "error",
  "data": {
    "code": "ERROR_CODE",
    "message": "Human-readable error message",
    "details": { ... }
  }
}
```

#### Error Codes

| Code                  | Description                                    |
|-----------------------|------------------------------------------------|
| `INVALID_REQUEST`     | Malformed request message                      |
| `UNKNOWN_METHOD`      | Method not found                               |
| `INVALID_PARAMS`      | Invalid or missing parameters                  |
| `OPERATION_FAILED`    | Operation failed during execution              |
| `OPERATION_CANCELLED` | Operation was cancelled by client              |
| `NOT_INITIALIZED`     | Required data not initialized/bootstrapped     |
| `RATE_LIMITED`        | Too many concurrent operations                 |
| `INTERNAL_ERROR`      | Unexpected server error                        |

## API Methods

### Introspection (Recommended for UIs)

#### `system.info`

Returns protocol/server metadata so web/native clients can adapt without hardcoding.

**Request:**
```json
{
  "id": "sys-1",
  "method": "system.info",
  "params": {}
}
```

**Response:**
```json
{
  "id": "sys-1",
  "type": "result",
  "data": {
    "protocol_version": 1,
    "server_version": "1.0.2",
    "build": {
      "git_sha": "unknown",
      "timestamp": "unknown"
    },
    "features": {
      "streaming": true,
      "auth_required": false
    }
  }
}
```

#### `system.methods` (Optional)

Returns a minimal method catalog for discoverability (names + short schemas). Keep this intentionally lightweight to avoid maintaining a full IDL.

---

## API Methods

### Namespace Organization

| Namespace | Description | Feature |
|-----------|-------------|---------|
| `system.*` | Server introspection | cli |
| `time.*` | Time parsing utilities | lens-core |
| `ip.*` | IP information lookup | lens-bgpkit |
| `rpki.*` | RPKI validation and data | lens-bgpkit |
| `as2rel.*` | AS-level relationships | lens-bgpkit |
| `pfx2as.*` | Prefix-to-ASN mapping | lens-bgpkit |
| `country.*` | Country code/name lookup | lens-bgpkit |
| `inspect.*` | Unified AS/prefix inspection | lens-full |
| `parse.*` | MRT file parsing (streaming) | lens-bgpkit |
| `search.*` | BGP message search (streaming) | lens-bgpkit |
| `database.*` | Database management | database |

Methods are organized into namespaces matching the lens modules:

- `time.*` - Time parsing and formatting
- `ip.*` - IP information lookup
- `rpki.*` - RPKI validation and ROA/ASPA queries
- `inspect.*` - Unified AS/prefix inspection (replaces as2org)
- `as2rel.*` - AS-level relationships
- `pfx2as.*` - Prefix-to-ASN mappings
- `country.*` - Country code/name lookup
- `parse.*` - MRT file parsing (streaming)
- `search.*` - BGP message search (streaming)
- `database.*` - Database management operations

---

### Time Operations (`time.*`)

#### `time.parse`

Parse time strings into multiple formats.

**Request:**
```json
{
  "id": "1",
  "method": "time.parse",
  "params": {
    "times": ["1697043600", "2023-10-11T00:00:00Z"],
    "format": "table"
  }
}
```

**Response:**
```json
{
  "id": "1",
  "type": "result",
  "data": {
    "results": [
      {
        "unix": 1697043600,
        "rfc3339": "2023-10-11T15:00:00+00:00",
        "human": "about 1 year ago"
      }
    ]
  }
}
```

**Parameters:**

| Field    | Type     | Required | Default   | Description                              |
|----------|----------|----------|-----------|------------------------------------------|
| `times`  | string[] | No       | [now]     | Time strings to parse                    |
| `format` | string   | No       | "table"   | Output format: table, rfc3339, unix, json|

---

### IP Operations (`ip.*`)

#### `ip.lookup`

Look up information about an IP address.

**Request:**
```json
{
  "id": "2",
  "method": "ip.lookup",
  "params": {
    "ip": "1.1.1.1"
  }
}
```

**Response:**
```json
{
  "id": "2",
  "type": "result",
  "data": {
    "ip": "1.1.1.1",
    "asn": 13335,
    "as_name": "CLOUDFLARENET",
    "country": "US",
    "prefix": "1.1.1.0/24"
  }
}
```

#### `ip.public`

Get the public IP address of the server.

**Request:**
```json
{
  "id": "3",
  "method": "ip.public",
  "params": {}
}
```

---

### RPKI Operations (`rpki.*`)

#### `rpki.validate`

Validate a prefix-ASN pair against RPKI data.

**Request:**
```json
{
  "id": "4",
  "method": "rpki.validate",
  "params": {
    "prefix": "1.1.1.0/24",
    "asn": 13335
  }
}
```

**Response:**
```json
{
  "id": "4",
  "type": "result",
  "data": {
    "validation": {
      "prefix": "1.1.1.0/24",
      "asn": 13335,
      "state": "valid",
      "reason": "ROA exists with matching ASN and valid prefix length"
    },
    "covering_roas": [
      {
        "prefix": "1.1.1.0/24",
        "max_length": 24,
        "origin_asn": 13335,
        "ta": "APNIC"
      }
    ]
  }
}
```

**Parameters:**

| Field    | Type   | Required | Description              |
|----------|--------|----------|--------------------------|
| `prefix` | string | Yes      | IP prefix (e.g., 1.1.1.0/24) |
| `asn`    | number | Yes      | AS number to validate    |

#### `rpki.roas`

List ROAs filtered by ASN and/or prefix.

**DB-first policy:** this method reads from the local Monocle database only (no remote fetch).
If RPKI data is not present locally, the server returns a terminal `error` with code `NOT_INITIALIZED`.

**Current support note:** `date` and `source` parameters are accepted for forward compatibility, but **historical snapshots and source selection are not supported in DB-first mode yet**. If `date` is provided, the server returns a terminal `error` with code `INVALID_PARAMS`.

**Request:**
```json
{
  "id": "5",
  "method": "rpki.roas",
  "params": {
    "asn": 13335,
    "prefix": null,
    "date": null,
    "source": "cloudflare"
  }
}
```

**Response:**
```json
{
  "id": "5",
  "type": "result",
  "data": {
    "roas": [
      {
        "prefix": "1.1.1.0/24",
        "max_length": 24,
        "origin_asn": 13335,
        "ta": "APNIC"
      }
    ],
    "count": 1
  }
}
```

**Parameters:**

| Field     | Type   | Required | Default      | Description                           |
|-----------|--------|----------|--------------|---------------------------------------|
| `asn`     | number | No       | null         | Filter by origin ASN                  |
| `prefix`  | string | No       | null         | Filter by prefix                      |
| `date`    | string | No       | null         | Historical date (YYYY-MM-DD). **Not supported in DB-first mode** (request will be rejected). |
| `source`  | string | No       | "cloudflare" | Data source selector. **Not supported in DB-first mode** (ignored today; reserved for future). |

#### `rpki.aspas`

List ASPAs filtered by customer and/or provider ASN.

**DB-first policy:** this method reads from the local Monocle database only (no remote fetch).
If RPKI data is not present locally, the server returns a terminal `error` with code `NOT_INITIALIZED`.

**Current support note:** `date` and `source` parameters are accepted for forward compatibility, but **historical snapshots and source selection are not supported in DB-first mode yet**. If `date` is provided, the server returns a terminal `error` with code `INVALID_PARAMS`.

**Request:**
```json
{
  "id": "6",
  "method": "rpki.aspas",
  "params": {
    "customer_asn": 13335,
    "provider_asn": null
  }
}
```

---

### Inspect Operations (`inspect.*`)

The `inspect` namespace provides unified AS and prefix information lookup, replacing the former `as2org` namespace.

#### `inspect.query`

Query AS or prefix information from multiple data sources.

Search for AS-to-Organization mappings.

**Request:**
```json
{
  "id": "req-12",
  "method": "inspect.query",
  "params": {
    "query": "13335",
    "query_type": "auto",
    "sections": ["basic", "connectivity", "rpki"],
    "limits": {
      "roas": 10,
      "prefixes": 10,
      "connectivity": 5
    }
  }
}
```

**Parameters:**
- `query` (required): ASN (13335, AS13335), prefix (1.1.1.0/24), IP (1.1.1.1), or name (cloudflare)
- `query_type` (optional): "auto" (default), "asn", "prefix", "name"
- `sections` (optional): Array of sections to include: "basic", "prefixes", "connectivity", "rpki", "all"
- `limits` (optional): Limits for each section (default: roas=10, prefixes=10, connectivity=5)

**Response:**
```json
{
  "id": "req-12",
  "type": "result",
  "data": {
    "query": "13335",
    "query_type": "asn",
    "asn": 13335,
    "name": "CLOUDFLARENET",
    "country": "US",
    "sections": {
      "connectivity": {
        "upstreams": [{"asn": 174, "name": "COGENT-174", "percentage": 85.2}],
        "downstreams": [{"asn": 14789, "name": "CLOUDFLARE-CN", "percentage": 95.1}],
        "peers": [{"asn": 6939, "name": "HURRICANE", "percentage": 92.3}]
      },
      "rpki": {
        "roas": [{"prefix": "1.1.1.0/24", "max_length": 24, "ta": "ARIN"}],
        "roa_count": 150
      }
    }
  }
}
```

#### `inspect.search`

Search ASes by name or country.

**Request:**
```json
{
  "id": "req-13",
  "method": "inspect.search",
  "params": {
    "query": "cloudflare",
    "country": null,
    "limit": 20
  }
}
```

**Response:**
```json
{
  "id": "req-13",
  "type": "result",
  "data": {
    "results": [
      {"asn": 13335, "name": "CLOUDFLARENET", "country": "US"},
      {"asn": 14789, "name": "CLOUDFLARE-CN", "country": "CN"}
    ],
    "count": 2
  }
}
```

#### `inspect.refresh`

Bootstrap AS2Org data from bgpkit-commons.
Refresh the ASInfo local database from upstream source.

**Request:**
```json
{
  "id": "req-14",
  "method": "inspect.refresh",
  "params": {
    "force": false
  }
}
```

**Response:**
```json
{
  "id": "req-14",
  "type": "result",
  "data": {
    "refreshed": true,
    "as_count": 120415,
    "message": "ASInfo data refreshed successfully"
  }
}
```

---

### AS2Rel Operations (`as2rel.*`)

#### `as2rel.search`

Search for AS-level relationships.

**Request:**
```json
{
  "id": "9",
  "method": "as2rel.search",
  "params": {
    "asns": [13335],
    "sort_by_asn": false,
    "show_name": true
  }
}
```

**Response:**
```json
{
  "id": "9",
  "type": "result",
  "data": {
    "max_peers_count": 1000,
    "results": [
      {
        "asn1": 13335,
        "asn2": 174,
        "asn2_name": "COGENT-174",
        "connected": "85.3%",
        "peer": "45.2%",
        "as1_upstream": "20.1%",
        "as2_upstream": "20.0%"
      }
    ]
  }
}
```

#### `as2rel.relationship`

Get the relationship between two specific ASNs.

**Request:**
```json
{
  "id": "10",
  "method": "as2rel.relationship",
  "params": {
    "asn1": 13335,
    "asn2": 174
  }
}
```

#### `as2rel.update`

Update AS2Rel data from BGPKIT.

**Request:**
```json
{
  "id": "11",
  "method": "as2rel.update",
  "params": {
    "url": null
  }
}
```

---

### Pfx2as Operations (`pfx2as.*`)

#### `pfx2as.lookup`

Look up the origin AS for a prefix.

**DB-first policy:** this method is **cache-only**. The server MUST NOT fetch remote pfx2as data as part of `pfx2as.lookup`. If the pfx2as cache is missing/stale, clients should call `database.refresh` for `pfx2as-cache` first; otherwise the server returns a terminal `error` with code `NOT_INITIALIZED`.

**Request:**
```json
{
  "id": "12",
  "method": "pfx2as.lookup",
  "params": {
    "prefix": "1.1.1.0/24"
  }
}
```

---

### Country Operations (`country.*`)

#### `country.lookup`

Look up country information by code or name.

**Request:**
```json
{
  "id": "13",
  "method": "country.lookup",
  "params": {
    "query": "US"
  }
}
```

**Response:**
```json
{
  "id": "13",
  "type": "result",
  "data": {
    "countries": [
      {
        "code": "US",
        "name": "United States of America",
        "alpha3": "USA"
      }
    ]
  }
}
```

---

### Parse Operations (`parse.*`) - Streaming

#### `parse.start`

Start parsing an MRT file. Results are streamed back incrementally.

**Request:**
```json
{
  "id": "14",
  "method": "parse.start",
  "params": {
    "file_path": "https://data.ris.ripe.net/rrc00/updates.20231011.1600.gz",
    "filters": { ...QueryFilters... },
    "batch_size": 100,
    "max_results": 10000
  }
}
```

**Progress Response:**
```json
{
  "id": "14",
  "op_id": "op-parse-7c2f",
  "type": "progress",
  "data": {
    "stage": "running",
    "messages_processed": 50000,
    "rate": 15000.5,
    "elapsed_secs": 3.33
  }
}
```

**Stream Response (batch of results):**
```json
{
  "id": "14",
  "op_id": "op-parse-7c2f",
  "type": "stream",
  "data": {
    "elements": [
      {
        "timestamp": 1697043600.0,
        "elem_type": "A",
        "peer_ip": "192.168.1.1",
        "peer_asn": 64496,
        "prefix": "1.1.1.0/24",
        "as_path": "64496 13335",
        "origin_asns": [13335],
        "next_hop": "192.168.1.1"
      }
    ],
    "batch_index": 0,
    "total_so_far": 100
  }
}
```

**Final Response:**
```json
{
  "id": "14",
  "op_id": "op-parse-7c2f",
  "type": "result",
  "data": {
    "total_messages": 1500,
    "duration_secs": 5.2,
    "rate": 288.46
  }
}
```

#### `parse.cancel`

Cancel an ongoing parse operation.

**Request:**
```json
{
  "id": "15",
  "method": "parse.cancel",
  "params": {
    "op_id": "op-parse-7c2f"
  }
}
```

---

### Search Operations (`search.*`) - Streaming

#### `search.start`

Start a BGP message search across multiple MRT files.

**Request:**
```json
{
  "id": "16",
  "method": "search.start",
  "params": {
    "filters": { ...QueryFilters... },
    "collector": "rrc00",
    "project": "riperis",
    "dump_type": "updates",
    "batch_size": 100,
    "max_results": 10000
  }
}
```

**Progress Responses:**

```json
{
  "id": "16",
  "type": "progress",
  "data": {
    "stage": "querying_broker"
  }
}
```

```json
{
  "id": "16",
  "type": "progress",
  "data": {
    "stage": "files_found",
    "count": 5
  }
}
```

```json
{
  "id": "16",
  "type": "progress",
  "data": {
    "stage": "processing",
    "files_completed": 2,
    "total_files": 5,
    "total_messages": 1500,
    "percent_complete": 40.0,
    "elapsed_secs": 10.5,
    "eta_secs": 15.75
  }
}
```

**Stream Response:**
```json
{
  "id": "16",
  "type": "stream",
  "data": {
    "elements": [...],
    "collector": "rrc00",
    "batch_index": 5,
    "total_so_far": 600
  }
}
```

**Final Response:**
```json
{
  "id": "16",
  "type": "result",
  "data": {
    "total_files": 5,
    "successful_files": 5,
    "failed_files": 0,
    "total_messages": 3500,
    "duration_secs": 25.3
  }
}
```

#### `search.cancel`

Cancel an ongoing search operation.

**Request:**
```json
{
  "id": "17",
  "method": "search.cancel",
  "params": {
    "op_id": "op-search-19aa"
  }
}
```

---

### Database Operations (`database.*`)

#### `database.status`

Get the status of all data sources.

**Request:**
```json
{
  "id": "18",
  "method": "database.status",
  "params": {}
}
```

**Response:**
```json
{
  "id": "18",
  "type": "result",
  "data": {
    "sqlite": {
      "path": "/home/user/.monocle/monocle.db",
      "exists": true,
      "size_bytes": 52428800,
      "asinfo_count": 120415,
      "as2rel_count": 500000,
      "rpki_roa_count": 450000
    },
    "sources": {
      "rpki": {
        "state": "ready",
        "last_updated": "2024-01-15T10:30:00Z",
        "next_refresh_after": "2024-01-15T11:30:00Z"
      }
    },
    "cache": {
      "directory": "/home/user/.monocle/cache",
      "pfx2as_cache_count": 3
    }
  }
}
```

Notes:
- `state` is one of: `absent`, `ready`, `stale`, `refreshing`, `error`.
- UI clients should use `database.status` to decide whether to request `database.refresh`.

#### `database.refresh`

Refresh a specific data source.

**Request:**
```json
{
  "id": "19",
  "method": "database.refresh",
  "params": {
    "source": "rpki",  // "asinfo", "as2rel", "rpki", or "pfx2as"
    "force": false
  }
}
```

**Progress Response:**
```json
{
  "id": "19",
  "op_id": "op-refresh-rpki-3f91",
  "type": "progress",
  "data": {
    "stage": "downloading",
    "message": "Downloading from Cloudflare..."
  }
}
```

DB-first rule:
- All query methods (`rpki.*`, `inspect.*`, `as2rel.*`, `pfx2as.*`, etc.) must be **network-neutral** and **read from local database/cache only**.
- Any network download/refresh must be explicit via `database.refresh` (or a dedicated refresh method if added later).
- The server should deduplicate refresh: if `database.refresh` is called while a refresh for the same `source` is already running and `force=false`, return a response that references the existing `op_id` (and then stream progress for that operation).

---

## Client Libraries

### Examples (kept out of this design doc)

Full runnable client examples live in the repo under `monocle/examples/` to avoid bloating this design document.

- WebSocket client (Rust): `monocle/examples/ws_client_all.rs`
  - Demonstrates calling all currently registered WebSocket methods.
  - Includes the requested `search.start` / `parse.start` request presets as commented blocks (disabled until those endpoints exist).
- WebSocket client (JavaScript/TypeScript): `monocle/examples/ws_client_all.js`
  - Demonstrates calling all currently registered WebSocket methods.
- Library (non-WS) examples:
  - `monocle/examples/search_bgp_messages.rs`

To run the WebSocket client examples:

1) Start the server (in a separate terminal):
- `cargo run --features server --bin monocle -- server --address 127.0.0.1 --port 8080`

2) Ensure the server is healthy:
- `curl http://127.0.0.1:8080/health`

3) Run the examples:
- Rust:
  - `MONOCLE_WS_URL=ws://127.0.0.1:8080/ws cargo run --example ws_client_all`
- JS:
  - `MONOCLE_WS_URL=ws://127.0.0.1:8080/ws node monocle/examples/ws_client_all.js`

## Client Operations

### Cancellation

Clients can cancel long-running operations by sending a cancel request:

```json
{
  "id": "cancel-1",
  "method": "cancel",
  "params": {
    "op_id": "op-parse-7c2f"
  }
}
```

Cancellation rules:
- `cancel` is a generic alias; method-specific cancels (`parse.cancel`, `search.cancel`) are allowed but optional.
- Cancelling an unknown `op_id` should return `error` with `INVALID_PARAMS` (or a dedicated `UNKNOWN_OPERATION` if you decide to add one later).


### Subscription (Future)

For future implementations, clients may subscribe to real-time updates:

```json
{
  "id": "sub-1",
  "method": "subscribe",
  "params": {
    "topic": "rpki.updates"
  }
}
```

---

## Implementation Details

### Maintainability: Handler Traits + (Optional) Macros

Defining a small handler trait is a net positive for maintainability **if** it stays focused on enforcing protocol consistency (envelope, `op_id`, streaming contract, error mapping) and does not try to become a full framework.

The goal is:
- every lens method has a single entry point with consistent validation and error handling,
- streaming methods consistently produce `progress`/`stream` followed by a terminal `result`/`error`,
- the router is data-driven (registry) rather than a growing `match`.

#### Suggested Trait Shape (Minimal)

- A **method handler** describes:
  - method name (`namespace.operation`)
  - whether it is streaming
  - how to parse/validate params
  - how to execute and emit responses

```rust
// src/server/ws/handler.rs
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct WsRequest {
    pub id: String,
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Debug)]
pub struct WsContext {
    // Holds DB handles, caches, config, rate-limiter, etc.
    // Kept opaque here for design purposes.
}

#[async_trait]
pub trait WsMethod: Send + Sync + 'static {
    /// Fully qualified method name, e.g. "rpki.validate"
    const METHOD: &'static str;

    /// Parameter type for this method.
    type Params: DeserializeOwned + Send;

    /// Called by the router after JSON parsing; implementers should validate inputs here.
    fn validate(params: &Self::Params) -> Result<(), WsError> {
        let _ = params;
        Ok(())
    }

    /// Execute the method. Implementations may emit progress/stream messages via `sink`.
    async fn handle(
        ctx: WsContext,
        req: WsRequest,
        params: Self::Params,
        sink: WsSink,
    ) -> Result<(), WsError>;
}
```

- `WsSink` is an abstraction over the WebSocket sender that only exposes “send typed envelopes”:
  - `send_progress(id, op_id, data)`
  - `send_stream(id, op_id, data)`
  - `send_result(id, op_id, data)`
  - `send_error(id, op_id, code, message, details)`

That single abstraction prevents each handler from re-implementing envelope formatting.

#### Optional Macro (Use Carefully)

A small macro can reduce boilerplate for trivial methods (non-streaming) without hiding important control flow. For example:

- `ws_method!("time.parse", ParamsType, |ctx, params| async move { ... })`

Avoid a macro that generates too much infrastructure; the trait already provides the consistency boundary.

#### Router Registry

Instead of a large `match`, register handlers at startup:

- `HashMap<&'static str, Arc<dyn DynWsHandler>>`
- where `DynWsHandler` is a type-erased adapter that:
  - deserializes `params` into the handler's `Params`,
  - calls `validate`,
  - assigns/generates `op_id` for streaming,
  - invokes `handle`,
  - ensures exactly one terminal `result` or `error`.

This keeps maintenance cost low as method count grows.

### Server Setup

```rust
// Cargo.toml additions
[dependencies]
axum = { version = "0.7", features = ["ws"] }
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = "0.21"
futures = "0.3"
```

### Connection Handling

```rust
// src/server/mod.rs
use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::Response,
    routing::get,
    Router,
};

pub fn create_router() -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
}

async fn ws_handler(ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(handle_socket)
}

async fn handle_socket(socket: WebSocket) {
    let (sender, receiver) = socket.split();
    // Handle incoming messages and route to appropriate handlers
}
```

### Message Routing

```rust
// src/server/router.rs
pub async fn route_message(
    method: &str,
    params: serde_json::Value,
    sender: &mut SplitSink<WebSocket, Message>,
) -> Result<(), Error> {
    match method {
        "time.parse" => handle_time_parse(params, sender).await,
        "rpki.validate" => handle_rpki_validate(params, sender).await,
        "parse.start" => handle_parse_start(params, sender).await,
        // ... other methods
        _ => Err(Error::UnknownMethod(method.to_string())),
    }
}
```

### Progress Streaming

For long-running operations, use channels to stream progress:

```rust
// src/server/handlers/parse.rs
pub async fn handle_parse_start(
    params: ParseParams,
    sender: &mut SplitSink<WebSocket, Message>,
    request_id: String,
) -> Result<(), Error> {
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(100);
    
    // Spawn parsing task
    let handle = tokio::spawn(async move {
        let lens = ParseLens::new();
        let callback = Arc::new(move |progress| {
            let _ = progress_tx.blocking_send(progress);
        });
        lens.parse_with_progress(&params.filters, &params.file_path, Some(callback))
    });
    
    // Stream progress updates
    while let Some(progress) = progress_rx.recv().await {
        let msg = create_progress_message(&request_id, progress);
        sender.send(Message::Text(msg)).await?;
    }
    
    // Send final result
    let result = handle.await??;
    let msg = create_result_message(&request_id, result);
    sender.send(Message::Text(msg)).await?;
    
    Ok(())
}
```

---

## Configuration

### Server Configuration

```toml
# monocle.toml
[server]
# WebSocket server address
address = "127.0.0.1"
port = 8800

# Maximum concurrent operations per connection
max_concurrent_ops = 5

# Maximum message size (bytes)
max_message_size = 10485760  # 10MB

# Connection timeout (seconds)
connection_timeout = 300

# Ping interval for keepalive (seconds)
ping_interval = 30
```

### Environment Variables

```bash
MONOCLE_SERVER_ADDRESS=0.0.0.0
MONOCLE_SERVER_PORT=8800
MONOCLE_DATA_DIR=~/.monocle
```

---

## Security Considerations

### Authentication (Future)

For production deployments, authentication should be added:

```json
{
  "id": "auth-1",
  "method": "auth.login",
  "params": {
    "token": "api-key-or-jwt"
  }
}
```

### Rate Limiting

- Maximum concurrent operations per connection: 5
- Maximum connections per IP: 10
- Request rate limit: 100 requests/minute

### Input Validation

All inputs are validated before processing:
- Prefix format validation (valid CIDR notation)
- ASN range validation (1-4294967295)
- Time string parsing validation
- File path/URL validation for parse operations

---

## Client Libraries

### JavaScript/TypeScript Example

```typescript
class MonocleClient {
  private ws: WebSocket;
  private pending: Map<string, { resolve: Function; reject: Function }>;
  private messageId: number = 0;

  constructor(url: string = 'ws://localhost:8800/ws') {
    this.ws = new WebSocket(url);
    this.pending = new Map();
    
    this.ws.onmessage = (event) => {
      const response = JSON.parse(event.data);
      const handler = this.pending.get(response.id);
      
      if (response.type === 'result') {
        handler?.resolve(response.data);
        this.pending.delete(response.id);
      } else if (response.type === 'error') {
        handler?.reject(new Error(response.data.message));
        this.pending.delete(response.id);
      } else if (response.type === 'progress' || response.type === 'stream') {
        // Handle streaming updates
        handler?.onProgress?.(response.data);
      }
    };
  }

  async call(method: string, params: any = {}): Promise<any> {
    const id = String(++this.messageId);
    
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  // Convenience methods
  async validateRpki(prefix: string, asn: number) {
    return this.call('rpki.validate', { prefix, asn });
  }

  async searchAs(query: string) {
    return this.call('as2org.search', { query: [query] });
  }
}
```

### Python Example

```python
import asyncio
import json
import websockets

class MonocleClient:
    def __init__(self, url='ws://localhost:8800/ws'):
        self.url = url
        self.message_id = 0
        
    async def call(self, method: str, params: dict = None):
        async with websockets.connect(self.url) as ws:
            self.message_id += 1
            request = {
                'id': str(self.message_id),
                'method': method,
                'params': params or {}
            }
            await ws.send(json.dumps(request))
            
            while True:
                response = json.loads(await ws.recv())
                if response['type'] == 'result':
                    return response['data']
                elif response['type'] == 'error':
                    raise Exception(response['data']['message'])
                elif response['type'] in ('progress', 'stream'):
                    yield response['data']
    
    async def validate_rpki(self, prefix: str, asn: int):
        return await self.call('rpki.validate', {'prefix': prefix, 'asn': asn})
```

---

## Implementation Tracking

### Implemented Methods

| Method | Status | Notes |
|--------|--------|-------|
| `system.info` | ✅ | Server introspection |
| `system.methods` | ✅ | Method listing |
| `time.parse` | ✅ | Time string parsing |
| `ip.lookup` | ✅ | IP information |
| `ip.public` | ✅ | Public IP lookup |
| `rpki.validate` | ✅ | RFC 6811 validation |
| `rpki.roas` | ✅ | ROA listing |
| `rpki.aspas` | ✅ | ASPA listing |
| `as2rel.search` | ✅ | Relationship search |
| `as2rel.relationship` | ✅ | Pair relationship |
| `as2rel.update` | ✅ | Data refresh |
| `pfx2as.lookup` | ✅ | Prefix-to-ASN mapping |
| `country.lookup` | ✅ | Country code/name |
| `inspect.query` | ✅ | Unified AS/prefix lookup |
| `inspect.search` | ✅ | Name/country search |
| `inspect.refresh` | ✅ | ASInfo refresh |
| `parse.start` | ✅ | Streaming MRT parsing |
| `parse.cancel` | ✅ | Cancel parsing |
| `search.start` | ✅ | Streaming BGP search |
| `search.cancel` | ✅ | Cancel search |
| `database.status` | ✅ | Database info |
| `database.refresh` | ✅ | Data source refresh |

### Deprecated Methods

| Method | Replacement | Notes |
|--------|-------------|-------|
| `as2org.search` | `inspect.search` | Use unified inspect namespace |
| `as2org.bootstrap` | `inspect.refresh` | Use unified inspect namespace |

---

## Comparison with REST API

| Aspect                | WebSocket                      | REST                           |
|-----------------------|--------------------------------|--------------------------------|
| Connection            | Persistent                     | Per-request                    |
| Streaming             | Native support                 | Requires SSE or polling        |
| Progress updates      | Push from server               | Polling required               |
| Cancellation          | Immediate via message          | Requires separate endpoint     |
| Complexity            | Higher initial setup           | Simpler                        |
| Caching               | Not applicable                 | HTTP caching available         |
| Load balancing        | Sticky sessions needed         | Stateless, easy to scale       |

For Monocle's use case, WebSocket is preferred because:
1. Long-running operations (parse, search) benefit greatly from streaming
2. Real-time progress updates improve user experience
3. Single connection reduces overhead for frequent queries
4. Cancellation is a first-class feature

---

## Future Enhancements

1. **Pub/Sub for Real-time Updates**: Subscribe to RPKI changes, new BGP data
2. **Query Batching**: Send multiple queries in a single message
3. **Binary Protocol**: Option for more efficient binary encoding (MessagePack, CBOR)
4. **GraphQL over WebSocket**: For complex query scenarios
5. **Multiplexing**: Multiple logical channels over single connection