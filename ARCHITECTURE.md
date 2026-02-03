# Monocle Architecture

This document describes the architecture of the `monocle` project: a BGP information toolkit that can be used as both a Rust library, a command-line application, and a WebSocket server.

## Goals and Design Principles

1. **Library-first**: the core capability lives in the library; the CLI is a thin wrapper.
2. **Clear separation of concerns**:
   - persistence and caching in `database/`
   - domain operations in `lens/`
   - presentation and UX concerns in the CLI (`bin/`)
3. **Extensible**: new functionality should be added as new lenses (and optionally wired into the CLI).
4. **Composability**: lenses should be usable programmatically and in batch/automation contexts.

## Layering Rules

The codebase follows strict layering rules to maintain separation of concerns:

### Repository Layer (`database/`)

Repositories are **data access only**:
- CRUD operations (Create, Read, Update, Delete)
- Query methods that return raw data
- No business logic or policy decisions
- No output formatting

### Lens Layer (`lens/`)

Lenses contain **business logic and policy**:
- Interpretation of data (e.g., RPKI validation logic)
- Coordination between multiple repositories
- Output formatting
- Cache refresh decisions

### CLI Layer (`bin/`)

CLI commands are **thin wrappers**:
- Argument parsing
- Output format selection
- Progress display
- Error presentation

## High-level Architecture

Monocle is organized into three primary layers:

- **CLI layer** (`src/bin/`):
  - parses flags/arguments
  - loads configuration
  - selects output format
  - calls into lenses
- **Lens layer** (`src/lens/`):
  - provides "use-case" APIs (e.g., search, parse, RPKI lookups)
  - controls output shaping via a unified `OutputFormat`
  - uses `database/` and external libraries as needed
- **Database layer** (`src/database/`):
  - manages storage (SQLite), schema initialization, and caching primitives
  - contains repositories for specific datasets

## Directory Structure

```
src/
├── lib.rs                    # Library entry point / exports
├── config.rs                 # Configuration and shared status helpers
│
├── database/                 # Persistence + caching
│   ├── mod.rs
│   ├── README.md
│   │
│   ├── core/                 # Connection + schema
│   │   ├── mod.rs
│   │   ├── connection.rs
│   │   └── schema.rs
│   │
│   ├── session/              # Ephemeral / per-run databases
│   │   ├── mod.rs
│   │   └── msg_store.rs
│   │
│   └── monocle/              # Main persistent monocle database
│       ├── mod.rs
│       ├── asinfo.rs         # Unified AS information (from bgpkit-commons)
│       ├── as2rel.rs         # AS relationships
│       ├── rpki.rs           # ROAs/ASPAs cache (SQLite with blob prefixes)
│       └── pfx2as.rs         # Prefix-to-ASN mappings (SQLite with blob prefixes)
│
├── lens/                     # Business logic ("use-cases")
│   ├── mod.rs
│   ├── README.md
│   ├── utils.rs              # OutputFormat, formatting helpers
│   ├── country.rs            # Country code/name lookup (lens-bgpkit)
│   │
│   ├── as2rel/               # AS relationship lens (lens-bgpkit)
│   │   ├── mod.rs
│   │   ├── args.rs
│   │   └── types.rs
│   │
│   ├── inspect/              # Unified AS/prefix inspection (lens-full)
│   │   ├── mod.rs            # InspectLens implementation
│   │   └── types.rs          # Result types, section selection
│   │
│   ├── ip/                   # IP information lookup (lens-bgpkit)
│   │   └── mod.rs
│   │
│   ├── parse/                # MRT file parsing (lens-bgpkit)
│   │   └── mod.rs
│   │
│   ├── pfx2as/               # Prefix-to-ASN mapping types (lens-bgpkit)
│   │   └── mod.rs            # Types only; repository handles lookups
│   │
│   ├── rpki/                 # RPKI validation and data (lens-bgpkit)
│   │   ├── mod.rs            # RpkiLens with validation logic
│   │   └── commons.rs        # bgpkit-commons integration
│   │
│   ├── search/               # BGP message search (lens-bgpkit)
│   │   ├── mod.rs
│   │   └── query_builder.rs
│   │
│   └── time/                 # Time parsing and formatting (lens-core)
│       └── mod.rs
│
├── server/                   # WebSocket server (cli feature)
│   ├── mod.rs                # Server startup, handle_socket
│   ├── protocol.rs           # Core protocol types (RequestEnvelope, ResponseEnvelope)
│   ├── router.rs             # Router + Dispatcher
│   ├── handler.rs            # WsMethod trait, WsContext
│   ├── sink.rs               # WsSink (transport primitive)
│   ├── op_sink.rs            # WsOpSink (terminal-guarded)
│   ├── operations.rs         # Operation registry for cancellation
│   └── handlers/             # Method handlers
│       ├── mod.rs
│       ├── inspect.rs        # inspect.query, inspect.refresh
│       ├── rpki.rs           # rpki.validate, rpki.roas, rpki.aspas
│       ├── as2rel.rs         # as2rel.search, as2rel.relationship
│       ├── database.rs       # database.status, database.refresh
│       ├── parse.rs          # parse.start, parse.cancel (streaming)
│       ├── search.rs         # search.start, search.cancel (streaming)
│       └── ...
│
└── bin/
    ├── monocle.rs            # CLI entry point
    └── commands/             # Command handlers (thin wrappers around lenses)
        ├── as2rel.rs
        ├── config.rs         # Config display + update, backup, sources
        ├── country.rs
        ├── inspect.rs        # Unified inspect command (replaces whois, pfx2as)
        ├── ip.rs
        ├── parse.rs
        ├── rpki.rs
        ├── search.rs
        └── time.rs
```

## Key Modules

### `inspect` - Unified AS/Prefix Information

The `inspect` command and lens consolidate multiple data sources into a single query interface:

- **ASInfo**: Core AS data from bgpkit-commons (replaces as2org)
- **Connectivity**: AS2Rel-based upstream/peer/downstream relationships
- **RPKI**: ROAs and ASPA records
- **Pfx2as**: Prefix-to-ASN mappings

Features:
- Auto-detects query type (ASN, prefix, IP address, or name)
- Section selection (`--show basic/prefixes/connectivity/rpki/all`)
- Display limits with `--full`, `--full-roas`, `--full-prefixes`, `--full-connectivity`
- Auto-refresh of stale data
- Multiple output formats

### `rpki` - RPKI Validation

The RPKI lens (`RpkiLens`) provides:
- **Validation logic** (RFC 6811): Valid/Invalid/NotFound states
- **Cache management**: Uses `RpkiRepository` for current data (SQLite with blob prefixes)
- **Historical queries**: Uses bgpkit-commons for date-specific lookups

Layering:
- `RpkiRepository` (database): Raw data access only (CRUD, prefix range queries)
- `RpkiLens` (lens): Validation logic, cache refresh, formatting

### `pfx2as` - Prefix-to-ASN Mapping

The Pfx2as repository (`Pfx2asRepository`) provides:
- **Lookup modes**: Exact, longest prefix match, covering (supernets), covered (subnets)
- **ASN queries**: Get all prefixes for an ASN
- **SQLite storage**: IP prefixes stored as 16-byte start/end address pairs
- **Cache management**: 24-hour TTL with automatic refresh

Note: The file-based cache has been removed; all pfx2as data now uses SQLite.

### `server` - WebSocket API

The WebSocket server (`monocle server`) provides programmatic access to monocle functionality:

- **Protocol**: JSON-RPC style with request/response envelopes
- **Streaming**: Progress updates for long-running operations (parse, search)
- **Terminal guard**: `WsOpSink` ensures exactly one terminal response per operation
- **Operation tracking**: `OperationRegistry` for cancellation support via `op_id`
- **DB-first policy**: Queries read from local SQLite cache

Available method namespaces:
- `system.*`: Server introspection (info, methods)
- `time.*`, `ip.*`, `country.*`: Utility lookups
- `rpki.*`, `as2rel.*`, `pfx2as.*`: BGP data queries
- `inspect.*`: Unified AS/prefix inspection
- `parse.*`, `search.*`: Streaming MRT operations
- `database.*`: Database management

## Module Architecture

### `config/` (Configuration + Status Reporting)

Responsibilities:
- compute default paths and load config file overrides
- provide shared helpers used by `config` CLI command to display:
  - SQLite database info (size, table counts, last update time)
  - cache settings and cache directory info
  - database management (refresh, backup, sources)

This module is intentionally "infra-ish": it should not implement domain logic.

### `database/` (Persistence and Caching)

The `database` module handles local persistence and caches shared across commands.

#### `database/core/`

Responsibilities:
- create/configure SQLite connections
- define and initialize schema
- expose schema/version checks (used on open)

Notes:
- schema management is owned here; higher-level modules should not issue `CREATE TABLE` etc.

#### `database/monocle/`

Responsibilities:
- main persistent monocle dataset store (SQLite DB under the monocle data directory)
- repositories for datasets:
  - ASInfo (unified AS information from bgpkit-commons)
  - AS2Rel (AS-level relationships)
  - RPKI (ROAs/ASPAs with blob-based prefix storage)
  - Pfx2as (prefix-to-ASN mappings with blob-based prefix storage)
- file cache helpers for auxiliary file-based caching

Key idea:
- `MonocleDatabase` is the entry point for accessing the persistent DB/repositories.

#### `database/session/`

Responsibilities:
- short-lived, per-operation SQLite storage (e.g., search results)
- optimized for write-heavy temporary usage and easy export

### `lens/` (Business Logic / Use-cases)

Lenses are the primary public-facing API surface for functionality. A lens:
- takes a `&MonocleDatabase` reference for data access
- defines **argument types** (often serde-serializable; optionally clap-derivable under `cli`)
- defines **result types** (serde-serializable)
- performs the operation (may call into Broker, Parser, SQLite repositories, file caches, etc.)
- emits output using the **unified `OutputFormat`**

#### Output formatting

`lens/utils.rs` contains the global `OutputFormat` used across the CLI to keep formatting consistent and predictable.

#### Progress reporting

Certain lenses support progress callbacks (e.g., parse/search). Progress types are designed to be:
- thread-safe (`Send + Sync`)
- serializable (for GUI or other frontends)

### `bin/` (CLI)

The CLI layer wires together:
- clap argument parsing
- config loading
- output selection (`--format`, `--json`)
- invocation of lens operations
- printing human-readable messages to stderr and data output to stdout (to support piping)

The CLI should not duplicate core logic. It should:
- validate/normalize CLI inputs
- call library APIs
- format/print results

## Typical Data Flows

### CLI flow (conceptual)

1. User runs `monocle <command> ...`
2. CLI parses args and loads `MonocleConfig`
3. CLI determines `OutputFormat`
4. CLI opens `MonocleDatabase` and constructs the lens
5. Lens executes operation:
   - uses repository for data access
   - applies business logic
   - returns typed results (and optionally progress events via callback)
6. CLI prints results via `OutputFormat`

### Library flow (conceptual)

1. Application creates (or opens) the `MonocleDatabase`
2. Application constructs lens with database reference
3. Application calls lens methods
4. Application consumes typed results directly, or uses `OutputFormat` to format for display

### WebSocket flow (conceptual)

1. Client connects and sends JSON request envelope
2. Router dispatches to appropriate handler
3. Handler creates lens with database reference
4. Lens executes operation (may send progress via `WsOpSink`)
5. Handler sends terminal result/error via `WsOpSink`

## Feature Flags

Monocle supports conditional compilation via Cargo features with a simplified three-tier structure:

### Feature Hierarchy

```
cli (default)
 ├── server
 │    └── lib
 └── lib
```

**Quick Guide:**
- **Need the CLI binary?** Use `cli` (includes everything)
- **Need WebSocket server without CLI?** Use `server` (includes lib)
- **Need only library/data access?** Use `lib` (database + all lenses + display)

### Feature Descriptions

| Feature | Description | Key Dependencies |
|---------|-------------|------------------|
| `lib` | Complete library: database + all lenses + display | `rusqlite`, `bgpkit-parser`, `bgpkit-broker`, `tabled`, etc. |
| `server` | WebSocket server (implies `lib`) | `axum`, `tokio`, `serde_json` |
| `cli` | Full CLI binary with progress bars (implies `lib` and `server`) | `clap`, `indicatif` |

### Use Case Scenarios

#### Scenario 1: Library Only
**Features**: `lib`

Use when building applications that need:
- Database operations (SQLite, data loading)
- All lenses (TimeLens, ParseLens, SearchLens, RPKI, Country, InspectLens, etc.)
- Table formatting with tabled

```toml
monocle = { version = "1.0", default-features = false, features = ["lib"] }
```

```rust
use monocle::database::MonocleDatabase;
use monocle::lens::inspect::{InspectLens, InspectQueryOptions};

let db = MonocleDatabase::open_in_dir("~/.monocle")?;
let lens = InspectLens::new(&db);
let result = lens.query("AS13335", &InspectQueryOptions::default())?;
```

#### Scenario 2: Library with WebSocket Server
**Features**: `server`

Use when building applications that need:
- Everything in `lib`
- WebSocket server for remote API access

```toml
monocle = { version = "1.0", default-features = false, features = ["server"] }
```

```rust
use monocle::server::start_server;

// Start WebSocket server on default port
start_server("127.0.0.1:3000").await?;
```

#### Scenario 3: CLI Binary (Default)
**Features**: `cli` (default)

The full CLI binary with all features, WebSocket server, and terminal UI:

```toml
monocle = "1.0"
```

Or explicitly:

```toml
monocle = { version = "1.0", features = ["cli"] }
```

### Valid Feature Combinations

All of these combinations compile successfully:

| Combination | Use Case |
|-------------|----------|
| (none) | Config types only, no functionality |
| `lib` | Full library functionality |
| `server` | Library + WebSocket server |
| `cli` | Full CLI (includes everything) |

### Feature Dependencies

When you enable a higher-tier feature, lower-tier features are automatically included:

- `server` → automatically enables `lib`
- `cli` → automatically enables `lib` and `server`

## Related Documents

- `README.md` — user-facing CLI and library overview
- `CHANGELOG.md` — version history and breaking changes
- `DEVELOPMENT.md` — contributor guide for adding lenses and fixing bugs
- `src/server/README.md` — WebSocket API protocol specification
- `src/database/README.md` — database module notes
- `src/lens/README.md` — lens module patterns and conventions
- `examples/README.md` — example code organized by feature tier
