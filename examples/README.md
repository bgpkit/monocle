# Monocle Examples

Practical examples demonstrating monocle's lens-based API. Each lens has one example showing its primary use case.

## Quick Start

```bash
cargo run --example <name> --features lib
```

## Lens Examples

| Example | Lens | Description |
|---------|------|-------------|
| `time_lens` | TimeLens | Parse timestamps from various formats |
| `country_lens` | CountryLens | Country code/name lookup |
| `ip_lens` | IpLens | IP address information (ASN, RPKI, geolocation) |
| `parse_lens` | ParseLens | Parse MRT files with filters |
| `search_lens` | SearchLens | Search BGP messages via broker |
| `rpki_lens` | RpkiLens | RPKI validation for prefixes |
| `pfx2as_lens` | Pfx2asLens | Prefix-to-ASN mapping lookups |
| `as2rel_lens` | As2relLens | AS-level relationship queries |
| `inspect_lens` | InspectLens | Unified AS/prefix inspection |

## Other Examples

| Example | Description |
|---------|-------------|
| `database` | Low-level database operations |
| `ws_client_all` | WebSocket client demo |

## Usage

All examples use the `lib` feature:

```bash
# Time parsing
cargo run --example time_lens --features lib

# RPKI validation
cargo run --example rpki_lens --features lib

# Unified inspection
cargo run --example inspect_lens --features lib
```

## Common Pattern

```rust
use monocle::database::MonocleDatabase;
use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs};

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = RpkiLens::new(&db);
let result = lens.validate("1.1.1.0/24", 13335)?;
```

See individual example files for complete working code.
