# Monocle Examples

Practical examples demonstrating how to use monocle as a library.

## Quick Start

All examples use the `lib` feature:

```bash
cargo run --example <name> --features lib
```

## Examples

### Basic Utilities

- **`time_parsing`** - Parse timestamps from various formats
- **`output_formats`** - Work with OutputFormat enum

### Database Operations

- **`database`** - Basic database operations and queries

### BGP Operations

- **`country_lookup`** - Country code/name lookup
- **`rpki_validation`** - RPKI validation for prefixes
- **`mrt_parsing`** - Parse MRT files with filters
- **`search_bgp_messages`** - Search BGP messages via broker

### Unified Inspection

- **`inspect`** - Combined AS/prefix information lookup

### WebSocket Client

- **`ws_client_all`** - WebSocket client demo (requires `cli` feature)

## Feature Guide

- **`lib`** - Library only (all examples above)
- **`server`** - Library + WebSocket server
- **`cli`** - Everything including CLI binary

## Common Patterns

```rust
// Database operations
use monocle::database::MonocleDatabase;
let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let rels = db.as2rel().search_asn(13335)?;

// RPKI validation
use monocle::lens::rpki::RpkiLens;
let lens = RpkiLens::new(&db);
let result = lens.validate("1.1.1.0/24", 13335)?;

// Unified inspection
use monocle::lens::inspect::{InspectLens, InspectQueryOptions};
let lens = InspectLens::new(&db, &config);
let result = lens.query_as_asn(&["13335".to_string()], &options)?;
```

See individual example files for complete working code.
