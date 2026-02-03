# Monocle Examples

This directory contains examples demonstrating how to use monocle as a library.
Examples are organized by the features they require, helping you understand
the minimum dependencies needed for different use cases.

## Running All Examples

You can run these examples using `cargo run --example <name>`.

```bash
# Standalone utilities
cargo run --release --example time_parsing --features lib
cargo run --release --example output_formats --features lib

# Database operations
cargo run --release --example database_basics --features lib
cargo run --release --example as2rel_queries --features lib
cargo run --release --example pfx2as_search --features lib

# BGP operations
cargo run --release --example country_lookup --features lib
cargo run --release --example rpki_validation --features lib
cargo run --release --example mrt_parsing --features lib
cargo run --release --example search_bgp_messages --features lib

# Full functionality
cargo run --release --example inspect_unified --features lib
cargo run --release --example progress_callbacks --features lib

# WebSocket Client (requires running server)
# cargo run --example ws_client_all --features cli
```

## Feature Tiers

Monocle uses a simplified 3-feature system:

| Feature | Description | Key Dependencies |
|---------|-------------|------------------|
| `lib` | Complete library (database + all lenses + display) | `bgpkit-*`, `rusqlite`, `rayon`, `tabled` |
| `server` | WebSocket server (implies lib) | `axum`, `tokio` |
| `cli` | Full CLI binary (implies lib and server) | `clap` + all above |

**Quick Guide:**
- **Need the CLI binary?** Use `cli` feature (includes everything)
- **Need WebSocket server without CLI?** Use `server` feature (includes lib)
- **Need only library/data access?** Use `lib` feature (all examples below)

## Examples by Feature

### Standalone Examples (`lib`)

Minimal dependencies - time parsing and output formatting utilities.

**Files:**
- `standalone/time_parsing.rs` - Parse timestamps, convert formats
- `standalone/output_formats.rs` - Work with OutputFormat enum

### Database Examples (`lib`)

SQLite operations with database repositories.

**Files:**
- `database/database_basics.rs` - MonocleDatabase, schema management
- `database/as2rel_queries.rs` - Query AS-level relationships
- `database/pfx2as_search.rs` - Prefix-to-ASN mapping and search

### BGPKIT Examples (`lib`)

Full BGP functionality with bgpkit-* integration.

**Files:**
- `bgpkit/country_lookup.rs` - Country code/name lookup
- `bgpkit/rpki_validation.rs` - RPKI ROA validation
- `bgpkit/mrt_parsing.rs` - Parse MRT files with filters
- `bgpkit/search_bgp_messages.rs` - Search BGP announcement messages (Real-world example)

### Full Examples (`lib`)

All lenses including unified inspection.

**Files:**
- `full/inspect_unified.rs` - InspectLens for unified lookups
- `full/progress_callbacks.rs` - Progress tracking for GUI/CLI

### CLI/Server Examples (`cli`, `server`)

Examples requiring the CLI or Server feature set.

**Files:**
- `ws_client_all.rs` - WebSocket client demonstrating all API methods (requires `cli`)

## Using in Your Project

See the main [README](../README.md) for dependency configuration with version numbers and feature tiers.

### Minimal Database Access

```rust
use monocle::database::MonocleDatabase;
use std::time::Duration;

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let ttl = Duration::from_secs(24 * 60 * 60); // 24 hours
if db.needs_as2rel_refresh(ttl) {
    db.update_as2rel()?;
}
let rels = db.as2rel().search_asn(13335)?;
```

### Standalone Utilities

```rust
use monocle::lens::time::{TimeLens, TimeParseArgs};

let lens = TimeLens::new();
let args = TimeParseArgs::new(vec!["2024-01-01T00:00:00Z".to_string()]);
let results = lens.parse(&args)?;
```

### BGP Operations

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
