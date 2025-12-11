# Monocle Refactoring Plan

## Overview

This document outlines a comprehensive refactoring plan for the monocle project to:
1. Consolidate database functionality under a unified module
2. Create a clean separation between data access, business logic, and presentation
3. Enable library reuse across CLI, web API, and GUI applications
4. Design for future extensibility with new BGP data types

---

## Current State Analysis

### Database-Related Code (Scattered)
| Location | Component | Purpose |
|----------|-----------|---------|
| `src/database.rs` | `MonocleDatabase`, `MsgStore` | Connection wrapper, session-based BGP message storage |
| `src/datasets/as2org.rs` | `As2org` | Own database initialization, schema, and queries |
| `src/datasets/as2rel.rs` | `As2rel` | Own database initialization, schema, and queries |

### Overlap Between `datasets/` and `bin/commands/`
| Dataset | Command | Duplication |
|---------|---------|-------------|
| `datasets/as2org.rs` | `commands/whois.rs` | CLI wrapper with similar logic |
| `datasets/as2rel.rs` | `commands/as2rel.rs` | CLI wrapper with similar logic |
| `datasets/pfx2as.rs` | `commands/pfx2as.rs` | CLI wrapper with similar logic |

### Key Issues
1. Each database-backed dataset manages its own schema independently
2. No unified schema management or migration strategy
3. CLI args and core logic are split across two locations
4. No reusability outside of CLI context
5. Hard to add cross-dataset queries (e.g., JOIN as2org with as2rel)

---

## Proposed Architecture

```
src/
├── lib.rs                    # Public API exports (feature-gated)
├── config.rs                 # MonocleConfig (enhanced)
├── time.rs                   # Time utilities
├── filters/                  # BGP message filters
│
├── database/                 # ═══ ALL DATABASE FUNCTIONALITY ═══
│   ├── mod.rs                # Module exports
│   │
│   ├── core/                 # Core database infrastructure
│   │   ├── mod.rs
│   │   ├── connection.rs     # MonocleDatabase (base connection)
│   │   ├── schema.rs         # Unified schema management & migrations
│   │   └── migration.rs      # Schema versioning and drift detection
│   │
│   ├── session/              # One-time/session databases
│   │   ├── mod.rs
│   │   └── msg_store.rs      # BGP search results (per-search instance)
│   │
│   └── shared/               # Shared persistent database
│       ├── mod.rs            # SharedDatabase struct (single entry point)
│       ├── as2org.rs         # AS2Org data access layer
│       ├── as2rel.rs         # AS2Rel data access layer
│       └── ...               # Future: pfx2as_cache, rpki_roas, etc.
│
├── services/                 # ═══ BUSINESS LOGIC + TYPES ═══
│   ├── mod.rs                # Service exports
│   │
│   ├── as2org/               # AS2Org service
│   │   ├── mod.rs            # Service struct, core logic
│   │   ├── types.rs          # SearchResult, SearchType, etc.
│   │   └── args.rs           # Reusable argument definitions
│   │
│   ├── as2rel/               # AS2Rel service
│   │   ├── mod.rs
│   │   ├── types.rs
│   │   └── args.rs
│   │
│   ├── search/               # BGP search service
│   │   ├── mod.rs
│   │   ├── types.rs
│   │   └── args.rs
│   │
│   ├── country.rs            # Country lookup (in-memory, no DB)
│   ├── pfx2as.rs             # Prefix-to-ASN (in-memory trie)
│   ├── ip.rs                 # IP information
│   ├── radar.rs              # Cloudflare Radar
│   └── rpki/                 # RPKI utilities
│
├── server/                   # ═══ WEB SERVER (feature-gated) ═══
│   ├── mod.rs
│   ├── websocket.rs          # WebSocket handler
│   ├── handlers/             # Request handlers per service
│   │   ├── mod.rs
│   │   ├── as2org.rs
│   │   ├── as2rel.rs
│   │   └── search.rs
│   └── protocol.rs           # Message types for client-server communication
│
└── bin/
    └── monocle.rs            # ═══ THIN CLI LAYER ═══
                              # Clap args, dispatch to services
```

---

## Design Decisions

### 1. Database Module (`database/`)

#### Core Infrastructure

```rust
// database/core/connection.rs
pub struct MonocleDatabase {
    pub conn: Connection,
}

impl MonocleDatabase {
    pub fn open(path: &str) -> Result<Self>;
    pub fn open_in_memory() -> Result<Self>;
}

// database/core/schema.rs
pub const SCHEMA_VERSION: u32 = 1;

pub struct SchemaManager<'a> {
    conn: &'a Connection,
}

impl SchemaManager<'_> {
    pub fn initialize(&self) -> Result<()>;
    pub fn check_version(&self) -> Result<SchemaStatus>;
    pub fn migrate(&self) -> Result<()>;
}

pub enum SchemaStatus {
    Current,
    NeedsMigration { from: u32, to: u32 },
    Incompatible,  // Requires reset
}

// database/core/migration.rs
pub struct MigrationManager {
    // Handles schema drift detection and automatic reset/repopulation
}
```

#### Shared Database

```rust
// database/shared/mod.rs
pub struct SharedDatabase {
    db: MonocleDatabase,
}

impl SharedDatabase {
    /// Opens the shared database, checking schema and resetting if needed
    pub fn new(data_dir: &str) -> Result<Self> {
        let db = MonocleDatabase::open(&format!("{}/monocle-data.sqlite3", data_dir))?;
        let schema = SchemaManager::new(&db.conn);
        
        match schema.check_version()? {
            SchemaStatus::Current => {},
            SchemaStatus::NeedsMigration { .. } => schema.migrate()?,
            SchemaStatus::Incompatible => {
                // Reset and repopulate - completely behind the scenes
                schema.reset()?;
            }
        }
        
        Ok(Self { db })
    }
    
    pub fn as2org(&self) -> As2orgRepository<'_>;
    pub fn as2rel(&self) -> As2relRepository<'_>;
    // Future: pub fn rpki_roas(&self) -> RpkiRoasRepository<'_>;
}
```

#### Benefits
- Single entry point to the shared database
- Automatic schema management (invisible to user)
- Unified versioning across all data types
- Enables cross-table queries (JOINs)
- Easy to add new data types

### 2. Services Module (`services/`)

#### Reusable Arguments Pattern

Arguments are defined in the library and can be reused across CLI, web API, and GUI:

```rust
// services/as2org/args.rs
use serde::{Deserialize, Serialize};

/// Search parameters for AS2Org queries
/// Reusable across CLI (clap), REST API (query params), WebSocket (JSON), GUI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct As2orgSearchArgs {
    /// Search query: ASN (e.g., "400644") or name (e.g., "bgpkit")
    #[cfg_attr(feature = "cli", clap(required = true))]
    pub query: Vec<String>,
    
    /// Search AS and Org name only
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub name_only: bool,
    
    /// Search by ASN only
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub asn_only: bool,
    
    /// Search by country only
    #[cfg_attr(feature = "cli", clap(short = 'C', long))]
    #[serde(default)]
    pub country_only: bool,
    
    /// Show full country names instead of 2-letter code
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub full_country: bool,
}

/// Output format options
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum OutputFormat {
    #[default]
    Markdown,
    Pretty,
    Json,
    Psv,
}
```

#### Service Implementation

```rust
// services/as2org/mod.rs
pub struct As2orgService<'a> {
    repo: As2orgRepository<'a>,
    country_lookup: CountryLookup,
}

impl<'a> As2orgService<'a> {
    pub fn new(shared_db: &'a SharedDatabase) -> Self;
    
    // Core functionality
    pub fn search(&self, args: &As2orgSearchArgs) -> Result<Vec<SearchResult>>;
    pub fn lookup_org_name(&self, asn: u32) -> Option<String>;
    pub fn update(&self) -> Result<UpdateProgress>;
    pub fn is_data_available(&self) -> bool;
    
    // Streaming updates (for WebSocket)
    pub fn update_with_progress<F>(&self, callback: F) -> Result<()>
    where F: Fn(UpdateProgress);
    
    // Output formatting (reusable in CLI and web API)
    pub fn format_results(&self, results: &[SearchResult], format: OutputFormat) -> String;
}

// Progress reporting for long-running operations
pub struct UpdateProgress {
    pub stage: UpdateStage,
    pub current: usize,
    pub total: Option<usize>,
    pub message: String,
}

pub enum UpdateStage {
    Downloading,
    Parsing,
    Inserting,
    Complete,
    Error(String),
}
```

### 3. Web Server Module (`server/`)

#### WebSocket-First Design

WebSocket is the primary communication method for several reasons:
1. **Connection awareness**: Know if client disconnects (avoid wasted computation)
2. **Streaming progress**: Real-time feedback for long operations
3. **Bidirectional**: Client can cancel operations mid-flight

```rust
// server/protocol.rs
use serde::{Deserialize, Serialize};

/// Client-to-server message
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ClientMessage {
    // AS2Org operations
    As2orgSearch(As2orgSearchArgs),
    As2orgUpdate,
    
    // AS2Rel operations
    As2relSearch(As2relSearchArgs),
    As2relUpdate,
    
    // BGP Search operations
    SearchStart(SearchArgs),
    SearchCancel { request_id: String },
    
    // Control
    Ping,
}

/// Server-to-client message
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerMessage {
    // Results
    SearchResult { request_id: String, data: serde_json::Value },
    SearchComplete { request_id: String, total: usize },
    
    // Progress updates
    Progress { request_id: String, progress: ProgressInfo },
    
    // Errors
    Error { request_id: Option<String>, message: String },
    
    // Control
    Pong,
    Connected { session_id: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProgressInfo {
    pub stage: String,
    pub current: usize,
    pub total: Option<usize>,
    pub message: String,
    pub percent: Option<f32>,
}
```

#### Server Implementation

```rust
// server/mod.rs
pub struct MonocleServer {
    config: MonocleConfig,
    shared_db: SharedDatabase,
}

impl MonocleServer {
    pub fn new(config: MonocleConfig) -> Result<Self>;
    
    /// Start the server on the given address
    pub async fn run(&self, addr: &str) -> Result<()>;
}

// server/websocket.rs
pub async fn handle_websocket(
    ws: WebSocket,
    shared_db: Arc<SharedDatabase>,
) -> Result<()> {
    // Handle incoming messages, dispatch to services
    // Stream results back to client
    // Handle cancellation on disconnect
}
```

### 4. Feature Flags

```toml
# Cargo.toml
[features]
default = ["cli"]

# CLI support (clap derives, terminal output)
cli = ["clap"]

# Web server support
server = ["tokio", "axum", "tokio-tungstenite"]

# Full library with all features
full = ["cli", "server"]

# Minimal library (no CLI, no server)
# Just: database, services, types
```

Usage:
```rust
// As a library (minimal)
// Cargo.toml: monocle = { version = "x.y", default-features = false }

// As a library with CLI args
// Cargo.toml: monocle = { version = "x.y", features = ["cli"] }

// Full binary
// Cargo.toml: monocle = { version = "x.y", features = ["full"] }
```

### 5. GUI Considerations (GPUI)

The architecture supports GPUI integration:

```rust
// Example GPUI integration (separate crate: monocle-gui)
use monocle::{SharedDatabase, As2orgService, As2orgSearchArgs};

struct As2orgView {
    service: As2orgService<'static>,
    search_args: As2orgSearchArgs,
    results: Vec<SearchResult>,
    loading: bool,
}

impl As2orgView {
    fn search(&mut self, cx: &mut ViewContext<Self>) {
        self.loading = true;
        let args = self.search_args.clone();
        
        cx.spawn(|this, mut cx| async move {
            let results = this.read(&cx).service.search(&args);
            this.update(&mut cx, |this, cx| {
                this.results = results.unwrap_or_default();
                this.loading = false;
                cx.notify();
            });
        }).detach();
    }
}
```

Key design points for GUI compatibility:
1. **No blocking I/O in main thread**: Services support async patterns
2. **Progress callbacks**: Long operations report progress
3. **Cancellation support**: Operations can be cancelled
4. **Serializable types**: All types derive `Serialize`/`Deserialize`
5. **Shared args**: Same `Args` structs work in GUI forms

---

## Schema Versioning Strategy

### Simple Migration Approach (No External Libraries)

```rust
// database/core/migration.rs

const SCHEMA_VERSION: u32 = 1;

const MIGRATIONS: &[(u32, &str)] = &[
    (1, include_str!("migrations/001_initial.sql")),
    // Future migrations:
    // (2, include_str!("migrations/002_add_rpki.sql")),
];

pub struct MigrationManager<'a> {
    conn: &'a Connection,
}

impl MigrationManager<'_> {
    pub fn check_and_migrate(&self) -> Result<MigrationResult> {
        let current = self.get_current_version()?;
        
        if current == 0 {
            // Fresh database
            self.apply_all_migrations()?;
            return Ok(MigrationResult::Initialized);
        }
        
        if current == SCHEMA_VERSION {
            return Ok(MigrationResult::Current);
        }
        
        if current > SCHEMA_VERSION {
            // Database from a newer version - need to reset
            self.reset_database()?;
            self.apply_all_migrations()?;
            return Ok(MigrationResult::Reset);
        }
        
        // Apply pending migrations
        for (version, sql) in MIGRATIONS.iter() {
            if *version > current {
                self.apply_migration(*version, sql)?;
            }
        }
        
        Ok(MigrationResult::Migrated { from: current, to: SCHEMA_VERSION })
    }
    
    fn reset_database(&self) -> Result<()> {
        // Drop all tables and repopulate
        // This is acceptable since our data can be redownloaded
    }
}

pub enum MigrationResult {
    Current,
    Initialized,
    Migrated { from: u32, to: u32 },
    Reset,  // Incompatible schema, had to reset
}
```

### Schema Drift Detection

```rust
impl MigrationManager<'_> {
    pub fn verify_schema_integrity(&self) -> Result<bool> {
        // Check that expected tables exist with expected columns
        let expected_tables = ["monocle_meta", "as2org_as", "as2org_org", "as2rel", "as2rel_meta"];
        
        for table in expected_tables {
            if !self.table_exists(table)? {
                return Ok(false);
            }
        }
        
        // Could also verify column types if needed
        Ok(true)
    }
}
```

---

## Migration Path

### Phase 1: Create `database/` Module ✅ COMPLETED
**Goal**: Consolidate all database code

1. ✅ Create directory structure:
   - `database/core/` (connection, schema)
   - `database/session/` (msg_store)
   - `database/shared/` (as2org, as2rel repositories)

2. ✅ Implement `SchemaManager` with schema definitions

3. ✅ Create `SharedDatabase` as the unified entry point

4. ✅ Extract database operations from existing `as2org.rs` and `as2rel.rs`

**Files created:**
- `src/database/mod.rs` - Module exports and documentation
- `src/database/core/mod.rs` - Core infrastructure exports
- `src/database/core/connection.rs` - MonocleDatabase connection wrapper
- `src/database/core/schema.rs` - Schema definitions and management
- `src/database/session/mod.rs` - Session database exports
- `src/database/session/msg_store.rs` - BGP message store
- `src/database/shared/mod.rs` - SharedDatabase implementation
- `src/database/shared/as2org.rs` - AS2Org repository
- `src/database/shared/as2rel.rs` - AS2Rel repository

### Phase 2: Create `services/` Module ✅ COMPLETED
**Goal**: Clean separation of business logic

1. ✅ Create `services/` directory

2. ✅ Move business logic from `datasets/`:
   - `as2org.rs` → `services/as2org/`
   - `as2rel.rs` → `services/as2rel/`

3. ✅ Create `args.rs` files with reusable argument structs

4. ✅ Update services to use repository pattern from `database/`

**Files created:**
- `src/services/mod.rs` - Service exports and documentation
- `src/services/as2org/mod.rs` - AS2Org service implementation
- `src/services/as2org/types.rs` - Result types and enums
- `src/services/as2org/args.rs` - Reusable argument structs
- `src/services/as2rel/mod.rs` - AS2Rel service implementation
- `src/services/as2rel/types.rs` - Result types and enums
- `src/services/as2rel/args.rs` - Reusable argument structs
- `src/services/country.rs` - Country lookup service

### Phase 3: Implement Feature Flags ✅ COMPLETED
**Goal**: Enable library usage with minimal dependencies

1. ✅ Added feature flags to `Cargo.toml`:
   - `default = ["cli"]` - CLI enabled by default
   - `cli` - gates clap, indicatif, json_to_table, tracing-subscriber
   - `server` - placeholder for future web server dependencies
   - `full` - enables all features

2. ✅ Gated CLI-specific code behind `cli` feature:
   - Updated `filters/mod.rs`, `filters/parse.rs`, `filters/search.rs`
   - Filter structs use `#[cfg_attr(feature = "cli", derive(Args))]`
   - `ElemTypeEnum` and `DumpType` conditionally derive `ValueEnum`
   - Added `Serialize`/`Deserialize` to filter types for API usage

3. ✅ Minimal build compiles without CLI dependencies:
   - `cargo build --no-default-features` succeeds
   - All 74 tests pass with and without CLI feature
   - Binary requires `cli` feature via `required-features = ["cli"]`

4. ✅ Library can be used with minimal dependencies:
   - Core: database, services, filters (without clap derives)
   - CLI: adds argument parsing, progress bars, table formatting
   - Server: placeholder for Phase 4

**Files modified:**
- `Cargo.toml` - Feature definitions and optional dependencies
- `src/filters/mod.rs` - Feature-gated clap imports
- `src/filters/parse.rs` - Feature-gated Args derive
- `src/filters/search.rs` - Feature-gated Args derive

**Usage examples:**
```toml
# Minimal library (no CLI deps)
monocle = { version = "0.9", default-features = false }

# Library with CLI argument structs
monocle = { version = "0.9", features = ["cli"] }

# Full build (default)
monocle = { version = "0.9" }
```

### Phase 4: Web Server Implementation
**Goal**: Enable web API and WebSocket support

1. Create `server/` module (behind `server` feature)

2. Implement WebSocket handler

3. Create handlers for each service

4. Add progress streaming for long operations

### Phase 5: Cleanup and Documentation ✅ COMPLETED
**Goal**: Production-ready release

1. ✅ Remove old `datasets/` module
   - Legacy module completely removed
   - Useful utilities moved to new `utils/` module
   - Clean separation: database, services, filters, utils

2. ✅ Update all imports and exports in `lib.rs`
   - Clean separation of database, services, filters, and utils modules
   - Services types re-exported at top level for convenience
   - Feature-gated exports properly organized

3. ✅ Update `bin/monocle.rs` to use new structure
   - ✅ `whois` command migrated to use `As2orgService`
   - ✅ `as2rel` command migrated to use `As2relService`
   - ✅ `country` command updated to use `services::CountryLookup`
   - ✅ Other commands use library exports from appropriate modules

4. ✅ Country lookup now uses bgpkit-commons instead of hardcoded data
   - `src/services/country.rs` uses `BgpkitCommons::load_countries()`
   - Data is lazy-loaded and cached globally

5. ✅ Added example for library usage
   - `examples/search_bgp_messages.rs` - demonstrates searching BGP messages

6. ✅ Comprehensive documentation
   - `src/database/README.md` - Database module architecture and usage
   - `src/services/README.md` - Services module patterns and examples
   - Updated `ARCHITECTURE.md` with phase completion status
   - Updated `lib.rs` with clear module organization

7. ✅ Created `utils/` module for standalone utilities
   - `utils/ip.rs` - IP information lookup
   - `utils/pfx2as.rs` - Prefix-to-AS mapping
   - `utils/rpki/` - RPKI validation and data access

**Files created/modified in this phase:**
- `src/lib.rs` - Reorganized exports, clean module structure
- `src/utils/mod.rs` - New utilities module
- `src/utils/ip.rs` - Moved from datasets
- `src/utils/pfx2as.rs` - Moved from datasets
- `src/utils/rpki/` - Moved from datasets
- `src/bin/commands/whois.rs` - Uses `SharedDatabase` and `As2orgService`
- `src/bin/commands/as2rel.rs` - Uses `SharedDatabase` and `As2relService`
- `src/bin/commands/country.rs` - Uses `services::CountryLookup`
- `src/services/country.rs` - Uses bgpkit-commons
- `src/database/README.md` - New documentation file
- `src/services/README.md` - New documentation file
- `examples/search_bgp_messages.rs` - New example file
- `ARCHITECTURE.md` - Updated implementation status
- Deleted: `src/datasets/` - Legacy module removed

---

## Future Extensibility

### Adding New Data Types

With this structure, adding a new data type (e.g., `rpki_roas`) involves:

1. **Database layer** (`database/shared/rpki_roas.rs`):
   ```rust
   pub struct RpkiRoasRepository<'a> { ... }
   impl RpkiRoasRepository<'_> {
       pub fn insert_roas(&self, roas: &[Roa]) -> Result<()>;
       pub fn lookup_prefix(&self, prefix: IpNet) -> Result<Vec<Roa>>;
   }
   ```

2. **Schema migration** (`migrations/002_add_rpki.sql`):
   ```sql
   CREATE TABLE rpki_roas (
       prefix TEXT NOT NULL,
       max_length INTEGER NOT NULL,
       asn INTEGER NOT NULL,
       ta TEXT NOT NULL,
       PRIMARY KEY (prefix, asn, ta)
   );
   ```

3. **Service layer** (`services/rpki_roas/`):
   ```rust
   pub struct RpkiRoasService { ... }
   pub struct RpkiRoasArgs { ... }
   ```

4. **CLI/API** (minimal additions)

### Cross-Data Queries

The shared database enables queries like:

```sql
-- Find AS relationships with organization names
SELECT 
    r.asn1, o1.org_name as org1,
    r.asn2, o2.org_name as org2,
    r.rel
FROM as2rel r
JOIN as2org_all o1 ON r.asn1 = o1.asn
JOIN as2org_all o2 ON r.asn2 = o2.asn;

-- Future: Validate prefixes against RPKI and show origin org
SELECT 
    p.prefix, p.origin_asn,
    o.org_name,
    r.validity
FROM observed_prefixes p
JOIN as2org_all o ON p.origin_asn = o.asn
JOIN rpki_roas r ON p.prefix = r.prefix AND p.origin_asn = r.asn;
```

---

## GUI Architecture (GPUI)

### Separate Crate Structure

```
monocle/                    # Core library
monocle-server/             # Web server binary (optional)
monocle-gui/                # GPUI desktop application
    ├── Cargo.toml
    └── src/
        ├── main.rs
        ├── app.rs          # Application state
        ├── views/
        │   ├── mod.rs
        │   ├── as2org.rs   # AS2Org search view
        │   ├── as2rel.rs   # AS2Rel view
        │   └── search.rs   # BGP search view
        └── components/
            ├── mod.rs
            ├── table.rs    # Results table
            └── progress.rs # Progress indicator
```

### Key Integration Points

```rust
// monocle-gui/src/app.rs
use monocle::{MonocleConfig, SharedDatabase};
use monocle::services::{As2orgService, As2relService, SearchService};

pub struct MonocleApp {
    config: MonocleConfig,
    shared_db: Arc<SharedDatabase>,
}

impl MonocleApp {
    pub fn as2org_service(&self) -> As2orgService<'_> {
        As2orgService::new(&self.shared_db)
    }
}

// monocle-gui/src/views/as2org.rs
use monocle::services::as2org::{As2orgSearchArgs, SearchResult, OutputFormat};

struct As2orgView {
    args: As2orgSearchArgs,
    results: Vec<SearchResult>,
    is_loading: bool,
    error: Option<String>,
}

impl Render for As2orgView {
    fn render(&mut self, cx: &mut ViewContext<Self>) -> impl IntoElement {
        // GPUI rendering using gpui-component
        v_flex()
            .child(self.render_search_form(cx))
            .child(self.render_results_table(cx))
    }
}
```

---

## Success Criteria

1. **Library Usability**
   - [ ] Can import monocle as a library with minimal dependencies
   - [x] All public types are well-documented
   - [ ] Feature flags work correctly

2. **Database Consolidation**
   - [x] Single `SharedDatabase` entry point
   - [x] Automatic schema management
   - [x] Clean repository pattern

3. **Web Server**
   - [ ] WebSocket communication works
   - [ ] Progress streaming for long operations
   - [ ] Client disconnect detection

4. **GUI Ready**
   - [x] All args are serializable
   - [ ] No blocking in service methods
   - [ ] Progress callbacks available

---

## Progress Summary

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 1 | ✅ Complete | Database module created with core, session, and shared sub-modules |
| Phase 2 | ✅ Complete | Services module created with as2org, as2rel, and country services |
| Phase 3 | 🔲 Pending | Feature flags implementation |
| Phase 4 | 🔲 Pending | Web server with WebSocket support |
| Phase 5 | 🔲 Pending | Cleanup, update CLI, documentation |
