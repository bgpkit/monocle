# SSE Search Terminal Statistics Design (#135)

## 1. Overview

Extend `POST /api/v1/search/stream` so every terminal SSE event reports an unambiguous summary of the search work and its results. The server already emits a `completed` event containing `SearchSummary`, but it does not identify matching sources, source-byte totals, throughput, or whether a counter means matched elements versus all BGP messages parsed.

The design preserves the bounded-channel, cancellation, max-result, timeout, and one-terminal-event guarantees of the current shared search executor.

## 2. Motivation and Use Cases

- A UI can show a final, trustworthy “N matching elements from M files” result without reconstructing it from intermittent progress events.
- Operators can compare searches by source data volume and matched-element rate.
- Clients can retain the collectors and files that actually produced matches for audit, download, or follow-up analysis.
- A client can distinguish a successful empty search from an incomplete/cancelled search.

| Aspect | Current terminal event | Proposed terminal event |
|---|---|---|
| Result count | `total_messages` (actually matched elements) | Explicit `matched_elements` |
| File outcome | totals only | totals plus matching source list |
| Source volume | unavailable | broker-advertised compressed bytes |
| Rate | unavailable | matched elements/sec |
| Stop state | separate event name | event name plus `exit_reason` and partial metrics |

## 3. Design Decisions

**Use a server-only `SearchStreamStats` payload instead of changing the public `SearchSummary` struct.** `SearchSummary` is a library API used by CLI and lens consumers. The additional source metadata is meaningful only to SSE and should not force unrelated callers to carry server accounting state.

**Call the existing `total_messages` counter `matched_elements` at the API boundary.** The shared executor increments it only when it reserves output slots for filtered `BgpElem`s. Calling it “messages processed” would be false whenever filters exclude data or a BGP message yields multiple elements.

**Report broker-advertised compressed source bytes, with availability metadata.** Sum `BrokerItem.exact_size` when positive and otherwise `rough_size`; call this `source_bytes_compressed`. It is the amount of source material selected, not a count of bytes actually downloaded before cancellation. Do not report uncompressed bytes in this release: the current parser/reader API exposes no reliable decompressed-byte counter.

**Do not claim a raw BGP-message count in the first implementation.** `BgpkitParser` exposes filtered element iteration but no parser-work counter. Counting every record would require either upstream parser instrumentation or reimplementing filtering in Monocle. The terminal payload therefore has no `messages_processed` field; a future additive field may expose it once parser support exists.

**Report matched files and collectors only.** A source is included after its first accepted element batch. This gives clients useful follow-up sources without emitting a potentially enormous list of all queried files. Ordering is deterministic: sort collectors and source records at terminal serialization.

**Attach statistics to all final stream-result payloads.** `completed`, `cancelled`, and `error` retain their distinct SSE event names and remain mutually exclusive. They each carry a `SearchStreamResult` object. This replaces the current `null` cancelled payload and bare API-error payload; document it as a v1 additive/shape adjustment before implementation. The terminal payload makes partial work observable for cancellation, timeout, and execution errors.

**Keep collection in the SSE sink and immutable source plan in the executor outcome.** The sink already receives accepted batches and is shared by Rayon workers, so a mutex-protected set is the correct point to record matches. The executor owns broker items and can calculate selected source bytes before parallel processing.

## 4. Data Structures

Add these server-facing types in `src/server/search.rs`:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct SearchStreamResult {
    pub exit_reason: SearchExitReason,
    pub stats: SearchStreamStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiErrorResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchStreamStats {
    /// Filtered BGP elements accepted for SSE output; not raw BGP messages.
    pub matched_elements: u64,
    pub total_files: usize,
    pub successful_files: usize,
    pub failed_files: usize,
    /// Broker-advertised compressed bytes for all selected source files.
    pub source_bytes_compressed: u64,
    /// Always false in this version; retained as explicit semantic metadata.
    pub source_bytes_exact: bool,
    pub duration_secs: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_elements_per_sec: Option<f64>,
    pub matching_collectors: Vec<String>,
    pub matching_files: Vec<MatchingFile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct MatchingFile {
    pub collector: String,
    pub file_url: String,
}
```

Extend the lens outcome internally, without altering `SearchSummary`:

```rust
pub struct SearchOutcome {
    pub summary: SearchSummary,
    pub exit_reason: SearchExitReason,
    pub source_bytes_compressed: u64,
    pub source_bytes_exact: bool,
}
```

`source_bytes_exact` is true only when every selected `BrokerItem.exact_size > 0`; when false, `rough_size` was used for one or more files. The sum remains useful but must not be interpreted as measured transferred bytes.

Replace the SSE sink state with:

```rust
struct SseSearchState {
    matching_files: BTreeSet<MatchingFile>,
}
```

A `BTreeSet` deduplicates batches from the same file and supplies deterministic output. `total_so_far` is removed from sink state; `SearchElementBatch`/the executor’s atomic matched-elements counter is the source of truth. Add `total_so_far` to the batch at emission time or continue deriving it via a dedicated atomic passed to the sink so element events remain monotonically counted under parallel processing.

## 5. Algorithm

1. `SearchLens::search_with_options` queries the broker as it does today.
2. Before moving `items` into Rayon, calculate `source_bytes_compressed` and `source_bytes_exact` from the selected `BrokerItem`s.
3. Process each item with the current parser/filter pipeline. Do **not** add a second unfiltered parser pass.
4. When `emit_search_batch` reserves at least one result slot, it invokes `SseSearchSink::on_elements`.
5. The SSE sink locks `SseSearchState`, inserts `{collector, file_url}` into `matching_files`, and sends the element event. If the channel is closed, it sets cancellation and returns `Stop`, exactly as today.
6. On exit, `run_search_worker` takes a snapshot of the sink’s sorted matching set and combines it with `SearchOutcome` and elapsed duration to make `SearchStreamStats`.
7. Emit exactly one terminal event:
   - normal completion or `max_results` → `event: completed`, `exit_reason: "completed"` or `"max_results_reached"`;
   - client disconnect/sink stop → `event: cancelled`, `exit_reason: "cancelled"`;
   - timeout or executor failure → `event: error`, with an `error` object and `exit_reason: "timeout"` or `"error"`.
8. Do not send progress or element events after the terminal send. Existing `send_event` blocking semantics remain unchanged.

Worked example:

- Broker selects three files totaling 120 MB (100 MB exact + 20 MB rough), from `rrc00` and `rrc01`.
- Filters accept elements only from one `rrc00` file, producing two batches and 150 elements.
- The terminal stats contain `matched_elements: 150`, `matching_collectors: ["rrc00"]`, one matching-file record, `source_bytes_compressed: 125829120`, `source_bytes_exact: false`, and `matched_elements_per_sec: 75.0` for a two-second run.

## 6. Output Format

Successful terminal event:

```text
event: completed
data: {"exit_reason":"completed","stats":{"matched_elements":150,"total_files":3,"successful_files":3,"failed_files":0,"source_bytes_compressed":125829120,"source_bytes_exact":false,"duration_secs":2.0,"matched_elements_per_sec":75.0,"matching_collectors":["rrc00"],"matching_files":[{"collector":"rrc00","file_url":"https://data.ris.ripe.net/rrc00/2026.06/updates.20260601.0000.gz"}]}}
```

Cancelled terminal event (partial work is retained):

```text
event: cancelled
data: {"exit_reason":"cancelled","stats":{"matched_elements":64,"total_files":10,"successful_files":1,"failed_files":1,"source_bytes_compressed":524288000,"source_bytes_exact":true,"duration_secs":1.4,"matched_elements_per_sec":45.7,"matching_collectors":["rrc00"],"matching_files":[...]}}
```

No file is written; this is the terminal frame of the existing SSE stream.

## 7. Changes to Existing Files

### `src/lens/search/mod.rs`

- Extend `SearchOutcome` with source-byte fields shown above.
- Immediately after `let items = self.query_broker(filters)?`, calculate:

```rust
let source_bytes_exact = items.iter().all(|item| item.exact_size > 0);
let source_bytes_compressed = items.iter().map(|item| {
    u64::try_from(if item.exact_size > 0 { item.exact_size } else { item.rough_size })
        .unwrap_or(0)
}).sum();
```

- Include those values in every `SearchOutcome` return, including timeout-before-work and zero-file paths.
- Keep `SearchSummary.total_messages` unchanged for library compatibility; document in its Rustdoc that it is matched elements.

### `src/server/search.rs`

- Replace `Completed(SearchSummary)`, `Cancelled`, and `Error(ApiErrorResponse)` in `SearchStreamEvent` with final-result variants carrying `SearchStreamResult`.
- Add `SearchStreamResult`, `SearchStreamStats`, and `MatchingFile`.
- Make `SseSearchSink::snapshot_matching_files()` clone its sorted set after executor completion.
- Build terminal stats in `run_search_worker`; derive `matched_elements_per_sec` from `outcome.summary.total_messages / duration_secs` when duration is positive.
- Preserve `Started`, `Progress`, `Elements`, bounded channel capacity, and `send_event` behavior.
- Update SSE serialization tests for all terminal event names and payloads.

### `src/server/README.md`

- Replace the final-event table payload descriptions with `SearchStreamResult`.
- Define `matched_elements`, selected-source compressed bytes, `source_bytes_exact`, and matching-source semantics.
- State that raw BGP-message and uncompressed-byte counters are not yet reported.
- Update the terminal invariant wording to cover terminal objects for cancellation and errors.

### `CHANGELOG.md`

Add an Unreleased **New Features** item describing terminal SSE statistics and explicitly noting matching-element semantics.

## 8. New Files

No production files are required. The feature fits the existing server search module and shared lens executor.

Optionally add `tests/sse_search_terminal.rs` if the project’s server test harness can construct deterministic local MRT input; otherwise keep unit tests adjacent to `src/server/search.rs` and executor tests in `src/lens/search/mod.rs`.

## 9. Unit Tests

- Selected items with all positive `exact_size` values → exact compressed-byte total and `source_bytes_exact: true`.
- A selected item with no exact size → `rough_size` used and `source_bytes_exact: false`.
- Zero matching batches → empty collectors/files and zero matched elements.
- Two batches from the same file → one matching-file entry.
- Batches from two collectors arrive in reverse order → terminal collectors/files are sorted.
- Normal completion → one `completed` terminal frame with stats.
- Max-results exit → one `completed` frame with `max_results_reached` reason and capped matched elements.
- Sink/channel closure → one `cancelled` terminal frame with partial stats and no later event.
- Timeout → one `error` terminal frame with partial stats and timeout error.
- Executor error before any file parses → one `error` terminal frame with zero match sources.
- Existing progress and element events remain unchanged and no terminal event is duplicated.
- **CLI end-to-end:** start `monocle server` on an ephemeral local port, run `monocle search --remote-url http://127.0.0.1:<port>/api/v1/search/stream` against it, and assert the client consumes `started`, `elements`, and the final result event without protocol or deserialization errors. Use a deterministic local MRT fixture or mockable broker/parser source; do not make this CI test depend on a live archive.

## 10. Open Questions

**Question:** Should `max_results_reached` have its own terminal SSE event name?

**Default:** No. Keep `completed` as the event name and expose `exit_reason`; clients already treat it as a successful, intentionally bounded result.

**Question:** Should terminal stats include every selected file, not only files with matches?

**Default:** No. `total_files` and byte totals describe selection; `matching_files` is intentionally a concise follow-up list.

**Question:** When can `messages_processed` be added?

**Default:** Only after `bgpkit-parser` exposes a reliable counter for records/messages read before filtering. Do not infer it from matched elements.

## 11. Implementation Sequence

1. Extend `SearchOutcome` and calculate source-byte metadata in `src/lens/search/mod.rs`; add deterministic unit tests.
2. Add final stream-result DTOs and matching-source state to `src/server/search.rs`.
3. Convert terminal event construction and serialization, preserving the one-terminal-event invariant.
4. Add server tests for normal, capped, cancelled, timeout, and error outcomes.
5. Add a CLI end-to-end test that exercises `monocle server` and `monocle search --remote-url` over a local SSE connection using deterministic input.
6. Update `src/server/README.md` and `CHANGELOG.md`.
7. Run `cargo fmt -- --check`, `cargo test --all-features`, and the repository’s applicable clippy command.

## 12. Notes and Caveats

- `source_bytes_compressed` is planned input volume from broker metadata, not a network-transfer meter. It remains the same for a completed and early-cancelled search over the same broker result set.
- The executor currently reports a filtered `BgpElem` stream. A BGP UPDATE can yield more than one element, so neither `matched_elements` nor existing `SearchSummary.total_messages` is a literal BGP message count.
- `matching_files` can be large for broad searches. It is bounded only by the number of selected files; a future request may add a server-configured cap and truncation indicator, but this design does not silently truncate it.
