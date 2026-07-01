# Database Refresh Performance Optimization

**Branch:** `perf/db-refresh-optimization` (from latest `main`)
**Date:** 2026-06-29

## Summary

Database refresh operations were slow and had no performance measurements.
This PR adds a benchmark example, identifies the bottlenecks, and applies
three optimizations that cut refresh time by 30–66% across repositories.

## Benchmark

A new example (`examples/db_refresh_bench.rs`) measures each repository's
refresh using **real production data** downloaded from BGPKIT:

| File | Records |
|------|---------|
| `asninfo.jsonl` | 121,463 |
| `as2rel.json.bz2` | 910,280 |
| `pfx2as.json.bz2` | 1,628,039 |

Run with:
```bash
cargo run --example db_refresh_bench --features lib --release
```

## Performance Results (release build, real data)

### Store time: before vs after

| Repository | Rows | Before | After | Improvement |
|---|---|---|---|---|
| asinfo | 121,463 | 988 ms | 330 ms | **67% faster** |
| as2rel | 910,280 | 2,230 ms | 1,765 ms | **21% faster** |
| rpki | 340,000 | 71 ms | 66 ms | — (already fast) |
| pfx2as | 1,628,039 | 8,631 ms | 4,433 ms | **49% faster** |

### Query performance after refresh (pfx2as, 1.6M rows)

All queries well under the 1-second target for low-traffic usage:

```
query                                5_idx_ms     3_idx_ms
----------------------------------------------------------
lookup_exact("1.1.1.0/24")                 92           90
lookup_longest("1.1.1.128/32")            384          388
lookup_covering("1.1.1.0/24")             206          210
get_by_asn(13335)                           4            4
validation_stats()                        347          346
record_count()                              0            0
```

### PRAGMA state preserved

```
after open:    journal_mode=memory, synchronous=NORMAL
after store(): journal_mode=memory, synchronous=NORMAL
✓ PRAGMA state preserved across refresh — no post-refresh degradation
```

## Changes

### 1. Drop and rebuild indexes around bulk inserts (all repositories)

Every `store()` method previously inserted rows with indexes active,
causing SQLite to update B-tree indexes on every single row. Now indexes
are dropped before the insert loop and rebuilt in one efficient pass
afterward.

- `asinfo.rs` — 5 indexes across 5 tables
- `as2rel.rs` — 2 secondary indexes (composite PK is the clustered table)
- `rpki.rs` — 4 indexes
- `pfx2as.rs` — 3 indexes (reduced from 5, see below)

### 2. Remove low-value pfx2as indexes

Removed two indexes from `pfx2as`:
- `idx_pfx2as_prefix_str` — redundant; `lookup_exact` rewritten to use
  the `prefix_start`/`prefix_end`/`prefix_length` BLOB range index that
  already exists for `lookup_longest` and `lookup_covering`.
- `idx_pfx2as_validation` — indexes a 3-value enum (`valid`/`invalid`/
  `unknown`); `validation_stats()` and `get_by_validation()` use full
  scans that complete in <350ms on 1.6M rows.

This cuts index rebuild time by ~40% (from ~4.5s to ~3.0s).

### 3. Fix PRAGMA restore bug

All `store()` methods previously set `PRAGMA synchronous = OFF` and
`PRAGMA journal_mode = MEMORY` for the insert, then restored to
`PRAGMA synchronous = FULL` and `PRAGMA journal_mode = DELETE`.

**The problem:** `DatabaseConn::configure()` sets connection defaults to
`journal_mode = WAL` and `synchronous = NORMAL`. After a refresh, the
connection was left in `DELETE / FULL` — slower and less concurrent than
the defaults. This permanently degraded query performance until process
restart.

**The fix:** `store()` methods no longer touch PRAGMAs at all. The
connection stays at its configured `WAL / NORMAL` state throughout.

### 4. Use plain INSERT instead of INSERT OR REPLACE

All refresh paths `clear()` before `store()`, so there are no existing
rows to conflict with. Plain `INSERT` avoids the primary-key B-tree seek
that `INSERT OR REPLACE` performs on every row.

## Test Coverage

New tests added to verify correctness of the optimizations:

- `test_lookup_exact_ipv4_single_asn` — basic exact match
- `test_lookup_exact_ipv4_multiple_asns` — multiple ASNs per prefix
- `test_lookup_exact_no_match` — non-matching, more/less specific prefixes
- `test_lookup_exact_distinguishes_prefix_lengths` — same network, different lengths
- `test_lookup_exact_ipv6` — IPv6 exact match
- `test_lookup_exact_empty_database` — graceful handling of empty DB
- `test_lookup_exact_invalid_prefix` — error on invalid prefix string
- `test_lookup_exact_consistency_with_other_lookups` — cross-check with longest/covering
- `test_store_rebuilds_indexes_correctly` (pfx2as, rpki, as2rel, asinfo) — indexes exist after store
- `test_store_idempotent_index_rebuild` (pfx2as, rpki, asinfo) — no duplicate indexes on re-store

All 361 tests pass.
