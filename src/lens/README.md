# Lens Module

The `lens` module is Monocle’s **use‑case layer**. Each lens exposes a cohesive set of operations (search, parse, RPKI lookup, etc.) through a stable, programmatic API that can be reused by:

- the `monocle` CLI,
- GUI frontends (planned: GPUI),
- other Rust applications embedding Monocle as a library.

A lens is responsible for **domain logic**, **input normalization/validation**, and returning **typed results**. Formatting is handled consistently via the unified `OutputFormat`.

---

## Directory Layout

```
lens/
├── mod.rs              # Lens module exports
├── README.md           # This document
├── utils.rs            # OutputFormat + formatting helpers
│
├── as2org/             # AS→Org lookups (database-backed)
│   ├── mod.rs
│   ├── args.rs
│   └── types.rs
├── as2rel/             # AS relationship lookups (database-backed)
│   ├── mod.rs
│   ├── args.rs
│   └── types.rs
│
├── country.rs          # Country lookup (in-memory)
├── ip/                 # IP information lookups
│   └── mod.rs
├── parse/              # MRT parsing (+ progress callbacks)
│   └── mod.rs
├── pfx2as/             # Prefix→AS mapping
│   └── mod.rs
├── rpki/               # RPKI operations (current + historical sources)
│   ├── mod.rs
│   └── commons.rs
├── search/             # Search across public MRT files (+ progress callbacks)
│   ├── mod.rs
│   └── query_builder.rs
└── time/               # Time parsing / formatting utilities
    └── mod.rs
```

---

## Design Philosophy

### 1) Interface-neutral, frontend-friendly

Lenses are designed to be called from multiple frontends:

- **CLI**: commands call into lenses and print results using `OutputFormat`.
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

- `As2orgLens`
- `As2relLens`

(Some RPKI operations also depend on the database for current-data caching, depending on the operation path.)

### Standalone lenses

These lenses do not require a persistent database reference:

- `TimeLens`
- `CountryLens`
- `IpLens`
- `ParseLens`
- `SearchLens`
- `Pfx2asLens`
- `RpkiLens` (historical data paths can be fully standalone; current data may use the DB cache noted above)

---

## Usage Examples

> Note: code below is intentionally example-focused; check the module docs / rustdoc for exact function signatures where needed.

### TimeLens

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

### As2orgLens (database-backed)

```rust,ignore
use monocle::database::MonocleDatabase;
use monocle::lens::as2org::{As2orgLens, As2orgSearchArgs};
use monocle::lens::utils::OutputFormat;

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = As2orgLens::new(&db);

if lens.needs_bootstrap() {
    lens.bootstrap()?;
}

let args = As2orgSearchArgs::new("cloudflare");
let results = lens.search(&args)?;
println!("{}", OutputFormat::JsonPretty.format(&results));
```

### SearchLens with progress (GUI/CLI-friendly)

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
3. Wire into:
   - `src/lens/mod.rs` exports
   - CLI command module under `src/bin/commands/` (optional)

---

## Naming Conventions

Consistent naming makes lenses predictable:

- Lens struct: `<Name>Lens` (e.g., `TimeLens`, `RpkiLens`)
- Arg structs: `<Name><Op>Args` (e.g., `As2orgSearchArgs`, `RpkiRoaLookupArgs`)
- Result structs: `<Name><Op>Result` (e.g., `As2orgSearchResult`)
- Module names: snake_case (`as2org`, `pfx2as`)

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
- `src/database/README.md` (database module overview)