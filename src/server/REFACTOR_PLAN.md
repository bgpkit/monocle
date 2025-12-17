# WebSocket Server Refactor Plan (One-sweep + No Back-Compat)

This plan tracks the refactor of `monocle/src/server` to reduce bloat, eliminate duplicated configuration, and tighten protocol correctness—while staying within the guiding constraints of `WEBSOCKET_TODOS.md`.

**Important project decision (sticky):**
- **One-sweep changes only.** When refactoring an API/surface, update all call sites at once.
- **No backward compatibility required.** Prefer simpler end state over transitional adapters.

---

## Guiding constraints (do not violate)

- **One envelope**: stable minimal `RequestEnvelope` / `ResponseEnvelope`.
- **Two IDs**:
  - `id`: optional request correlation in request; server generates if omitted; always echoed in responses.
  - `op_id`: server-generated operation identity for long-running/streaming tasks.
- **Streaming contract**: `progress`/`stream` 0..N then exactly one terminal `result` or `error`.
- **DB-first queries**: query methods must be network-neutral.
- **Explicit refresh**: any network fetch only via `database.refresh` (deduplicated).
- **Keep stages small**: shared stages and method-specific metrics in separate fields.

---

## Status legend

- `[ ]` Not started
- `[~]` In progress
- `[x]` Done
- `(!)` Risk / decision needed

---

## 0) Current state snapshot (COMPLETED)

All major refactoring objectives have been achieved:

- `[x]` `WsOpSink` exists and enforces **single terminal envelope** semantics (terminal guard).
- `[x]` `WsRequest` includes `op_id: Option<String>` and dispatcher assigns it for streaming methods.
- `[x]` **One-sweep handler signature update**: All handlers use `WsOpSink`.
- `[x]` Router emits parse/unknown-method/rate-limited errors via `WsOpSink` (terminal-guarded).
- `[x]` `WsSink` is minimal transport-only (`send_envelope`, `send_message_raw`).
- `[x]` `WsContext` contains only resources (no transport policy fields).
- `[x]` Connection lifecycle enforced in `handle_socket`:
  - Max message size check
  - Keepalive ping at configured interval
  - Idle/connection timeout
- `[x]` `protocol.rs` contains only core protocol types.
- `[x]` `OperationRegistry` uses O(1) concurrency checking via `AtomicUsize`.
- `[x]` Concurrency limits wired from `ServerConfig.max_concurrent_ops` to `OperationRegistry`.

---

## 1) Refactor objectives (ACHIEVED)

### 1.1 Bloat reduction ✓
- `WsContext` is "resources only" (just `data_dir`), no transport policy.
- `WsSink` exposes only core primitives (`send_envelope`, `send_message_raw`).
- `protocol.rs` is minimal and stable.

### 1.2 Protocol correctness & operational safety ✓
- Streaming rules enforced centrally:
  - *Single terminal* enforced via `WsOpSink`
  - `op_id` presence policy enforced in `Router.dispatch()`:
    - Streaming methods: `op_id` MUST be present
    - Non-streaming methods: `op_id` MUST be absent
- Connection-level limits enforced in `handle_socket`:
  - Max message size
  - Ping keepalive
  - Idle timeout
- Operation registry correctly tracks and cleans up operations.

---

## 2) Plan overview (phases, one-sweep) — ALL COMPLETE

### Phase A — Terminal-guarded sink wrapper ✓
- `[x]` Implement `WsOpSink` enforcing "exactly one terminal response".

### Phase B — Make `op_id` systematic and enforceable ✓
- `[x]` Add `WsRequest.op_id: Option<String>` assigned by dispatcher for streaming methods.
- `[x]` One-sweep: change all handlers to use `WsOpSink`.
- `[x]` Strict rule enforced: streaming methods get `op_id`, non-streaming do not.
- `[x]` Dispatcher only registers an operation for `IS_STREAMING = true` methods.

### Phase C — Reduce `WsSink` API surface (one-sweep) ✓
- `[x]` `WsSink` reduced to minimal set:
  - `send_envelope`
  - `send_message_raw` (server-internal for ping/pong/close)
- `[x]` Semantic error creation moved to `ErrorData` constructors.
- `[x]` All protocol-specific helpers removed from `WsSink`.

### Phase D — Unify configuration (remove duplication, enforce limits) ✓
- `[x]` Transport policy fields stripped from `WsContext`.
- `[x]` Policy enforced in server loop (`handle_socket`) using `ServerConfig`:
  - `[x]` Max message size check for both text and binary
  - `[x]` Keepalive ping task (periodic ping)
  - `[x]` Idle timeout / connection timeout
- `[x]` Concurrency limits enforced via `OperationRegistry.with_max_concurrent()`.

### Phase E — Slim `protocol.rs` (reduce scope footprint) ✓
- `[x]` Non-core/future types (`Pagination`, `QueryFilters`) were never added / not present.
- `[x]` Core protocol types remain in `protocol.rs`:
  - `RequestEnvelope`, `ResponseEnvelope`, `ResponseType`
  - `ErrorData`, `ErrorCode`
  - `ProgressStage`
  - `SystemInfo` (and subtypes)

### Phase F — Operation management improvements ✓
- `[x]` `OperationRegistry` improvements:
  - Stores metadata (request_id, method, status, cancel_token, started_at)
  - O(1) concurrency check via `AtomicUsize` (no scanning/try_lock)
  - `cleanup()` method available for periodic cleanup
  - `complete_and_remove()`, `fail_and_remove()`, `cancel_and_remove()` for atomic status+removal
- `[x]` Cancellation semantics consistent:
  - Cancel method returns terminal `result` or terminal `error` for unknown `op_id`
  - Cancellation sets op status and triggers token

---

## 3) Remaining work (minor)

- `[ ]` Wire up periodic cleanup task for `OperationRegistry`
  - The `cleanup(older_than: Duration)` method exists but is not called periodically
  - Could add a background task in `handle_socket` or server startup to call it
  - Low priority: operations are removed on completion/failure/cancellation anyway

---

## 4) Non-goals (avoid scope creep)

This refactor does **not**:
- Add authentication
- Add new WS methods
- Add full backpressure semantics / buffering policy
- Add IDL/schema generation
- Add global rate limiting per IP

---

## 5) Definition of Done (ACHIEVED)

- ✓ `WsOpSink` is the exclusive high-level response API used by handlers and dispatcher.
- ✓ `WsSink` is a minimal transport-only component.
- ✓ Connection limits (size, idle, keepalive) are enforced centrally in server loop.
- ✓ `WsContext` contains no duplicated transport policy; resources only.
- ✓ `protocol.rs` contains only the stable core protocol types.
- ✓ Operation tracking does not grow unbounded and correctly reflects terminal outcomes.

---

## 6) File Summary

| File | Purpose | Status |
|------|---------|--------|
| `sink.rs` | Minimal transport wrapper (`send_envelope`, `send_message_raw`) | ✓ Clean |
| `op_sink.rs` | Terminal-guarded operation sink | ✓ Clean |
| `protocol.rs` | Core protocol types only | ✓ Clean |
| `handler.rs` | `WsMethod` trait, `WsContext` (resources only) | ✓ Clean |
| `router.rs` | `Router` + `Dispatcher` with op_id policy enforcement | ✓ Clean |
| `operations.rs` | `OperationRegistry` with O(1) concurrency | ✓ Clean |
| `mod.rs` | Server startup, `handle_socket` with lifecycle enforcement | ✓ Clean |
| `handlers/*.rs` | Individual method handlers using `WsOpSink` | ✓ Clean |