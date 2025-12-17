# Lens Module

The `lens` module is Monocle's **use‑case layer**. Each lens exposes a cohesive set of operations (search, parse, RPKI lookup, etc.) through a stable, programmatic API that can be reused by:

- the `monocle` CLI,
- the WebSocket server (`monocle server`),
- GUI frontends (planned: GPUI),
- other Rust applications embedding Monocle as a library.

A lens is responsible for **domain logic**, **input normalization/validation**, and returning **typed results**. Formatting is handled consistently via the unified `OutputFormat`.

---

## Directory Layout

```
lens/
├── mod.rs              # Lens module exports (feature-gated)
├── README.md           # This document
├── utils.rs            # OutputFormat + formatting helpers
│
├── time/               # Time parsing / formatting (lens-core)
│   └── mod.rs
│
├── country.rs          # Country lookup (lens-bgpkit)
├── ip/                 # IP information lookups (lens-bgpkit)
│   └── mod.rs
├── parse/              # MRT parsing + progress callbacks (lens-bgpkit)
│   └── mod.rs
├── search/             # Search across public MRT files (lens-bgpkit)
│   ├── mod.rs
│   └── query_builder.rs
├── rpki/               # RPKI operations (lens-bgpkit)
│   ├── mod.rs
│   └── commons.rs
├── pfx2as/             # Prefix→AS mapping types (lens-bgpkit)
│   └── mod.rs
├── as2rel/             # AS relationship lookups (lens-bgpkit)
│   ├── mod.rs
│   ├── args.rs
│   └── types.rs
│
└── inspect/            # Unified AS/prefix inspection (lens-full)
    ├── mod.rs          # InspectLens implementation
    └── types.rs        # Result types, section selection
```

---

## Feature Tiers

Lenses are organized by feature requirements:

| Feature | Lenses | Key Dependencies |
|---------|--------|------------------|
| `lens-core` | `TimeLens` | chrono, dateparser |
| `lens-bgpkit` | `CountryLens`, `IpLens`, `ParseLens`, `SearchLens`, `RpkiLens`, `Pfx2asLens`, `As2relLens` | bgpkit-*, rayon, tabled |
| `lens-full` | `InspectLens` | All above |

Library users can select minimal features:

```toml
# Time parsing only
monocle = { version = "0.10", default-features = false, features = ["lens-core"] }

# BGP operations without CLI
monocle = { version = "0.10", default-features = false, features = ["lens-bgpkit"] }

# All lenses including InspectLens
monocle = { version = "0.10", default-features = false, features = ["lens-full"] }
```

---

## Design Philosophy

### 1) Interface-neutral, frontend-friendly

Lenses are designed to be called from multiple frontends:

- **CLI**: commands call into lenses and print results using `OutputFormat`.
- **WebSocket**: handlers call lenses and stream progress/results via `WsOpSink`.
- **GUI**: a UI can call lenses on a worker thread and stream progress/results back to the UI.
- **Library**: users can call lens APIs directly without going through CLI argument parsing.

### 2) Clear responsibilities

A typical operation should look like:

- **CLI / GUI**: parse user input → construct *Args* → call lens method
- **Lens**: validate and execute → return typed results (and optionally progress events)
- **Formatting**: done via `OutputFormat` (shared across the project)

The lens layer should **not** own long-term persistence primitives directly. When persistence is needed, lenses depend on `database/` (for SQLite + caches) or external crates (Broker/Parser/etc.).

### 3) Stable, typed APIs

Prefer:
- typed argument structs (often `Serialize`/`Deserialize`)
- typed result structs/enums (often `Serialize`/`Deserialize`)
- explicit error returns (typically `anyhow::Result<_>`)

This makes it straightforward to:
- serialize requests/results for GUI message passing,
- test units without a CLI,
- keep output formatting consistent.

---

## Output Formatting: Unified `OutputFormat`

Monocle uses a single output format enum across commands and lenses:

- `table` (default)
- `markdown` / `md`
- `json`
- `json-pretty`
- `json-line` / `jsonl` / `ndjson`
- `psv`

The `OutputFormat` type lives in:

- `lens/utils.rs`

A common pattern is:

- lens methods return `Vec<T>` (or similar)
- the CLI chooses an `OutputFormat`
- formatting is done by calling `format.format(&results)` (or equivalent helper)

This keeps formatting logic consistent and avoids per-command format flags drifting over time.

---

## Progress Reporting (GUI-friendly)

Some lenses can emit progress updates via callbacks (used by CLI progress bars today and intended for GUIs):

- **Parse**: emits periodic progress while processing messages.
- **Search**: emits progress for broker querying, file processing, and completion.

Progress types are designed to be:
- `Send + Sync` friendly (callbacks may be called from parallel workers),
- serializable (`Serialize`/`Deserialize`) for easy GUI integration.

---

## Lens Categories

### Database-backed lenses

These lenses require access to the persistent SQLite database (typically via `MonocleDatabase`):

- `As2relLens` - AS-level relationships
- `InspectLens` - Unified AS/prefix inspection (uses ASInfo, AS2Rel, RPKI, Pfx2as repositories)

### Database-optional lenses

These lenses can use the database for caching but don't strictly require it:

- `RpkiLens` - Uses database for current data cache; historical queries use bgpkit-commons directly

### Standalone lenses

These lenses do not require a persistent database reference:

- `TimeLens` - Time parsing and formatting
- `CountryLens` - Country code/name lookup (uses bgpkit-commons)
- `IpLens` - IP information lookup (uses external API)
- `ParseLens` - MRT file parsing
- `SearchLens` - BGP message search across MRT files

---

## Usage Examples

> Note: code below is intentionally example-focused; check the module docs / rustdoc for exact function signatures where needed.

### TimeLens (lens-core)

```rust,ignore
use monocle::lens::time::{TimeLens, TimeParseArgs};
use monocle::lens::utils::OutputFormat;

let lens = TimeLens::new();
let args = TimeParseArgs::new(vec![
    "1697043600".to_string(),
    "2023-10-11T00:00:00Z".to_string(),
]);

let results = lens.parse(&args)?;
let out = OutputFormat::Table.format(&results);
println!("{}", out);
```

### InspectLens (lens-full, database-backed)

```rust,ignore
use monocle::database::MonocleDatabase;
use monocle::lens::inspect::{InspectLens, InspectQueryOptions};
use monocle::lens::utils::OutputFormat;

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = InspectLens::new(&db);

// Query AS information
let options = InspectQueryOptions::default();
let result = lens.query_asn(13335, &options)?;
println!("AS{}: {}", result.asn, result.name.unwrap_or_default());

// Query prefix information
let result = lens.query_prefix("1.1.1.0/24".parse()?, &options)?;
println!("{:?}", result);

// Search by name
let results = lens.search_by_name("cloudflare", 20)?;
for r in results {
    println!("AS{}: {}", r.asn, r.name.unwrap_or_default());
}
```

### As2relLens (lens-bgpkit, database-backed)

```rust,ignore
use monocle::database::MonocleDatabase;
use monocle::lens::as2rel::{As2relLens, As2relSearchArgs};
use monocle::lens::utils::OutputFormat;

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = As2relLens::new(&db);

// Update data if needed
if lens.needs_update() {
    lens.update()?;
}

// Query relationships for an ASN
let args = As2relSearchArgs::new(13335);
let results = lens.search(&args)?;
println!("{}", OutputFormat::Table.format(&results));
```

### SearchLens with progress (lens-bgpkit)

```rust,ignore
use monocle::lens::search::{SearchLens, SearchFilters, SearchProgress};
use std::sync::Arc;

let lens = SearchLens::new();
let filters = SearchFilters {
    // ...
    ..Default::default()
};

let on_progress = Arc::new(|p: SearchProgress| {
    // send to UI / update progress bar
    eprintln!("{:?}", p);
});

let on_elem = Arc::new(|elem, collector| {
    // stream results somewhere
});

let summary = lens.search_with_progress(&filters, Some(on_progress), on_elem)?;
eprintln!("{:?}", summary);
```

---

## Adding a New Lens (high level)

For a detailed contributor walkthrough, see `DEVELOPMENT.md`. In short:

1. Create `src/lens/<newlens>/mod.rs` (and `args.rs` / `types.rs` if needed)
2. Define:
   - `<NewLens>Args` (input)
   - `<NewLens>Result` (output)
   - `<NewLens>Lens` (operations)
3. Add feature gate in `src/lens/mod.rs`:
   ```rust
   #[cfg(feature = "lens-bgpkit")]  // or appropriate feature
   pub mod newlens;
   ```
4. Wire into (optional):
   - CLI command module under `src/bin/commands/`
   - WebSocket handler under `src/server/handlers/`

---

## Naming Conventions

Consistent naming makes lenses predictable:

- Lens struct: `<Name>Lens` (e.g., `TimeLens`, `RpkiLens`, `InspectLens`)
- Arg structs: `<Name><Op>Args` (e.g., `As2relSearchArgs`, `RpkiRoaLookupArgs`)
- Result structs: `<Name><Op>Result` (e.g., `As2relSearchResult`)
- Module names: snake_case (`as2rel`, `pfx2as`, `inspect`)

---

## Error Handling

- Lens methods generally return `anyhow::Result<T>`.
- Favor descriptive messages that help CLI and GUI users.
- Avoid panics; the library should be robust when called from long-running frontends.

---

## Testing Notes

- Prefer unit tests close to lens code for pure logic.
- For filesystem/network interactions, use `#[ignore]` tests or inject test data.
- Search/parse workloads should be tested with small fixtures where possible.

---

## Related Documentation

- `ARCHITECTURE.md` (project-level architecture)
- `DEVELOPMENT.md` (contributor guide)
- `src/server/README.md` (WebSocket API protocol)
- `src/database/README.md` (database module overview)
- `examples/README.md` (example code by feature tier)