# Monocle Architecture

This document describes the architecture of the monocle project, a BGP information toolkit that can be used as both a command-line application and a library.

## Overview

Monocle is designed with the following principles:

1. **Library-First Design**: Core functionality is implemented as a library that can be reused across different interfaces (CLI, web API, GUI)
2. **Separation of Concerns**: Clear boundaries between data access, business logic, and presentation
3. **Extensibility**: Easy to add new data types and services
4. **Single Source of Truth**: Shared database with unified schema management

## Directory Structure

```
src/
├── lib.rs                    # Public API exports
├── config.rs                 # Configuration management (MonocleConfig)
├── time.rs                   # Time utilities
│
├── database/                 # All database functionality
│   ├── mod.rs                # Module exports
│   ├── README.md             # Database module documentation
│   │
│   ├── core/                 # Core database infrastructure
│   │   ├── mod.rs
│   ├── connection.rs     # DatabaseConn connection wrapper
│   │   └── schema.rs         # Schema definitions and management
│   │
│   ├── session/              # One-time/session databases
│   │   ├── mod.rs
│   │   └── msg_store.rs      # BGP search results storage
│   │
│   └── monocle/              # Main monocle database
│       ├── mod.rs            # MonocleDatabase entry point
│       ├── as2org.rs         # AS2Org repository
│       └── as2rel.rs         # AS2Rel repository
│
├── services/                 # Business logic layer
│   ├── mod.rs                # Service exports
│   ├── README.md             # Services module documentation
│   │
│   ├── as2org/               # AS-to-Organization service
│   │   ├── mod.rs            # Service implementation
│   │   ├── types.rs          # Result types (SearchResult, etc.)
│   │   └── args.rs           # Reusable argument structs
│   │
│   ├── as2rel/               # AS-level relationship service
│   │   ├── mod.rs
│   │   ├── types.rs
│   │   └── args.rs
│   │
│   └── country.rs            # Country lookup (in-memory)
│
├── filters/                  # BGP message filters
│   ├── mod.rs
│   ├── parse.rs              # MRT file parsing filters
│   └── search.rs             # BGP message search filters
│
├── utils/                    # Utility functions
│   ├── mod.rs                # Utility exports
│   ├── ip.rs                 # IP information lookup
│   ├── pfx2as.rs             # Prefix-to-ASN mapping
│   └── rpki/                 # RPKI utilities
│       ├── mod.rs
│       ├── commons.rs        # bgpkit-commons RPKI data
│       └── validator.rs      # Cloudflare RPKI GraphQL API
│
└── bin/
    ├── monocle.rs            # CLI entry point
    └── commands/             # CLI command handlers
```

## Implementation Status

| Component | Status | Notes |
|-----------|--------|-------|
| `database/core` | ✅ Complete | Connection, schema management |
| `database/session` | ✅ Complete | MsgStore for search results |
| `database/monocle` | ✅ Complete | MonocleDatabase, As2org/As2rel repos |
| `services/as2org` | ✅ Complete | Service, args, types |
| `services/as2rel` | ✅ Complete | Service, args, types |
| `services/country` | ✅ Complete | In-memory lookup using bgpkit-commons |
| `filters` | ✅ Complete | Feature-gated clap derives |
| `utils` | ✅ Complete | IP lookup, Pfx2AS, RPKI utilities |
| Feature flags | ✅ Complete | Phase 3 - cli, server, full features |
| Web server | 🔲 Pending | Phase 4 |

### Phase Completion Status

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 1 | ✅ Complete | Database module (`database/`) |
| Phase 2 | ✅ Complete | Services module (`services/`) |
| Phase 3 | ✅ Complete | Feature flags (cli, server, full) |
| Phase 4 | 🔲 Pending | Web server implementation |
| Phase 5 | 🔄 In Progress | CLI migration & cleanup |

## Module Architecture

### Database Module (`database/`)

The database module provides all data persistence functionality, organized into three sub-modules:

#### Core (`database/core/`)

Foundation components used by all database operations:

```
┌─────────────────────────────────────────────────────────────┐
│                    DatabaseConn                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  - SQLite connection wrapper                        │   │
│  │  - Configuration (WAL mode, cache, etc.)            │   │
│  │  - Transaction management                           │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│                    SchemaManager                            │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  - Schema definitions for all tables                │   │
│  │  - Version tracking                                 │   │
│  │  - Initialization and reset                         │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Key Types:**
- `DatabaseConn`: Core connection wrapper with SQLite optimizations
- `SchemaManager`: Handles schema initialization and version checking
- `SchemaStatus`: Enum representing database schema state

#### Session (`database/session/`)

Storage for one-time/ephemeral data:

```
┌─────────────────────────────────────────────────────────────┐
│                      MsgStore                               │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  - Per-search SQLite database                       │   │
│  │  - BGP element storage (elems table)                │   │
│  │  - Batch insert with transactions                   │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Use Case:** Storing BGP search results during a search operation

#### Monocle (`database/monocle/`)

Main persistent database for monocle:

```
┌─────────────────────────────────────────────────────────────┐
│                   MonocleDatabase                           │
│  ┌─────────────────────────────────────────────────────┐   │
│  │  Single entry point to monocle data                 │   │
│  │  - Schema initialization on open                    │   │
│  │  - Automatic drift detection and reset              │   │
│  │  - Repository access methods                        │   │
│  └─────────────────────────────────────────────────────┘   │
│                           │                                 │
│         ┌─────────────────┼─────────────────┐              │
│         ▼                 ▼                 ▼              │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │ As2orgRepo  │  │ As2relRepo  │  │  (Future)   │        │
│  │             │  │             │  │             │        │
│  │ - as2org_as │  │ - as2rel    │  │ - rpki_roas │        │
│  │ - as2org_org│  │ - as2rel_   │  │ - etc.      │        │
│  │ - views     │  │   meta      │  │             │        │
│  └─────────────┘  └─────────────┘  └─────────────┘        │
└─────────────────────────────────────────────────────────────┘
```

**Database File:** `~/.monocle/monocle-data.sqlite3`

**Tables:**
- `monocle_meta`: Schema version and global metadata
- `as2org_as`: AS to organization mappings
- `as2org_org`: Organization details
- `as2rel`: AS-level relationships
- `as2rel_meta`: AS2Rel data metadata

### Services Module (`services/`)

Business logic layer that combines database access with domain operations:

```
┌─────────────────────────────────────────────────────────────┐
│                       Service Layer                         │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                  As2orgService                       │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌────────────┐   │   │
│  │  │    args     │  │   types     │  │  service   │   │   │
│  │  │ SearchArgs  │  │SearchResult │  │  search()  │   │   │
│  │  │ UpdateArgs  │  │ SearchType  │  │  format()  │   │   │
│  │  │ OutputArgs  │  │OutputFormat │  │  update()  │   │   │
│  │  └─────────────┘  └─────────────┘  └────────────┘   │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │                  As2relService                       │   │
│  │  ┌─────────────┐  ┌─────────────┐  ┌────────────┐   │   │
│  │  │    args     │  │   types     │  │  service   │   │   │
│  │  │ SearchArgs  │  │SearchResult │  │  search()  │   │   │
│  │  │ UpdateArgs  │  │ SortOrder   │  │  format()  │   │   │
│  │  │ OutputArgs  │  │OutputFormat │  │  update()  │   │   │
│  │  └─────────────┘  └─────────────┘  └────────────┘   │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              CountryLookup (in-memory)              │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Key Design Patterns:**

1. **Reusable Arguments**: Argument structs are serializable and can be used across CLI, REST API, WebSocket, and GUI interfaces.

2. **Repository Pattern**: Services use repositories from the database module for data access.

3. **Output Formatting**: Services handle result formatting (JSON, table, PSV) internally.

## Data Flow

### Library Usage Flow

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Client     │────▶│   Service    │────▶│  Repository  │
│   Code       │     │  (As2org)    │     │  (As2orgRepo)│
└──────────────┘     └──────────────┘     └──────────────┘
                            │                     │
                            │                     ▼
                     ┌──────▼──────┐     ┌───────────────┐
                     │   Format    │     │MonocleDatabase│
                     │   Results   │     └───────────────┘
                     └─────────────┘
```

### CLI Usage Flow

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   CLI Args   │────▶│   Command    │────▶│   Service    │
│   (clap)     │     │   Handler    │     │              │
└──────────────┘     └──────────────┘     └──────────────┘
                                                │
                                         ┌──────▼──────┐
                                         │   Output    │
                                         │   (stdout)  │
                                         └─────────────┘
```

## Schema Management

### Version Tracking

The database schema version is tracked in the `monocle_meta` table:

```sql
CREATE TABLE monocle_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
```

### Schema Status

```rust
pub enum SchemaStatus {
    NotInitialized,  // Fresh database
    Current,         // Schema matches expected version
    NeedsMigration { from: u32, to: u32 },
    Incompatible { database_version: u32, required_version: u32 },
    Corrupted,       // Missing tables
}
```

### Automatic Recovery

When opening a `MonocleDatabase`:
1. Check schema status
2. If incompatible or corrupted, reset and reinitialize
3. Data is repopulated from external sources on next use

## Migration Notes

### Using the New Architecture

For new code, use the services module:

```rust
use monocle::MonocleDatabase;
use monocle::services::{As2orgService, As2orgSearchArgs};

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let service = As2orgService::new(&db);
let results = service.search(&As2orgSearchArgs::new("cloudflare"))?;
```

### Utility Functions

For standalone utilities (IP lookup, RPKI, Pfx2AS), use the `utils` module:

```rust
use monocle::{fetch_ip_info, validate, Pfx2as};

// IP information lookup
let info = fetch_ip_info(None, false)?;

// RPKI validation
let (validity, roas) = validate(13335, "1.1.1.0/24")?;

// Prefix-to-AS mapping
let pfx2as = Pfx2as::new(None)?;
let origins = pfx2as.lookup_longest("1.1.1.0/24".parse()?);
```

## Feature Flags (Implemented)

Monocle supports conditional compilation via Cargo features, enabling minimal library builds
without CLI dependencies.

### Available Features

```toml
[features]
default = ["cli"]

# CLI support (clap derives, terminal output, progress bars)
cli = [
    "dep:clap",
    "dep:indicatif",
    "dep:json_to_table",
    "dep:tracing-subscriber",
]

# Web server support (placeholder for Phase 4)
server = []

# Full build with all features
full = ["cli", "server"]
```

### Feature-Gated Code

Types that conditionally derive clap traits:
- `ParseFilters` - MRT file parsing filters
- `SearchFilters` - BGP message search filters
- `ElemTypeEnum` - BGP element type (announce/withdraw)
- `DumpType` - MRT dump type (updates/rib)
- `As2orgSearchArgs`, `As2relSearchArgs` - Service argument structs

Example pattern used:
```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct ParseFilters {
    #[cfg_attr(feature = "cli", clap(short = 'o', long))]
    pub origin_asn: Option<u32>,
    // ...
}
```

### Usage Examples

```toml
# Minimal library (no CLI dependencies, smaller binary)
monocle = { version = "0.9", default-features = false }

# Library with CLI argument structs
monocle = { version = "0.9", features = ["cli"] }

# Full build (default)
monocle = { version = "0.9" }
```

## Future Architecture (Planned)

### Web Server Module

```
server/
├── mod.rs           # Server entry point
├── websocket.rs     # WebSocket handler
├── protocol.rs      # Message types
└── handlers/
    ├── as2org.rs
    ├── as2rel.rs
    └── search.rs
```

### GUI Integration (GPUI)

Separate crate `monocle-gui` using:
- `gpui` framework
- `gpui-component` for UI components
- Shared services from monocle library

## Key Types Reference

### Database Types

| Type | Location | Purpose |
|------|----------|---------|
| `DatabaseConn` | `database/core/connection.rs` | SQLite connection wrapper |
| `SchemaManager` | `database/core/schema.rs` | Schema management |
| `MonocleDatabase` | `database/monocle/mod.rs` | Main database interface |
| `MsgStore` | `database/session/msg_store.rs` | BGP message storage |
| `As2orgRepository` | `database/monocle/as2org.rs` | AS2Org data access |
| `As2relRepository` | `database/monocle/as2rel.rs` | AS2Rel data access |

### Service Types

| Type | Location | Purpose |
|------|----------|---------|
| `As2orgService` | `services/as2org/mod.rs` | AS2Org operations |
| `As2orgSearchArgs` | `services/as2org/args.rs` | Search parameters |
| `As2orgSearchResult` | `services/as2org/types.rs` | Search results |
| `As2relService` | `services/as2rel/mod.rs` | AS2Rel operations |
| `As2relSearchArgs` | `services/as2rel/args.rs` | Search parameters |
| `CountryLookup` | `services/country.rs` | Country code/name lookup |

## Usage Examples

### As a Library

```rust
use monocle::MonocleDatabase;
use monocle::services::{As2orgService, As2orgSearchArgs, As2orgOutputFormat};

// Open database
let db = MonocleDatabase::open_in_dir("~/.monocle")?;

// Create service
let service = As2orgService::new(&db);

// Bootstrap if needed
if service.needs_bootstrap() {
    service.bootstrap()?;
}

// Search
let args = As2orgSearchArgs::new("cloudflare");
let results = service.search(&args)?;

// Format output
let output = service.format_results(&results, &As2orgOutputFormat::Json, false);
println!("{}", output);
```

### Cross-Table Queries

```rust
// Get the underlying connection for custom queries
let conn = db.connection();

// Execute a JOIN query
let mut stmt = conn.prepare("
    SELECT r.asn1, o1.org_name, r.asn2, o2.org_name
    FROM as2rel r
    JOIN as2org_all o1 ON r.asn1 = o1.asn
    JOIN as2org_all o2 ON r.asn2 = o2.asn
    WHERE r.asn1 = ?1
")?;
```

## Contributing

When adding new features:

1. **New Data Type**: Add repository in `database/shared/`, service in `services/`
2. **New Service**: Follow the pattern of `as2org/` with separate `args.rs`, `types.rs`, and `mod.rs`
3. **Schema Changes**: Update `database/core/schema.rs` and increment `SCHEMA_VERSION`

## Related Documents

- [REVISION_PLAN.md](REVISION_PLAN.md) - Detailed refactoring plan and progress tracking
- [CHANGELOG.md](CHANGELOG.md) - Version history and release notes
- [WEB_API_DESIGN.md](WEB_API_DESIGN.md) - Web API design for REST and WebSocket endpoints
- [DEVELOPMENT.md](DEVELOPMENT.md) - Contribution guidelines for adding lenses and web endpoints
