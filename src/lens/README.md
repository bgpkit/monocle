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
- [WEB_API_DESIGN.md](../../WEB_API_DESIGN.md) - Web API design for REST and WebSocket endpoints
- [DEVELOPMENT.md](../../DEVELOPMENT.md) - Contribution guidelines for adding lenses and web endpoints


| customer_asn | providers                                                    |
|--------------|--------------------------------------------------------------|
| 553          | 174, 559, 680, 1299, 2914, 3320                              |
| 559          | 174, 513, 553, 1299, 3257, 3356, 20965, 21320                |
| 1200         | 1103, 4455, 6830, 12859                                      |
| 1299         | 0                                                            |
| 2121         | 3333                                                         |
| 2605         | 2852, 6762                                                   |
| 3170         | 174, 2914, 3257, 3356, 3491                                  |
| 4492         | 0                                                            |
| 4601         | 8298, 58115                                                  |
| 6424         | 174, 1273, 1299, 2914, 6461, 6762, 6830, 141193              |
| 6775         | 174, 6204, 6939, 13030                                       |
| 6777         | 0                                                            |
| 6823         | 174, 1200, 3223, 6204, 6205, 6695, 9002, 9121, 12735, 15924, |
|              |  20715, 34565, 44901, 56393, 204457, 211877                  |
| 8265         | 6453, 51569                                                  |
| 8283         | 8455, 15703, 38930, 57866, 58115                             |
| 8873         | 8674                                                         |
| 9120         | 1299, 31027                                                  |
| 11358        | 835, 6939, 34927                                             |
| 11967        | 835, 1299, 6939, 34872, 34927, 50917, 58057, 214809, 215828  |
| 12637        | 174, 1299, 2914, 3303, 50673                                 |
| 12779        | 174, 2914, 3257                                              |
| 13020        | 3320, 15943, 50472, 50629                                    |
| 15605        | 3356, 6762, 12637, 41327, 56911                              |
| 15974        | 9049, 20485                                                  |
| 16909        | 6939, 20473, 41051, 52025, 53667, 214481, 401507             |
| 18607        | 20473, 207487                                                |
| 23506        | 23507, 30433, 215828                                         |
| 23507        | 1002, 20473, 204044, 213669, 213683                          |
| 28370        | 53013, 271253                                                |
| 30581        | 47778, 397958                                                |
| 31319        | 174, 1299                                                    |
| 31454        | 8708, 9050                                                   |
| 33891        | 1299, 3257, 3320, 3356                                       |
| 34425        | 6424                                                         |
| 34878        | 553, 680                                                     |
| 35168        | 3216, 6695, 8631, 9002, 20485, 31133, 35104, 43727, 60299    |
| 35489        | 15669, 35761, 39505, 44814, 199230                           |
| 38927        | 9063, 15965                                                  |
| 39002        | 44854, 48112, 57984                                          |
| 39074        | 49100, 50810                                                 |
| 40544        | 924, 6939, 12186, 52025                                      |
| 40929        | 917, 14447, 17408, 20473, 23961, 44324, 47272, 53808, 57695, |
|              |  203314, 207529, 214439                                      |
| 40994        | 174, 1299, 2914, 3257, 6762                                  |
| 41327        | 1299, 3356, 3491, 6762                                       |
| 41720        | 174, 1299, 9002, 50877, 60068, 203446, 212508                |
| 42184        | 174, 1299, 3257, 3320, 33891, 55002                          |
| 42541        | 1299, 3356, 6939                                             |
| 43046        | 6830, 8708, 12302                                            |
| 43615        | 2588, 12578                                                  |
| 43617        | 29551, 58145                                                 |
| 43633        | 6939, 9002                                                   |
| 43652        | 2860, 6424                                                   |
| 43851        | 3170                                                         |
| 44244        | 12880, 49666                                                 |
| 44324        | 3204, 6204, 6939, 7720, 8849, 33387, 34927, 38074, 47272,    |
|              | 48266, 53356, 53667, 53808, 59105, 134823, 134835, 138997,   |
|              | 206499, 207841, 393577                                       |
| 44355        | 6939, 49775, 61049, 207841, 215759                           |
| 44716        | 12732                                                        |
| 44980        | 8607, 15830, 21371, 29611                                    |
| 45009        | 8283, 25091, 50869                                           |
| 47184        | 0                                                            |
| 47272        | 174, 835, 924, 1299, 3257, 6830, 6939, 16276, 20473, 34927,  |
|              | 35133, 41051, 48070, 50917, 52025, 61049, 61138, 64289,      |
|              | 212514, 215638, 400587                                       |
| 47311        | 6939, 24198, 45735, 58495, 133210, 137409                    |
| 47455        | 213313                                                       |
| 47536        | 5405, 6762, 34549, 34927, 49600, 50629                       |
| 47597        | 16276                                                        |
| 47689        | 1299, 6939, 21738, 23033, 34465, 34549, 34927, 54148, 395823 |
| 47694        | 52091, 206810                                                |
| 47778        | 6939, 7720, 34927, 51087, 62246, 206499                      |
| 47787        | 174, 6424, 6762                                              |
| 47943        | 6424, 35432                                                  |
| 48070        | 174, 1299, 6461, 61049                                       |
| 48444        | 3170, 20712                                                  |
| 48606        | 30893, 54681, 57974, 59678, 210691, 212276                   |
| 49260        | 6424                                                         |
| 49623        | 60983                                                        |
| 49697        | 47536                                                        |
| 49832        | 12297, 49666                                                 |
| 49935        | 174, 702, 1273, 1299, 2602, 2611, 2906, 2914, 3257, 3303,    |
|              | 3356, 3491, 3856, 4455, 5432, 5511, 6453, 6461, 6641, 6661,  |
|              | 6696, 6762, 6774, 6848, 6939, 7018, 8075, 8218, 8220, 8368,  |
|              | 8632, 8674, 8708, 9002, 9008, 9009, 9031, 9208, 13002,       |
|              | 13150, 13335, 15965, 16276, 20940, 23764, 24611, 29467,      |
|              | 32934, 34019, 35219, 39686, 39923, 47377, 47957, 48152,      |
|              | 48185, 48408, 50083, 50309, 56665, 62044, 199095, 199524,    |
|              | 199670, 201205, 210834                                       |
| 50028        | 6424, 47787                                                  |
| 50224        | 6939, 37988, 52025, 59920, 60841, 137409, 142064, 396993,    |
|              | 400304                                                       |
| 50338        | 16019, 47232, 60068                                          |
| 50391        | 6939, 20473, 48070                                           |
| 50633        | 3326, 35297                                                  |
| 51019        | 6939, 34549, 34927, 52025, 207841, 209735, 215823            |
| 51191        | 20473, 33891, 60068, 199524                                  |
| 51345        | 6939, 8283, 8298                                             |
| 51396        | 6939, 30823, 35133, 44066, 44592, 49581, 60223, 64457,       |
|              | 203446, 214497, 214995, 215436                               |
| 51826        | 1299                                                         |
| 52025        | 174, 835, 1299, 3257, 6939, 8849, 9678, 12488, 20473, 25369, |
|              |  26073, 26548, 29802, 32097, 34549, 34927, 35133, 37988,     |
|              | 39409, 48070, 56655, 60068, 61049, 61138, 64289, 208453,     |
|              | 212815, 394177, 396998, 397423, 400587                       |
| 52210        | 174, 835, 6939, 52025, 62513, 210667                         |
| 52838        | 174, 2914, 3356, 6762                                        |
| 53343        | 3204, 6939, 7720, 21700, 26006, 32595, 44324, 47272, 47741,  |
|              | 59105, 204844, 207841, 209735, 393577, 394177, 400587        |
| 53808        | 26042, 44324, 48266, 51847, 204044, 214439                   |
| 54148        | 835, 924, 6939, 20473, 21738, 34927, 37988, 44355, 47272,    |
|              | 52025, 53616, 53667, 137409, 207841, 209022, 209735, 210475, |
|              |  400212, 400587                                              |
| 54218        | 917, 16509, 35487, 37988, 53667, 57196, 57695, 59678, 137409 |
| 54286        | 32595                                                        |
| 56762        | 47689                                                        |
| 56853        | 44030, 48416                                                 |
| 57399        | 47536                                                        |
| 57511        | 49832                                                        |
| 57777        | 8298, 25901                                                  |
| 57984        | 8283, 48112, 206763                                          |
| 58145        | 29551                                                        |
| 58165        | 12297                                                        |
| 58308        | 29075, 59689                                                 |
| 59645        | 250, 6939, 24961, 29670, 34927, 50629, 58299                 |
| 59678        | 6939, 34927, 54218, 55081                                    |
| 59689        | 29075, 30781                                                 |
| 59715        | 6762, 12874                                                  |
| 59920        | 13335, 20473, 30456, 52041, 137409                           |
| 60132        | 29467, 197133                                                |
| 60247        | 0                                                            |
| 60431        | 174, 6939, 34927, 41495, 50917, 56630, 137409, 209735        |
| 60841        | 13335, 30456, 52041, 400304                                  |
| 60900        | 6939, 13335, 14618, 20473, 34927, 37988, 41051, 44324,       |
|              | 50224, 53667, 60841, 137409, 142064, 205329, 208453, 209533, |
|              |  210667, 214757, 214809, 396993, 399486                      |
| 60983        | 56987, 201376, 203500, 209255                                |
| 61290        | 6424                                                         |
| 61421        | 6939, 8283, 45009                                            |
| 61574        | 61575                                                        |
| 61604        | 6939, 22356                                                  |
| 61618        | 2914, 26615, 263903                                          |
| 61625        | 52468, 53013, 53181, 60503, 265147, 265269, 268463           |
| 61774        | 264011, 264595                                               |
| 62028        | 34154, 64475                                                 |
| 62078        | 0                                                            |
| 62228        | 0                                                            |
| 64475        | 3320, 29551, 47147, 50629                                    |
| 138038       | 4785                                                         |
| 149301       | 20473, 61138                                                 |
| 151642       | 20473, 61138                                                 |
| 154185       | 6939, 154155, 198025                                         |
| 196610       | 51531                                                        |
| 197301       | 1299, 31027                                                  |
| 197556       | 35168                                                        |
| 197942       | 1257, 3301                                                   |
| 198025       | 20473, 34927, 44324, 53667, 53808, 151194                    |
| 198136       | 0                                                            |
| 198249       | 6939, 13030, 34549                                           |
| 198304       | 44324                                                        |
| 199036       | 0                                                            |
| 199310       | 6939, 7720, 8772, 17433, 29632, 32595, 33387, 34927, 39249,  |
|              | 44324, 53808, 134835, 138997, 139317, 202662, 204844,        |
|              | 212895, 214439, 215828                                       |
| 199376       | 150249, 151544, 199762, 212001                               |
| 199422       | 174, 39801, 57734                                            |
| 199530       | 49070                                                        |
| 199557       | 35197                                                        |
| 199762       | 8849, 141067, 150249, 203868, 212001                         |
| 199881       | 60983                                                        |
| 200160       | 945, 6939, 15353, 21738, 29632, 34465, 34549, 34927, 36369,  |
|              | 48605, 140731, 150249, 199762, 203686, 203868, 203913,       |
|              | 210152, 210475, 212001, 400304, 400818                       |
| 200240       | 42831, 59796                                                 |
| 200242       | 835, 917, 6939, 21738, 34465, 34927, 52025, 52041, 59678,    |
|              | 60841, 142064, 208453, 210475                                |
| 200306       | 6939, 20473, 41051, 58057, 209533, 212895                    |
| 200351       | 6939, 54148                                                  |
| 200455       | 3204, 6939, 41051, 142064, 212271, 400304                    |
| 200462       | 174, 1299, 2914, 3257, 5405, 5511, 6453, 6762, 6830, 9002,   |
|              | 34549                                                        |
| 200712       | 1257, 1299                                                   |
| 200852       | 25019, 39386, 57187                                          |
| 200886       | 3326, 35297                                                  |
| 200995       | 12859                                                        |
| 201011       | 33891                                                        |
| 201281       | 34019, 204092, 208627                                        |
| 201376       | 0                                                            |
| 202010       | 0                                                            |
| 202032       | 174, 3356, 5398, 6730, 6939, 8220, 9002                      |
| 202359       | 44355                                                        |
| 202361       | 197301                                                       |
| 202585       | 37739, 41051, 42093, 57866, 200132, 212895, 215085           |
| 202881       | 44324, 205329                                                |
| 202939       | 6939, 7720, 14447, 31898, 34927, 41051, 44324, 202662        |
| 202945       | 30781, 34019, 204092, 208627                                 |
| 203019       | 212895                                                       |
| 203031       | 0                                                            |
| 203135       | 43366, 209390                                                |
| 203662       | 63473, 152900, 215467                                        |
| 203843       | 6939, 7720, 17433, 32595                                     |
| 203868       | 6939, 8772, 34927, 152368, 209533, 210475, 212895, 214481    |
| 203921       | 212238                                                       |
| 204092       | 30781, 34019                                                 |
| 204104       | 31549, 41881, 42337, 43754, 62403, 205647, 211904            |
| 204211       | 174, 6939, 44324, 53667, 53808, 151673, 207529, 210352,      |
|              | 210773                                                       |
| 204318       | 42615, 206924, 212232                                        |
| 204471       | 9002, 41327, 50877, 62275                                    |
| 204518       | 198304                                                       |
| 204653       | 44486                                                        |
| 204680       | 553                                                          |
| 204857       | 50224, 59920, 60841, 400304                                  |
| 204931       | 34927, 44103, 60431                                          |
| 205079       | 8283, 34872, 212895                                          |
| 205154       | 34927                                                        |
| 205235       | 42541                                                        |
| 205329       | 983, 6939, 7720, 8894, 20473, 27523, 34927, 41051, 131657,   |
|              | 134823, 134835, 209735                                       |
| 205603       | 38008, 38074, 59105, 63798                                   |
| 205619       | 16019, 28725                                                 |
| 205789       | 200242, 207960                                               |
| 205848       | 1299, 6939, 34549, 34927, 35133, 37988, 47272, 52025, 61138, |
|              |  208453, 209022                                              |
| 205929       | 1299, 6830, 6939, 34927, 58299                               |
| 205941       | 57050, 60900, 210464, 212895, 213449, 214639, 214757,        |
|              | 214809, 215828                                               |
| 206155       | 3215, 6939, 57199, 204092, 209533                            |
| 206236       | 9136                                                         |
| 206271       | 60707                                                        |
| 206345       | 12880, 24631                                                 |
| 206392       | 20712, 25160                                                 |
| 206557       | 2116, 6667                                                   |
| 206604       | 6939, 11967, 34872, 34927, 60326, 205941, 207567, 211358,    |
|              | 213449, 214757, 214809, 215828                               |
| 206924       | 1299, 3170, 5511, 6939, 44684, 44854, 137409                 |
| 207080       | 1299, 3170, 6939, 8943, 13335, 42615                         |
| 207113       | 6939, 37739, 328578, 328858, 328977, 329101, 329183, 329535, |
|              |  329539, 329552                                              |
| 207487       | 8283, 200132                                                 |
| 207510       | 29670, 58299, 207160                                         |
| 207529       | 6939, 7720, 17408, 17433, 34927, 41051, 44324, 51087, 53808, |
|              |  139317, 140915, 199310, 210176, 212895, 213605, 214439      |
| 207616       | 835, 49581, 53667, 136510, 214668                            |
| 207682       | 215085                                                       |
| 207727       | 42541, 205235                                                |
| 207833       | 49832                                                        |
| 207841       | 44355, 61049, 137409                                         |
| 207960       | 6939, 20473, 34854, 34927, 44355, 136620, 202359, 207968     |
| 207995       | 3257, 3356, 8676                                             |
| 208015       | 3320, 8220, 8881, 34549, 208016                              |
| 208016       | 3320, 8220, 8881, 34549                                      |
| 208018       | 41327                                                        |
| 208059       | 20473, 34872, 34927, 35661, 41047, 41051, 53667, 56381,      |
|              | 206499, 212895                                               |
| 208105       | 12297                                                        |
| 208453       | 3399, 35133, 60068                                           |
| 208460       | 20473, 209533                                                |
| 208492       | 206345                                                       |
| 208505       | 6939, 20473, 34927                                           |
| 208563       | 6939, 34549, 34854, 34927, 41051, 58299, 207960              |
| 208627       | 29075, 34019, 44097, 204092, 206165                          |
| 208702       | 6939, 34927                                                  |
| 208824       | 6939, 8772, 29632, 34927, 41051, 44324, 52025, 207841,       |
|              | 209533, 209735, 211358, 212895, 214757, 215828, 393577       |
| 208893       | 1299                                                         |
| 208974       | 204680                                                       |
| 209327       | 44869                                                        |
| 209442       | 6821, 50973                                                  |
| 209559       | 44097, 204092, 206165, 208627                                |
| 209718       | 34927, 209022                                                |
| 209735       | 6939, 44355, 207841                                          |
| 209807       | 6939, 34927, 58299                                           |
| 209870       | 43861                                                        |
| 210099       | 42216                                                        |
| 210118       | 60132                                                        |
| 210312       | 4601                                                         |
| 210440       | 6939, 20473, 53808, 213605                                   |
| 210464       | 6517, 23507, 59796, 212895, 213669, 213683, 214809           |
| 210561       | 207960                                                       |
| 210632       | 11967                                                        |
| 210715       | 209533                                                       |
| 210732       | 6939, 209768                                                 |
| 210796       | 215147                                                       |
| 210812       | 31898, 44355, 47272, 209735                                  |
| 210872       | 6661, 6939, 49624                                            |
| 211024       | 6939, 8283, 20473, 34927                                     |
| 211286       | 1273, 3320, 5405, 9002, 25291, 47147, 50629, 58299, 59645    |
| 211453       | 1257, 12552                                                  |
| 211575       | 6939, 7720, 20473, 31898, 44324, 53808, 210352, 212895,      |
|              | 214439                                                       |
| 211633       | 1299, 3356, 6939                                             |
| 212001       | 6939, 45735, 47311, 152368, 152394                           |
| 212068       | 6939, 13237, 62240, 207960                                   |
| 212223       | 9150, 39686                                                  |
| 212245       | 917, 57695, 212068                                           |
| 212271       | 174, 39120, 49245, 60501                                     |
| 212557       | 0                                                            |
| 212635       | 8283, 34927, 50869                                           |
| 212674       | 0                                                            |
| 212855       | 8283, 8298, 41051, 50869, 200132, 212895                     |
| 212934       | 835, 6939, 34927, 52025, 52210, 53667, 61138, 62513, 209533, |
|              |  210667                                                      |
| 213052       | 8283, 31477, 200132                                          |
| 213054       | 1299, 3356, 6939                                             |
| 213068       | 553, 9136, 34878                                             |
| 213279       | 6939, 20473, 34927, 212027                                   |
| 213313       | 35280                                                        |
| 213422       | 34872, 41051, 209533, 212895, 214915                         |
| 213423       | 34927, 47526, 212895, 214915                                 |
| 213449       | 6939, 11967, 34872, 34927, 41051, 52025, 58087, 203446,      |
|              | 206604                                                       |
| 213451       | 215638                                                       |
| 213525       | 8560, 44355, 207841                                          |
| 213605       | 6939, 7720, 17433, 34927, 44324, 47272, 51087, 53667,        |
|              | 134823, 199310, 209533                                       |
| 213768       | 6939, 34927, 52025                                           |
| 213967       | 49245, 212271                                                |
| 214205       | 34927, 52025, 205941, 212895                                 |
| 214380       | 1101, 1103, 6939, 8283, 44854, 50869, 200132, 212635, 212895 |
| 214495       | 53343                                                        |
| 214498       | 16276, 44355, 207841                                         |
| 214675       | 6939, 9663, 11967, 34872, 34927, 38074, 41051, 63798,        |
|              | 150369, 214757, 215828, 216324                               |
| 214701       | 20473, 39351                                                 |
| 214720       | 44324, 53808                                                 |
| 214749       | 34927, 47272, 52025, 207841, 209533, 212895, 214915          |
| 214757       | 6939, 34872, 34927, 49127, 50338, 50917, 60068, 62553,       |
|              | 208453, 214809                                               |
| 214768       | 6939, 52025, 207841, 209735                                  |
| 214805       | 29049, 49666, 51889                                          |
| 214809       | 174, 6204, 6939, 35133, 50917, 64289                         |
| 214857       | 29049, 49666, 51889                                          |
| 214903       | 35489, 39505                                                 |
| 214955       | 20473, 44324, 53667, 207529, 213605                          |
| 214958       | 8283                                                         |
| 214976       | 23507                                                        |
| 215063       | 1257                                                         |
| 215085       | 35133, 42093, 211588                                         |
| 215131       | 8283, 34927                                                  |
| 215134       | 34927, 207841, 209735                                        |
| 215147       | 34927, 47263, 205941, 207252, 210796, 213413, 215828         |
| 215163       | 200462                                                       |
| 215236       | 29670, 59645                                                 |
| 215248       | 924, 5405, 6204, 8283, 25091, 34872, 34927, 37739, 41051,    |
|              | 42093, 47272, 50263, 50917, 58057, 61049, 64289, 200132,     |
|              | 208210, 215664                                               |
| 215250       | 59645                                                        |
| 215296       | 6204, 8283, 64289, 200132, 212895                            |
| 215318       | 50917                                                        |
| 215368       | 207841                                                       |
| 215375       | 214138, 215828                                               |
| 215436       | 6762, 6939, 51396, 60223                                     |
| 215467       | 3399, 5511, 6939, 37988, 59711, 142064, 208453, 214354,      |
|              | 263702, 269070                                               |
| 215638       | 1299, 6939, 9002, 13213                                      |
| 215666       | 207252                                                       |
| 215782       | 1299, 3356                                                   |
| 215828       | 1299, 6204, 6939, 34927, 51202, 59796, 62255, 64289, 207841, |
|              |  209735, 211358, 393577                                      |
| 215849       | 215375, 215828                                               |
| 216107       | 6939, 16509, 20473, 21738, 31898, 37988, 211588              |
| 216164       | 212271                                                       |
| 216426       | 206276                                                       |
| 262486       | 4230, 20121, 26162, 53237, 263450                            |
| 263047       | 3356, 7738, 22356                                            |
| 263326       | 22381, 28260, 52863, 265389                                  |
| 263390       | 2914, 52772, 263009                                          |
| 263424       | 3356, 28202, 28306, 61712, 263432                            |
| 263515       | 264011, 264539, 264595                                       |
| 264011       | 16735, 264595                                                |
| 264539       | 264011, 264595                                               |
| 264595       | 14840, 28598, 53087, 61832, 264011                           |
| 264967       | 264011, 264595                                               |
| 265010       | 262462                                                       |
| 266086       | 264011, 264595                                               |
| 266106       | 264011, 264595                                               |
| 266342       | 264011, 264595                                               |
| 267386       | 6939, 25933, 28220, 28649, 262509, 267613                    |
| 267608       | 28370                                                        |
| 268047       | 263998, 268829, 272713                                       |
| 268105       | 52898, 263047, 272691                                        |
| 268151       | 265272, 274610, 274772                                       |
| 269114       | 263998, 268047                                               |
| 269288       | 52873, 61618                                                 |
| 269396       | 262999, 264963                                               |
| 269688       | 3356, 23106, 263623                                          |
| 270470       | 6939, 53062, 263495, 265189                                  |
| 270573       | 28283, 53062                                                 |
| 270684       | 6939, 52468, 263627, 267613                                  |
| 271150       | 269572                                                       |
| 271228       | 53087, 262773                                                |
| 271479       | 268502, 268525, 270907                                       |
| 272466       | 269396                                                       |
| 272564       | 174                                                          |
| 272570       | 263424                                                       |
| 272671       | 263324                                                       |
| 273514       | 28370, 263535                                                |
| 273556       | 53181, 263009                                                |
| 274610       | 28260, 269553                                                |
| 274786       | 53062, 263573                                                |
| 393577       | 174, 1299, 6939, 32097, 137409                               |
| 396064       | 924, 5405, 6204, 8283, 25091, 34872, 34927, 37739, 41051,    |
|              | 42093, 47272, 50263, 50917, 58057, 61049, 64289, 200132,     |
|              | 208210, 215664                                               |
| 396968       | 16909, 401507                                                |
| 397658       | 11967, 34927                                                 |
| 397730       | 835, 6939, 37988, 400212                                     |
| 397958       | 20473, 30581, 47778, 401519                                  |
| 401038       | 52210                                                        |
| 401507       | 16909, 212477                                                |
| 401519       | 30581, 47778, 397958                                         |
