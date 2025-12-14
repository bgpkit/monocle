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

## High-level Architecture

Monocle is organized into three primary layers:

- **CLI layer** (`src/bin/`):
  - parses flags/arguments
  - loads configuration
  - selects output format
  - calls into lenses
- **Lens layer** (`src/lens/`):
  - provides “use-case” APIs (e.g., search, parse, RPKI lookups)
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
│       ├── as2org.rs
│       ├── as2rel.rs
│       ├── rpki.rs
│       └── file_cache.rs
│
├── lens/                     # Business logic (“use-cases”)
│   ├── mod.rs
│   ├── README.md
│   ├── utils.rs              # OutputFormat, formatting helpers
│   │
│   ├── as2org/
│   │   ├── mod.rs
│   │   ├── args.rs
│   │   └── types.rs
│   ├── as2rel/
│   │   ├── mod.rs
│   │   ├── args.rs
│   │   └── types.rs
│   ├── country.rs
│   ├── ip/
│   │   └── mod.rs
│   ├── parse/
│   │   └── mod.rs
│   ├── pfx2as/
│   │   └── mod.rs
│   ├── rpki/
│   │   ├── mod.rs
│   │   └── commons.rs
│   ├── search/
│   │   ├── mod.rs
│   │   └── query_builder.rs
│   └── time/
│       └── mod.rs
│
└── bin/
    ├── monocle.rs            # CLI entry point
    └── commands/             # Command handlers (thin wrappers around lenses)
        ├── as2rel.rs
        ├── config.rs
        ├── country.rs
        ├── database.rs
        ├── ip.rs
        ├── parse.rs
        ├── pfx2as.rs
        ├── rpki.rs
        ├── search.rs
        ├── time.rs
        └── whois.rs
```

## Module Architecture

### `config/` (Configuration + Status Reporting)

Responsibilities:
- compute default paths and load config file overrides
- provide shared helpers used by both `config` and `database` CLI commands to display:
  - SQLite database info (size, table counts, last update time)
  - cache settings and cache directory info

This module is intentionally “infra-ish”: it should not implement domain logic.

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
  - AS-to-org (as2org)
  - AS relationships (as2rel)
  - RPKI (ROAs/ASPAs metadata and lookup tables)
- file cache helpers for datasets that are stored outside SQLite (if applicable)

Key idea:
- `MonocleDatabase` is the entry point for accessing the persistent DB/repositories.

#### `database/session/`

Responsibilities:
- short-lived, per-operation SQLite storage (e.g., search results)
- optimized for write-heavy temporary usage and easy export

### `lens/` (Business Logic / Use-cases)

Lenses are the primary public-facing API surface for functionality. A lens:
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
4. CLI constructs the lens + arguments
5. Lens executes operation:
   - may open `MonocleDatabase`
   - may fetch/update caches/data when needed
   - returns typed results (and optionally progress events via callback)
6. CLI prints results via `OutputFormat`

### Library flow (conceptual)

1. Application creates (or opens) the relevant database objects (when required)
2. Application constructs lens args and calls lens methods
3. Application consumes typed results directly, or uses `OutputFormat` to format for display

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
- `DEVELOPMENT.md` — contributor guide (how to add lenses, tests, style)
- `src/database/README.md` — database module notes
- `src/lens/README.md` — lens module patterns and conventions