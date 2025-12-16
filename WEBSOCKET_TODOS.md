# Monocle WebSocket Implementation Tracking (WEBSOCKET_TODOS)

This document tracks the implementation work for Monocle’s WebSocket API and keeps the scope from growing unintentionally.

## Guiding Constraints (Anti-bloat)

- **One envelope**: request/response shapes must remain stable and minimal.
- **Two IDs**:
  - `id`: optional request correlation (client may omit; server generates and echoes).
  - `op_id`: server-generated operation identity for long-running/streaming tasks (cancel, progress UI).
- **DB-first queries**: query methods must be network-neutral (read local DB/cache only).
- **Explicit refresh**: any network fetch occurs only via `database.refresh` (deduplicated).
- **Streaming contract**: `progress`/`stream` 0..N then exactly one terminal `result` or `error`.
- **Keep stages small**: prefer shared stages (`queued`, `running`, `downloading`, `processing`, `finalizing`, `done`) + method-specific metrics in separate fields.

---

## Status Legend

- `[ ]` Not started
- `[~]` In progress
- `[x]` Done
- `(!)` Risk / decision needed

---

## Track Board

### Track 0 — Protocol & Docs (Do this first)
- [ ] Update `WEBSOCKET_DESIGN.md`:
  - [x] Make request `id` optional; server generates one if omitted; always echoed in responses
  - [~] Clarify when `op_id` is present (long-running/streaming and refresh)
  - [ ] Ensure all examples reflect `id` optionality + `op_id` usage
  - [ ] Keep cancellation examples using `op_id`
- [x] Add a short “compatibility/versioning” policy (e.g., `protocol_version: 1` in `system.info`)
- [x] Define canonical error codes and when they are terminal
- [x] Define shared progress stages and usage rules

**Notes**
- The *code* already includes:
  - `SystemInfo { protocol_version: 1, ... }`
  - `ErrorCode` / `ErrorData`
  - `ProgressStage`
- The *docs* in `WEBSOCKET_DESIGN.md` still need to be updated to match the implemented behavior and to lock down `op_id` presence rules.

**Definition of Done:** documentation matches the intended minimal protocol; all examples consistent.

---

### Track 1 — Core WebSocket Server Skeleton
- [x] Add server entrypoint that hosts WebSocket endpoint (e.g. `/ws`)
- [~] Implement connection lifecycle:
  - [ ] limit max message size (config exists; not enforced in connection loop yet)
  - [~] ping/pong keepalive (responds to ping; periodic ping/idle handling not implemented)
  - [ ] connection timeout/idle timeout (config exists; not enforced yet)
- [~] Implement request parsing & validation:
  - [x] JSON parse + envelope validation (serde parse; invalid JSON returns terminal `INVALID_REQUEST`)
  - [x] accept missing `id` → generate server `id`
  - [~] enforce `method` required (struct requires `method`; missing field becomes parse error; explicit error messaging could improve)
- [x] Implement unified response envelope writer:
  - [x] `result`, `progress`, `stream`, `error`
  - [x] always include `id` in responses
  - [~] include `op_id` when applicable (plumbed via `WsRequest.op_id` + `WsOpSink`; but no streaming methods implemented yet to verify in practice)

**Extra (bloat-reduction/correctness already landed)**
- [x] Central single-terminal enforcement via `WsOpSink` (prevents multiple terminal envelopes per request/op)

**Definition of Done:** can accept a request, route it to a handler, and respond with a valid envelope.

---

### Track 2 — Router & Maintainability Architecture
Goal: avoid a growing `match` statement and keep protocol rules consistent.

- [x] Implement handler abstractions:
  - [x] `WsSink` (low-level envelope writer; still has extra helper methods to trim later)
  - [~] `WsContext` (currently holds `data_dir` + transport limit fields; needs dedupe per refactor plan)
  - [x] `WsMethod` trait (typed params, validation, handle) — now uses `WsOpSink` in one sweep
- [x] Implement registry-based router:
  - [x] register methods at startup
  - [x] type-erased adapter that:
    - [x] deserializes params
    - [x] calls validate
    - [x] assigns `op_id` for streaming methods (router sets `WsRequest.op_id` on register)
    - [x] ensures single terminal message (enforced by `WsOpSink`, and dispatcher uses it for terminal sends)
- [ ] (Optional) macro for trivial non-streaming handlers
  - (!) Decision: only add macro if it reduces boilerplate without obscuring control flow

**Definition of Done:** adding a new method means implementing a handler + registering it; no router boilerplate explosion.

---

### Track 3 — Operation Management (Streaming, Cancellation, Limits)
- [~] Implement operation registry keyed by `op_id`:
  - [x] store cancellation token (tracked)
  - [ ] store join handle (not implemented)
  - [x] track status (running/done/error/cancelled) (tracked as enum)
- [~] Implement generic `cancel` method:
  - [~] cancel by `op_id` (dispatcher has a `cancel(op_id, request_id, ...)` function, but no WebSocket method wired yet)
  - [~] return success result or terminal error for unknown `op_id` (implemented in dispatcher cancel helper)
- [~] Enforce concurrency limits:
  - [~] per-connection `max_concurrent_ops` (enforced globally via `OperationRegistry::with_max_concurrent`; needs true per-connection registry)
  - [ ] global cap (optional) (not implemented; current is effectively global if registry is shared)
  - [x] return `RATE_LIMITED` when exceeded (dispatcher emits `RATE_LIMITED` on registry refusal)
- [ ] Backpressure strategy:
  - [ ] bounded channels for progress/stream
  - [ ] define behavior when client is slow (drop progress? disconnect? buffer?)

**Definition of Done:** streaming ops can be cancelled reliably, and resource limits prevent runaway tasks.

---

### Track 4 — Introspection API (UI Support)
- [x] Implement `system.info`
  - [x] protocol version
  - [x] server version/build info
  - [x] feature flags (streaming/auth required)
- [ ] (Optional) implement `system.methods` (lightweight)
  - (!) Keep minimal: names + brief param fields; avoid full schema IDL

**Definition of Done:** a web/native UI can discover capabilities at runtime.

---

### Track 5 — Database Layer APIs (DB-first Contract)
- [~] Implement `database.status`
  - [x] sqlite path/exists/size counts
  - [~] `sources.*` freshness:
    - [~] `state` (`absent|ready|stale|refreshing|error`) (currently `absent|empty|ready`; not full freshness model)
    - [ ] `last_updated`
    - [ ] `next_refresh_after`
- [~] Implement `database.refresh`
  - [x] explicit refresh by `source` (supports `rpki|as2org|as2rel|all`)
  - [ ] deduplicate refresh when one is already running and `force=false`
  - [ ] emit progress updates and a final result (currently non-streaming; returns a single result)
  - [ ] ensure refresh uses DB transactions / safe writes (not enforced here; relies on database layer)
- [~] Enforce “query methods are network-neutral”
  - [ ] audit: rpki/as2org/as2rel/pfx2as lookups must not download
  - [~] refresh only from `database.refresh`
    - (!) `pfx2as.lookup` currently fetches if not cached (violates DB-first + explicit refresh constraint)

**Definition of Done:** freshness is observable; refresh behavior is predictable and deduplicated; queries never trigger download.

---

### Track 6 — Stateless Query Methods (Low Risk, High Value)
Implement quick RPC-style methods first to validate routing and schema.

- [x] `time.parse`
- [x] `country.lookup`
- [x] `ip.lookup`
- [x] `ip.public`
- [x] `pfx2as.lookup` (implemented, but violates DB-first / explicit-refresh constraint due to implicit fetch)

**Definition of Done:** stable request/response; input validation; good errors.

---

### Track 7 — RPKI Methods (DB-backed)
- [x] `rpki.validate` (DB-only)
  - [x] return validation state + covering roas
  - [x] predictable error on missing DB (`NOT_INITIALIZED`)
- [~] `rpki.roas` (DB-only)
  - [ ] ensure DB-only (currently uses lens; needs audit)
  - [ ] add optional `limit/offset` (if design includes it)
- [~] `rpki.aspas` (DB-only)
  - [ ] ensure DB-only (currently uses lens; needs audit)

**Definition of Done:** queries are fast, deterministic, and do not hit network.

---

### Track 8 — AS2Org / AS2Rel Methods (DB-backed)
- [x] `as2org.search`
  - [ ] consider pagination/limits for UI
- [x] `as2org.bootstrap` (writes DB)
  - [ ] treat as long-running op? return `op_id` + progress if needed
- [x] `as2rel.search`
- [x] `as2rel.relationship`
- [x] `as2rel.update` (writes DB; likely long-running)
  - [ ] treat as long-running op? return `op_id` + progress if needed

**Definition of Done:** DB operations are safe and observable; long-running writes expose progress.

---

### Track 9 — Streaming Operations: `parse.*`
- [ ] Implement `parse.start`
  - [ ] spawn task
  - [ ] emit `progress` + `stream` batches
  - [ ] final `result` includes totals/duration
  - [ ] uses `op_id` and registers operation for cancellation
- [ ] Implement `parse.cancel` as alias to generic cancel (optional)

**Definition of Done:** parsing streams data reliably; cancellation stops work; no repeated envelope boilerplate.

---

### Track 10 — Streaming Operations: `search.*`
- [ ] Implement `search.start`
  - [ ] broker query stage
  - [ ] file discovery stage
  - [ ] processing progress with ETA when possible
  - [ ] stream elements in batches
- [ ] Implement `search.cancel` as alias to generic cancel (optional)

**Definition of Done:** search streams data reliably across multiple files; progress stages remain within the shared vocabulary.

---

### Track 11 — Security / Production Readiness
- [ ] Input validation hardening:
  - [ ] prefix/ASN validation
  - [ ] URL validation for parse/search
  - [ ] max results/batch size bounds
- [ ] Authentication (if needed)
  - (!) Decision: whether auth is required for local-only deployments
- [ ] Rate limiting:
  - [ ] per-connection request rate
  - [ ] per-IP caps (if applicable)
- [ ] Observability:
  - [ ] structured logs for op lifecycle (`op_id`)
  - [ ] basic metrics (ops running, errors, queue depth)

**Definition of Done:** safe defaults; clear logs/metrics; predictable failure modes.

---

### Track 12 — Client Libraries & Examples
- [ ] Update JS/TS example to handle:
  - [ ] optional request `id` (server-generated echo)
  - [ ] `op_id` tracking for progress/stream
  - [ ] cancellation by `op_id`
- [ ] Update Python example similarly
- [ ] Provide “UI integration notes”:
  - [ ] recommended state machine per `op_id`
  - [ ] handling reconnects (optional)

**Definition of Done:** examples reflect final protocol and encourage correct streaming handling.

---

## Open Decisions (Keep This List Short)

1. (!) Should `system.methods` ship in v1, or remain optional?
2. (!) Backpressure: drop progress vs disconnect slow clients vs bounded buffering policy.
3. (!) Whether to include pagination (`limit/offset`) for list methods in v1.
4. (!) Whether to include a dedicated `UNKNOWN_OPERATION` error code or reuse `INVALID_PARAMS`.

---

## Notes / Implementation Order Recommendation (Updated)

Suggested order to minimize risk and align with refactor plan:

1. Finish Tracks 0–2 doc alignment (update `WEBSOCKET_DESIGN.md` to match implemented envelopes/IDs)
2. Complete Track 1 lifecycle enforcement (max message size, idle timeout, keepalive ping task)
3. Track 3 cancel method exposure (wire a `system.cancel` or `operation.cancel` websocket method; keep name minimal)
4. Track 5 DB-first contract audit (especially `pfx2as.lookup` and `rpki.*` lens usage)
5. Only then implement Tracks 9–10 streaming operations and backpressure policy (bounded channels)