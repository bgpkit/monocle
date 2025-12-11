# Lens Module

The lens module provides high-level business logic with output formatting. Lenses are designed to be **reusable across different interfaces** (CLI, REST API, WebSocket, GUI).

## Architecture

```
lens/
├── as2org/         # AS-to-Organization lookup lens
│   ├── mod.rs      # As2orgLens implementation
│   ├── args.rs     # Reusable argument structs
│   └── types.rs    # Result types and enums
│
├── as2rel/         # AS-level relationship lens
│   ├── mod.rs      # As2relLens implementation
│   ├── args.rs     # Reusable argument structs
│   └── types.rs    # Result types and enums
│
├── country.rs      # Country lookup lens (in-memory)
│
├── ip/             # IP information lookup lens
│   └── mod.rs      # IpLens with types and args
│
├── parse/          # MRT file parsing lens
│   └── mod.rs      # ParseLens with ParseFilters
│
├── pfx2as/         # Prefix-to-ASN mapping lens
│   └── mod.rs      # Pfx2asLens with types and args
│
├── rpki/           # RPKI validation and data lens
│   ├── mod.rs      # RpkiLens implementation
│   ├── commons.rs  # bgpkit-commons integration (ROA/ASPA data)
│   └── validator.rs # Cloudflare GraphQL API integration
│
├── search/         # BGP message search lens
│   └── mod.rs      # SearchLens with SearchFilters
│
└── time/           # Time parsing and formatting lens
    └── mod.rs      # TimeLens with types and args
```

## Design Philosophy

### Separation of Concerns

```
┌─────────────────────────────────────────────────────────────┐
│                     Interface Layer                         │
│  CLI (commands/)  │  REST API  │  WebSocket  │  GUI (GPUI)  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      Lens Layer                             │
│  - Business logic                                           │
│  - Input validation                                         │
│  - Output formatting                                        │
│  - Progress reporting                                       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    Database Layer                           │
│  - Data storage (for lenses that need it)                   │
│  - Queries                                                  │
│  - Schema management                                        │
└─────────────────────────────────────────────────────────────┘
```

### Reusable Argument Structs

Argument structs are designed to work in multiple contexts:

```rust
// In args - works for CLI, API, and programmatic use
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]  // Only derive Args when CLI is enabled
pub struct TimeParseArgs {
    #[cfg_attr(feature = "cli", clap(value_name = "TIME"))]
    pub times: Vec<String>,
    
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: TimeOutputFormat,
}
```

This pattern enables:
- **CLI**: Direct use with clap for argument parsing
- **REST API**: Deserialize from query parameters
- **WebSocket**: Deserialize from JSON messages
- **Library**: Construct programmatically

## Lens Categories

### Database-Backed Lenses

These lenses require a `MonocleDatabase` reference:

- **As2orgLens**: AS-to-Organization lookup using CAIDA's AS2Org data
- **As2relLens**: AS-level relationship data from BGPKIT

### Standalone Lenses

These lenses are self-contained and don't require database access:

- **TimeLens**: Time parsing and formatting
- **IpLens**: IP information lookup via BGPKIT API
- **Pfx2asLens**: Prefix-to-ASN mapping using trie data structure
- **RpkiLens**: RPKI validation and ROA/ASPA data lookup
- **CountryLens**: Country code/name lookup (in-memory)
- **ParseLens**: MRT file parsing
- **SearchLens**: BGP message search across multiple MRT files

## Usage Examples

### TimeLens

Time parsing and formatting for BGP-related timestamps.

```rust
use monocle::lens::time::{TimeLens, TimeParseArgs, TimeOutputFormat};

let lens = TimeLens::new();

// Parse timestamps
let args = TimeParseArgs::new(vec!["1697043600".to_string(), "2023-10-11T00:00:00Z".to_string()]);
let results = lens.parse(&args)?;

// Format output
let output = lens.format_results(&results, &TimeOutputFormat::Table);
println!("{}", output);
```

### IpLens

IP information lookup including ASN, prefix, RPKI validation, and geolocation.

```rust
use monocle::lens::ip::{IpLens, IpLookupArgs};
use std::net::IpAddr;

let lens = IpLens::new();

// Look up a specific IP
let args = IpLookupArgs::new("1.1.1.1".parse().unwrap());
let info = lens.lookup(&args)?;

println!("IP: {}", info.ip);
if let Some(asn) = &info.asn {
    println!("ASN: {}", asn.asn);
}
```

### Pfx2asLens

Prefix-to-ASN mapping using a trie-based data structure.

```rust
use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asLookupArgs, Pfx2asLookupMode};

// Load the lens (downloads data from BGPKIT)
let lens = Pfx2asLens::new(None)?;

// Look up a prefix with longest prefix match
let args = Pfx2asLookupArgs::new("1.1.1.0/24").longest();
let asns = lens.lookup(&args)?;

println!("Origin ASNs: {:?}", asns);
```

### RpkiLens

RPKI validation and data access via bgpkit-commons and Cloudflare's API.

```rust
use monocle::lens::rpki::{RpkiLens, RpkiValidationArgs, RpkiRoaLookupArgs};

let mut lens = RpkiLens::new();

// Validate a prefix/ASN pair
let args = RpkiValidationArgs::new(13335, "1.1.1.0/24");
let (validity, covering_roas) = lens.validate(&args)?;

// Get ROAs for a prefix
let args = RpkiRoaLookupArgs::new().with_prefix("1.1.1.0/24");
let roas = lens.get_roas(&args)?;
```

### As2orgLens

AS-to-Organization lookup using CAIDA's AS2Org data via bgpkit-commons.

```rust
use monocle::database::MonocleDatabase;
use monocle::lens::as2org::{As2orgLens, As2orgSearchArgs, As2orgOutputFormat};

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = As2orgLens::new(&db);

// Bootstrap if needed
if lens.needs_bootstrap() {
    lens.bootstrap()?;
}

// Search
let args = As2orgSearchArgs::new("cloudflare").name_only();
let results = lens.search(&args)?;

// Format output
let output = lens.format_results(&results, &As2orgOutputFormat::Json, false);
println!("{}", output);
```

### As2relLens

AS-level relationship data from BGPKIT's AS2Rel dataset.

```rust
use monocle::database::MonocleDatabase;
use monocle::lens::as2rel::{As2relLens, As2relSearchArgs};

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = As2relLens::new(&db);

// Update data if needed
if lens.needs_update() {
    lens.update()?;
}

// Search for relationships of a single ASN
let args = As2relSearchArgs::new(13335);
let results = lens.search(&args)?;

// Or search for relationship between two ASNs
let args = As2relSearchArgs::pair(13335, 174);
let results = lens.search(&args)?;
```

### ParseLens and SearchLens

MRT file parsing and BGP message search.

```rust
use monocle::lens::parse::{ParseLens, ParseFilters};
use monocle::lens::search::{SearchLens, SearchFilters, SearchDumpType};

// Parse a single MRT file
let parse_lens = ParseLens::new();
let filters = ParseFilters {
    origin_asn: Some(13335),
    prefix: Some("1.1.1.0/24".to_string()),
    ..Default::default()
};
let parser = parse_lens.create_parser(&filters, "path/to/file.mrt")?;

// Search across multiple MRT files
let search_lens = SearchLens::new();
let filters = SearchFilters {
    parse_filters: ParseFilters {
        start_ts: Some("2023-01-01T00:00:00Z".to_string()),
        end_ts: Some("2023-01-01T01:00:00Z".to_string()),
        ..Default::default()
    },
    collector: Some("rrc00".to_string()),
    dump_type: SearchDumpType::Updates,
    ..Default::default()
};
let broker_items = search_lens.query_broker(&filters)?;
```

### CountryLens

In-memory country code/name lookup using bgpkit-commons.

```rust
use monocle::lens::country::CountryLens;

let lens = CountryLens::new();

// Lookup by code
let name = lens.lookup_code("US");  // Some("United States")

// Search by name (partial match)
let countries = lens.lookup("united");  // Multiple results

// Get all countries
let all = lens.all();
```

## Naming Conventions

All types are prefixed with their module name to avoid ambiguity:

- `As2orgLens`, `As2orgSearchArgs`, `As2orgOutputFormat`, `As2orgSearchResult`
- `As2relLens`, `As2relSearchArgs`, `As2relOutputFormat`, `As2relSearchResult`
- `RpkiLens`, `RpkiValidationArgs`, `RpkiRoaLookupArgs`, `RpkiRoaEntry`
- `TimeLens`, `TimeParseArgs`, `TimeOutputFormat`, `TimeBgpTime`
- `IpLens`, `IpLookupArgs`, `IpOutputFormat`, `IpInfo`
- `Pfx2asLens`, `Pfx2asLookupArgs`, `Pfx2asOutputFormat`
- `ParseLens`, `ParseFilters`, `ParseElemType`
- `SearchLens`, `SearchFilters`, `SearchDumpType`

## Error Handling

Lenses use `anyhow::Result` for flexible error handling:

```rust
let results = lens.parse(&args)?;  // Propagates errors

// Or handle explicitly
match lens.parse(&args) {
    Ok(results) => { /* use results */ }
    Err(e) => eprintln!("Parse failed: {}", e),
}
```

## Adding a New Lens

To add a new lens (e.g., `NewLens`):

1. **Create directory structure:**
   ```
   lens/
   └── newlens/
       └── mod.rs      # Lens implementation with types and args
   ```

2. **Define types and argument structs** (in `mod.rs`):
   ```rust
   // Types
   #[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
   pub struct NewLensResult { ... }

   // Args
   #[derive(Debug, Clone, Default, Serialize, Deserialize)]
   #[cfg_attr(feature = "cli", derive(clap::Args))]
   pub struct NewLensArgs {
       pub input: String,
   }

   // Lens
   pub struct NewLens;

   impl NewLens {
       pub fn new() -> Self { Self }
       pub fn process(&self, args: &NewLensArgs) -> Result<NewLensResult> { ... }
       pub fn format_result(&self, result: &NewLensResult, format: &NewLensOutputFormat) -> String { ... }
   }
   ```

3. **Export from `lens/mod.rs`:**
   ```rust
   pub mod newlens;
   ```

4. **Update CLI command** (if applicable) to use the lens

## Feature Flags

The `cli` feature affects lenses:

- **Without `cli`**: Args structs are plain structs with serde derives
- **With `cli`**: Args structs also derive `clap::Args` for CLI parsing

Both modes support:
- Programmatic construction
- JSON serialization/deserialization
- Builder pattern methods

## Testing

Run lens tests:
```bash
cargo test lens::
```

Example test:
```rust
#[test]
fn test_time_lens() {
    let lens = TimeLens::new();
    let args = TimeParseArgs::new(vec!["1697043600".to_string()]);
    let results = lens.parse(&args).unwrap();
    
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].unix, 1697043600);
}
```

## Related Documentation

- [Database Module](../database/README.md) - Data access layer
- [ARCHITECTURE.md](../../ARCHITECTURE.md) - Overall system design