# Monocle Architecture

This document describes the architecture of the `monocle` project: a BGP information toolkit that can be used as both a Rust library and a command-line application.

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
│       ├── asinfo.rs         # Unified AS information (replaces as2org)
│       ├── as2rel.rs         # AS relationships
│       ├── rpki.rs           # ROAs/ASPAs cache
│       ├── pfx2as.rs         # Prefix-to-ASN mappings
│       └── file_cache.rs     # File-based caching utilities
│
├── lens/                     # Business logic ("use-cases")
│   ├── mod.rs
│   ├── README.md
│   ├── utils.rs              # OutputFormat, formatting helpers
│   ├── country.rs            # Country code/name lookup
│   │
│   ├── as2rel/               # AS relationship lens
│   │   ├── mod.rs
│   │   ├── args.rs
│   │   └── types.rs
│   │
│   ├── inspect/              # Unified AS/prefix inspection (main entry point)
│   │   ├── mod.rs            # InspectLens implementation
│   │   └── types.rs          # Result types, section selection
│   │
│   ├── ip/                   # IP information lookup
│   │   └── mod.rs
│   │
│   ├── parse/                # MRT file parsing
│   │   └── mod.rs
│   │
│   ├── pfx2as/               # Prefix-to-ASN mapping lens
│   │   └── mod.rs            # Pfx2asLens with lookup/refresh
│   │
│   ├── rpki/                 # RPKI validation and data
│   │   ├── mod.rs            # RpkiLens with validation logic
│   │   └── commons.rs        # bgpkit-commons integration
│   │
│   ├── search/               # BGP message search
│   │   ├── mod.rs
│   │   └── query_builder.rs
│   │
│   └── time/                 # Time parsing and formatting
│       └── mod.rs
│
├── server/                   # WebSocket server
│   ├── mod.rs                # Server startup, handle_socket
│   ├── protocol.rs           # Core protocol types
│   ├── router.rs             # Router + Dispatcher
│   ├── handler.rs            # WsMethod trait, WsContext
│   ├── sink.rs               # WsSink (transport primitive)
│   ├── op_sink.rs            # WsOpSink (terminal-guarded)
│   ├── operations.rs         # Operation registry
│   └── handlers/             # Method handlers
│       ├── mod.rs
│       ├── inspect.rs        # inspect.query, inspect.refresh
│       ├── rpki.rs
│       ├── as2rel.rs
│       └── ...
│
└── bin/
    ├── monocle.rs            # CLI entry point
    └── commands/             # Command handlers (thin wrappers around lenses)
        ├── as2rel.rs
        ├── config.rs
        ├── country.rs
        ├── inspect.rs        # Unified inspect command
        ├── ip.rs
        ├── parse.rs
        ├── rpki.rs
        ├── search.rs
        └── time.rs
```

## Key Modules

### `inspect` - Unified AS/Prefix Information

The `inspect` command and lens consolidate multiple data sources into a single query interface:

- **ASInfo**: Core AS data, AS2Org, PeeringDB, Hegemony, Population
- **Connectivity**: AS2Rel-based upstream/peer/downstream relationships
- **RPKI**: ROAs and ASPA records
- **Pfx2as**: Prefix-to-ASN mappings

Features:
- Mixed query types (ASN, prefix, name search)
- Section selection (`--select`)
- Auto-refresh of stale data
- Multiple output formats

### `rpki` - RPKI Validation

The RPKI lens (`RpkiLens`) provides:
- **Validation logic** (RFC 6811): Valid/Invalid/NotFound states
- **Cache management**: Uses `RpkiRepository` for current data
- **Historical queries**: Uses bgpkit-commons for date-specific lookups

Layering:
- `RpkiRepository` (database): Raw data access only
- `RpkiLens` (lens): Validation logic, cache refresh, formatting

### `pfx2as` - Prefix-to-ASN Mapping

The Pfx2as lens (`Pfx2asLens`) provides:
- **Lookup modes**: Exact, longest, covering, covered
- **ASN queries**: Get all prefixes for an ASN
- **Cache management**: Download and store pfx2as data

### `server` - WebSocket API

The WebSocket server provides programmatic access to monocle functionality:

- **Protocol**: JSON-RPC style with request/response envelopes
- **Streaming**: Progress updates for long-running operations
- **Terminal guard**: `WsOpSink` ensures exactly one terminal response
- **Operation tracking**: `OperationRegistry` for cancellation support

## Module Architecture

### `config/` (Configuration + Status Reporting)

Responsibilities:
- compute default paths and load config file overrides
- provide shared helpers used by both `config` and `database` CLI commands to display:
  - SQLite database info (size, table counts, last update time)
  - cache settings and cache directory info

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
  - ASInfo (unified AS information from multiple sources)
  - AS2Rel (AS-level relationships)
  - RPKI (ROAs/ASPAs metadata and lookup tables)
  - Pfx2as (prefix-to-ASN mappings)
- file cache helpers for datasets that are stored outside SQLite (if applicable)

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

Monocle supports conditional compilation via Cargo features.

- `cli` (default):
  - enables clap derives and other CLI-only dependencies
  - required to build the `monocle` binary
- `full`:
  - currently aliases to `cli`

Library users can disable default features to reduce dependency footprint:

```toml
monocle = { version = "0.10", default-features = false }
```

## Related Documents

- `README.md` — user-facing CLI and library overview
- `REFACTOR_TODO.md` — refactoring progress and remaining tasks
- `src/database/README.md` — database module notes
- `src/lens/README.md` — lens module patterns and conventions
- `src/server/REFACTOR_PLAN.md` — WebSocket server design notes