# Monocle Examples

This directory contains examples demonstrating how to use monocle as a library.
Examples are organized by the features they require, helping you understand
the minimum dependencies needed for different use cases.

## Feature Tiers

Monocle uses a layered feature system:

| Feature | Description | Key Dependencies |
|---------|-------------|------------------|
| `database` | SQLite database operations | `rusqlite`, `oneio`, `ipnet` |
| `lens-core` | Standalone utilities (TimeLens) | `chrono-humanize`, `dateparser` |
| `lens-bgpkit` | BGP-related lenses | `bgpkit-*`, `rayon`, `tabled` |
| `lens-full` | All lenses including InspectLens | All above |
| `display` | Table formatting | `tabled` (included in lens-bgpkit) |
| `cli` | Full CLI binary | All above + `clap`, `axum` |

## Examples by Feature

### Standalone Examples (`lens-core`)

Minimal dependencies - no bgpkit-* crates required.

```bash
# Time parsing and formatting
cargo run --example time_parsing --features lens-core

# Output format utilities
cargo run --example output_formats --features lens-core
```

**Files:**
- `standalone/time_parsing.rs` - Parse timestamps, convert formats
- `standalone/output_formats.rs` - Work with OutputFormat enum

### Database Examples (`database`)

SQLite operations without lens overhead.

```bash
# Basic database operations
cargo run --example database_basics --features database

# AS2Rel relationship queries
cargo run --example as2rel_queries --features database
```

**Files:**
- `database/database_basics.rs` - MonocleDatabase, schema management
- `database/as2rel_queries.rs` - Query AS-level relationships

### BGPKIT Examples (`lens-bgpkit`)

Full BGP functionality with bgpkit-* integration.

```bash
# Country code lookup
cargo run --example country_lookup --features lens-bgpkit

# RPKI validation
cargo run --example rpki_validation --features lens-bgpkit

# MRT file parsing
cargo run --example mrt_parsing --features lens-bgpkit

# BGP message search
cargo run --example bgp_search --features lens-bgpkit
```

**Files:**
- `bgpkit/country_lookup.rs` - Country code/name lookup
- `bgpkit/rpki_validation.rs` - RPKI ROA validation
- `bgpkit/mrt_parsing.rs` - Parse MRT files with filters
- `bgpkit/bgp_search.rs` - Search BGP messages across files

### Full Examples (`lens-full`)

All lenses including unified inspection.

```bash
# Unified AS/prefix inspection
cargo run --example inspect_unified --features lens-full

# Progress callback patterns
cargo run --example progress_callbacks --features lens-full
```

**Files:**
- `full/inspect_unified.rs` - InspectLens for unified lookups
- `full/progress_callbacks.rs` - Progress tracking for GUI/CLI

## Using in Your Project

### Minimal Database Access

```toml
[dependencies]
monocle = { version = "0.9", default-features = false, features = ["database"] }
```

```rust
use monocle::database::MonocleDatabase;

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
if db.needs_as2rel_update() {
    db.update_as2rel()?;
}
let rels = db.as2rel().search_asn(13335)?;
```

### Standalone Utilities

```toml
[dependencies]
monocle = { version = "0.9", default-features = false, features = ["lens-core"] }
```

```rust
use monocle::lens::time::{TimeLens, TimeParseArgs};

let lens = TimeLens::new();
let args = TimeParseArgs::new(vec!["2024-01-01T00:00:00Z".to_string()]);
let results = lens.parse(&args)?;
```

### BGP Operations

```toml
[dependencies]
monocle = { version = "0.9", default-features = false, features = ["lens-bgpkit"] }
```

```rust
use monocle::database::MonocleDatabase;
use monocle::lens::rpki::RpkiLens;

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let mut lens = RpkiLens::new(&db);

if lens.needs_refresh()? {
    lens.refresh()?;
}

let result = lens.validate("1.1.1.0/24", 13335)?;
println!("{}: {}", result.state, result.reason);
```

### Full Functionality

```toml
[dependencies]
monocle = { version = "0.9", default-features = false, features = ["lens-full"] }
```

```rust
use monocle::database::MonocleDatabase;
use monocle::lens::inspect::{InspectLens, InspectQueryOptions};

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = InspectLens::new(&db);

lens.ensure_data_available()?;

let options = InspectQueryOptions::default();
let results = lens.query(&["13335".to_string()], &options)?;
let json = lens.format_json(&results, true);
```

## Running All Examples

```bash
# Run all examples with full features
cargo run --example time_parsing --features cli
cargo run --example database_basics --features cli
cargo run --example country_lookup --features cli
cargo run --example rpki_validation --features cli
cargo run --example inspect_unified --features cli
```

## Notes

- Examples with network operations (RPKI, search) require internet access
- First run may take time to download/bootstrap data
- Use `--release` for better performance with large datasets
- Database operations use WAL mode for concurrent access