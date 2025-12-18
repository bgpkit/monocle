# Monocle Examples

This directory contains examples demonstrating how to use monocle as a library.
Examples are organized by the features they require, helping you understand
the minimum dependencies needed for different use cases.

## Running All Examples

You can run these examples using `cargo run --example <name>`.

```bash
# Standalone utilities
cargo run --release --example time_parsing --features lens-core
cargo run --release --example output_formats --features lens-core

# Database operations
cargo run --release --example database_basics --features database
cargo run --release --example as2rel_queries --features database
cargo run --release --example pfx2as_search --features lens-bgpkit

# BGP operations
cargo run --release --example country_lookup --features lens-bgpkit
cargo run --release --example rpki_validation --features lens-bgpkit
cargo run --release --example mrt_parsing --features lens-bgpkit
cargo run --release --example search_bgp_messages --features lens-bgpkit

# Full functionality
cargo run --release --example inspect_unified --features lens-full
cargo run --release --example progress_callbacks --features lens-full

# WebSocket Client (requires running server)
# cargo run --example ws_client_all --features cli
```

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

**Files:**
- `standalone/time_parsing.rs` - Parse timestamps, convert formats
- `standalone/output_formats.rs` - Work with OutputFormat enum

### Database Examples (`database`)

SQLite operations without lens overhead.

**Files:**
- `database/database_basics.rs` - MonocleDatabase, schema management
- `database/as2rel_queries.rs` - Query AS-level relationships
- `database/pfx2as_search.rs` - Prefix-to-ASN mapping and search (requires `lens-bgpkit`)

### BGPKIT Examples (`lens-bgpkit`)

Full BGP functionality with bgpkit-* integration.

**Files:**
- `bgpkit/country_lookup.rs` - Country code/name lookup
- `bgpkit/rpki_validation.rs` - RPKI ROA validation
- `bgpkit/mrt_parsing.rs` - Parse MRT files with filters
- `bgpkit/search_bgp_messages.rs` - Search BGP announcement messages (Real-world example)

### Full Examples (`lens-full`)

All lenses including unified inspection.

**Files:**
- `full/inspect_unified.rs` - InspectLens for unified lookups
- `full/progress_callbacks.rs` - Progress tracking for GUI/CLI

### CLI/Server Examples (`cli`)

Examples requiring the full CLI/Server feature set.

**Files:**
- `ws_client_all.rs` - WebSocket client demonstrating all API methods

## Using in Your Project

### Minimal Database Access

```toml
[dependencies]
monocle = { version = "1.0", default-features = false, features = ["database"] }
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
monocle = { version = "1.0", default-features = false, features = ["lens-core"] }
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
monocle = { version = "1.0", default-features = false, features = ["lens-bgpkit"] }
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
monocle = { version = "1.0", default-features = false, features = ["lens-full"] }
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

## Notes

- Examples with network operations (RPKI, search) require internet access
- First run may take time to download/bootstrap data
- Use `--release` for better performance with large datasets
- Database operations use WAL mode for concurrent access
