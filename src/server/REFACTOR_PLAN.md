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

## 0) Current state snapshot (updated)

### Implemented (as of this plan update)
- `[x]` `WsOpSink` exists and enforces **single terminal envelope** semantics (terminal guard).
- `[x]` `WsRequest` includes `op_id: Option<String>` and dispatcher assigns it for streaming methods.
- `[x]` **One-sweep handler signature update**:
  - `WsMethod::handle(..., sink: WsOpSink)` (no `WsSink` in handlers anymore).
  - Router/dispatcher passes `WsOpSink` into all handlers.
- `[x]` Router emits parse/unknown-method/rate-limited errors via `WsOpSink` (terminal-guarded).
- `[x]` All existing handlers updated to accept `WsOpSink` and use `sink.send_result(...)`.

### Still present / to improve
- `[ ]` Connection lifecycle limits are not enforced in `handle_socket`:
  - max message size
  - keepalive ping
  - idle/connection timeout
- `[ ]` `WsContext` still contains transport policy fields (`max_*`) duplicating `ServerConfig`.
- `[ ]` `WsSink` still has a large API surface (many helpers).
- `[ ]` `protocol.rs` still includes non-core/future structures (`Pagination`, `QueryFilters`).
- `[ ]` Operation registry concurrency limit is global-ish; still missing per-connection limit semantics & cleanup wiring.
- `[ ]` Lossy/implicit behavior around “streaming contract” is only partly enforced:
  - terminal uniqueness is enforced via `WsOpSink`
  - but the system still needs a consistent rule for when `op_id` must be present (and for which methods).

---

## 1) Refactor objectives

### 1.1 Bloat reduction
- Shrink and clarify public surfaces:
  - `WsContext` should be “resources only” (db handles, config references), not transport policy.
  - `WsSink` should expose just core primitives; semantics live in `ErrorData` and higher-level wrappers.
- Keep `protocol.rs` minimal and stable.

### 1.2 Protocol correctness & operational safety
- Centralize streaming rules to prevent handler bugs:
  - already enforced: *single terminal*
  - still needed: policy for `op_id` presence and operation lifecycle
- Enforce connection-level limits and keepalive policy centrally in server loop.
- Improve operation registry for predictable enforcement and cleanup.

---

## 2) Plan overview (phases, one-sweep)

### Phase A — Terminal-guarded sink wrapper
- `[x]` Implement `WsOpSink` enforcing “exactly one terminal response”.

**Acceptance met**:
- Terminal sends are guarded and reusable.

---

### Phase B — Make `op_id` systematic and enforceable
- `[x]` Add `WsRequest.op_id: Option<String>` assigned by dispatcher for streaming methods.
- `[x]` One-sweep: change all handlers to use `WsOpSink` (no compatibility layer).

**Remaining work**
- `[ ]` Define and enforce a **strict rule**:
  - For streaming methods: `op_id` MUST be present in `progress`/`stream`/terminal messages.
  - For non-streaming methods: `op_id` MUST be absent.
- `[ ]` Ensure dispatcher only registers an operation when it will actually be used (i.e. method is truly streaming/long-running per protocol).
  - This likely means: only `IS_STREAMING = true` methods get an operation entry today.
  - When refresh becomes streaming/long-running, set it to streaming.

**Implementation note**
- `WsOpSink` already supports including `op_id`; ensure `send_progress`/`send_stream` require `op_id` and error loudly otherwise.

---

### Phase C — Reduce `WsSink` API surface (one-sweep)
- `[ ]` Replace `WsSink` helper methods with a minimal set:
  - `send_envelope`
  - (maybe) `send_message_raw` (server-internal only)
- `[ ]` Move semantic error creation to `ErrorData` constructors (already exist).
- `[ ]` Remove `WsSink::send_result/send_error/...` if they are now redundant with `WsOpSink`.

**Why**
- Handlers now use `WsOpSink`, so the extra `WsSink` sugar is largely unused bloat.

**Acceptance**
- Server compiles.
- Router/dispatcher uses `WsOpSink` for envelope-level responses; `WsSink` becomes a low-level transport primitive.

---

### Phase D — Unify configuration (remove duplication, enforce limits)
- `[ ]` **One-sweep**: strip transport policy fields from `WsContext`:
  - remove `max_concurrent_ops`
  - remove `max_message_size`
- `[ ]` Enforce policy in server loop (`handle_socket`) using `ServerConfig`:
  - `[ ]` max message size check for both text and binary
  - `[ ]` keepalive ping task (periodic ping)
  - `[ ]` idle timeout / connection timeout (disconnect on inactivity)
- `[ ]` Ensure concurrency limits are enforced consistently:
  - per-connection (preferred) and/or global
  - return `RATE_LIMITED` (terminal) when exceeded

**Acceptance**
- Only `ServerConfig` carries these knobs.
- Connection loop actually enforces them.

---

### Phase E — Slim `protocol.rs` (reduce scope footprint)
- `[ ]` Move non-core/future types out of `protocol.rs`:
  - `Pagination`
  - `QueryFilters`
- `[ ]` Keep core protocol types in `protocol.rs`:
  - `RequestEnvelope`, `ResponseEnvelope`, `ResponseType`
  - `ErrorData`, `ErrorCode`
  - `ProgressStage`
  - `SystemInfo` (and subtypes)

**Acceptance**
- No breaking changes to the wire envelope.
- Cleaner separation between “protocol core” vs “method-specific helpers”.

---

### Phase F — Operation management improvements (still within refactor scope)
This plan does **not** add new methods, but it does tighten correctness around operations.

- `[ ]` Improve `OperationRegistry`:
  - store metadata (already)
  - add cleanup policy wiring (periodic cleanup or cleanup-on-complete)
  - enforce concurrency with less contention (e.g. avoid scanning/try_lock patterns)
- `[ ]` Confirm cancellation semantics are consistent:
  - cancel method returns terminal `result` or terminal `error` for unknown `op_id`
  - cancellation sets op status and triggers token

**Acceptance**
- Predictable concurrency enforcement and no unbounded growth in registry.

---

## 3) Execution checklist (remaining work, ordered)

### Step 1 (C): Shrink `WsSink` to minimal transport
- [ ] Remove specialized helpers from `WsSink` (`send_invalid_request`, `send_rate_limited`, etc).
- [ ] Ensure all semantic errors come from `ErrorData::*`.
- [ ] Keep only what server internals need:
  - [ ] `send_envelope`
  - [ ] raw send helper for pong/close (server-only)

### Step 2 (D): Make `WsContext` resource-only
- [ ] Remove duplicated config fields from `WsContext`.
- [ ] Update constructors/tests accordingly.

### Step 3 (D): Enforce connection lifecycle constraints in `handle_socket`
- [ ] Reject messages exceeding `ServerConfig.max_message_size`.
- [ ] Add ping keepalive at `ServerConfig.ping_interval_secs`.
- [ ] Add idle timeout / connection timeout using `ServerConfig.connection_timeout_secs`.

### Step 4 (E): Move future protocol helpers out of `protocol.rs`
- [ ] Create a new module for query/pagination helpers (or future methods).
- [ ] Update imports accordingly.

### Step 5 (F): Upgrade operation registry enforcement and cleanup
- [ ] Improve concurrency limit implementation to avoid lock-scanning under load.
- [ ] Add cleanup wiring (either periodic task or cleanup-on-complete).
- [ ] Confirm dispatcher marks operation:
  - completed on successful terminal
  - failed on error terminal
  - cancelled on cancel

---

## 4) Non-goals (avoid scope creep)

This refactor does **not**:
- Add authentication
- Add new WS methods
- Add full backpressure semantics / buffering policy
- Add IDL/schema generation
- Add global rate limiting per IP

---

## 5) Risks & mitigations (updated)

- **Large churn risk**
  - Mitigation: keep each phase “one sweep” but limited in scope; land in distinct commits.
- **Behavioral changes around connection lifetime**
  - Mitigation: enforce only explicit knobs in `ServerConfig`, and keep defaults conservative.
- **Accidental protocol expansion**
  - Mitigation: keep `protocol.rs` minimal; push method-specific helpers out.

---

## 6) Definition of Done (for refactor work)

- `WsOpSink` is the exclusive high-level response API used by handlers and dispatcher.
- `WsSink` is a minimal transport-only component.
- Connection limits (size, idle, keepalive) are enforced centrally in server loop.
- `WsContext` contains no duplicated transport policy; resources only.
- `protocol.rs` contains only the stable core protocol types.
- Operation tracking does not grow unbounded and correctly reflects terminal outcomes.

---