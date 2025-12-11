# Database Module

The database module provides all database functionality for monocle, organized into three sub-modules:

## Architecture

```
database/
├── core/           # Foundation layer
│   ├── connection  # DatabaseConn connection wrapper
│   └── schema      # Schema definitions, versioning, and migrations
│
├── session/        # Temporary/session storage
│   └── msg_store   # BGP message storage for search results
│
└── monocle/        # Main monocle database
    ├── mod         # MonocleDatabase unified interface
    ├── as2org      # AS-to-Organization repository
    └── as2rel      # AS-level relationships repository
```

## Module Overview

### Core (`core/`)

The foundation layer providing database connections and schema management.

**Key Types:**
- `DatabaseConn` - SQLite connection wrapper with common operations
- `SchemaManager` - Handles schema initialization, versioning, and migrations
- `SchemaStatus` - Enum representing current schema state (Current, NeedsMigration, etc.)
- `SCHEMA_VERSION` - Current schema version constant

**Usage:**
```rust
use monocle::database::{DatabaseConn, SchemaManager, SchemaStatus};

// Direct connection (for custom use cases)
let db = DatabaseConn::open_path("/path/to/db.sqlite3")?;

// Check schema status
let schema = SchemaManager::new(&db.conn);
match schema.check_status()? {
    SchemaStatus::Current => println!("Schema up to date"),
    SchemaStatus::NeedsMigration { from, to } => {
        println!("Migration needed: v{} -> v{}", from, to);
    }
    _ => {}
}
```

### Session (`session/`)

Temporary storage for one-time operations like BGP message search results.

**Key Types:**
- `MsgStore` - SQLite-backed storage for BGP elements during search operations

**Usage:**
```rust
use monocle::database::MsgStore;

// Create in-memory store for a search session
let store = MsgStore::new(None, false)?;

// Or create persistent store
let store = MsgStore::new(Some("/tmp/search.db"), false)?;

// Insert BGP elements
store.insert_elems(&bgp_elements)?;

// Query stored messages
let count = store.count()?;
```

### Monocle Database (`monocle/`)

The main persistent database for monocle data that can be shared across sessions.

**Key Types:**
- `MonocleDatabase` - Unified interface to all monocle data tables
- `As2orgRepository` - AS-to-Organization data access
- `As2relRepository` - AS-level relationships data access
- `As2orgRecord`, `As2relRecord` - Data record types

**Usage:**
```rust
use monocle::MonocleDatabase;

// Open monocle database (creates if needed, handles migrations)
let db = MonocleDatabase::open_in_dir("~/.monocle")?;

// Or open at specific path
let db = MonocleDatabase::open("/path/to/monocle-data.sqlite3")?;

// Access repositories
let as2org = db.as2org();
let as2rel = db.as2rel();

// Bootstrap data if needed
if db.needs_as2org_bootstrap() {
    db.bootstrap_as2org()?;
}

// Query data
let results = as2org.search_by_name("cloudflare")?;
let org_name = as2org.lookup_org_name(13335);
```

## Data Flow

```
External Sources                    Database                    Services
─────────────────                   ────────                    ────────
bgpkit-commons      ──────────►   MonocleDatabase   ◄──────►   As2orgService
  (AS2Org data)                     └── as2org                As2relService
                                    └── as2rel
BGPKIT AS2Rel URL   ──────────►        │
                                       │
MRT Files           ──────────►   MsgStore (session)  ◄──►   Search operations
```

## Schema Management

The database module uses a simple versioning strategy:

1. **Version Tracking**: Schema version stored in `monocle_meta` table
2. **Status Detection**: On open, schema status is checked
3. **Auto-Recovery**: For monocle data (regeneratable), schema issues trigger reset + reinitialization

```rust
pub enum SchemaStatus {
    NotInitialized,     // Fresh database
    Current,            // Version matches
    NeedsMigration { from, to },  // Version behind
    Incompatible { database_version, required_version },  // Major version mismatch
    Corrupted,          // Schema verification failed
}
```

## Interaction with Services

The database module is a **low-level data access layer**. For business logic and output formatting, use the services module:

```rust
use monocle::MonocleDatabase;
use monocle::services::{As2orgService, As2orgSearchArgs};

// Database handles storage
let db = MonocleDatabase::open_in_dir("~/.monocle")?;

// Service handles business logic
let service = As2orgService::new(&db);
let args = As2orgSearchArgs::new("cloudflare");
let results = service.search(&args)?;

// Service formats output
let output = service.format_results(&results, &format, false);
```

## Adding New Data Types

To add a new data type (e.g., RPKI ROAs):

1. **Create repository** in `monocle/`:
   ```rust
   // monocle/rpki_roas.rs
   pub struct RpkiRoasRepository<'a> { conn: &'a Connection }
   impl RpkiRoasRepository<'_> {
       pub fn insert_roas(&self, roas: &[Roa]) -> Result<()> { ... }
       pub fn lookup_prefix(&self, prefix: IpNet) -> Result<Vec<Roa>> { ... }
   }
   ```

2. **Update schema** in `core/schema.rs`:
   - Add table creation SQL
   - Increment `SCHEMA_VERSION`

3. **Expose via MonocleDatabase** in `monocle/mod.rs`:
   ```rust
   impl MonocleDatabase {
       pub fn rpki_roas(&self) -> RpkiRoasRepository<'_> {
           RpkiRoasRepository::new(&self.db.conn)
       }
   }
   ```

4. **Create service** in `services/` for business logic (see services README)

## Testing

All modules have unit tests. Run with:

```bash
cargo test database::
```

The `MonocleDatabase::open_in_memory()` method is useful for testing:

```rust
#[test]
fn test_my_feature() {
    let db = MonocleDatabase::open_in_memory().unwrap();
    // Test with in-memory database
}
```

## Related Documentation

- [Services Module](../services/README.md) - Business logic layer
- [ARCHITECTURE.md](../../ARCHITECTURE.md) - Overall system design
- [REVISION_PLAN.md](../../REVISION_PLAN.md) - Migration roadmap