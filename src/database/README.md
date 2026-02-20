# Database Module

The database module provides all persistence and caching functionality for monocle, organized into three sub-modules.

## Architecture

```
database/
├── core/               # Foundation layer
│   ├── connection      # SQLite DatabaseConn wrapper
│   └── schema          # SQLite schema definitions and management
│
├── session/            # Temporary/session storage
│   └── msg_store       # BGP message storage for search results (SQLite)
│
└── monocle/            # Main monocle database (SQLite)
    ├── asinfo          # Unified AS information (from bgpkit-commons)
    ├── as2rel          # AS-level relationships
    ├── rpki            # RPKI ROA/ASPA data (blob-based prefix storage)
    └── pfx2as          # Prefix-to-ASN mappings (blob-based prefix storage)
```

## Database Backend Strategy

Monocle uses **SQLite** as its primary persistence layer:
- ASInfo mappings (unified AS information from bgpkit-commons)
- AS2Rel relationships (AS-level relationships)
- RPKI ROA/ASPA data (cached locally for fast queries and offline validation)
- Pfx2as prefix-to-ASN mappings (cached locally for fast lookups)
- Search result exports / session stores

**Blob-based prefix storage**: Both RPKI and Pfx2as store IP prefixes as 16-byte start/end address pairs (BLOBs).
IPv4 addresses are converted to IPv6-mapped format for uniform storage. This enables efficient range lookups
directly in SQLite without native INET/cidr operators.

## Module Overview

### Core (`core/`)

The foundation layer providing database connections and schema management.

- `DatabaseConn` - SQLite connection wrapper with configuration
- `SchemaManager` - SQLite schema management
- `SchemaStatus` - Schema state enumeration
- `SchemaDefinitions` - SQL table definitions

### Session (`session/`)

Temporary storage for one-time operations like BGP message search results.

- `MsgStore` - SQLite-backed storage for BGP elements

### Monocle Database (`monocle/`)

The main persistent database for monocle data.

#### SQLite Repositories
- `MonocleDatabase` - Main database interface
- `AsinfoRepository` - Unified AS information queries (from bgpkit-commons)
- `As2relRepository` - AS-level relationship queries
- `RpkiRepository` - RPKI ROA/ASPA tables + local validation (blob-based prefix storage)
- `Pfx2asRepository` - Prefix-to-ASN mappings (blob-based prefix storage)

## Usage Examples

### SQLite Connection

```rust
use monocle::database::{DatabaseConn, SchemaManager};

// Create in-memory database
let conn = DatabaseConn::open_in_memory()?;

// Or open persistent database
let conn = DatabaseConn::open_path("/path/to/monocle.sqlite3")?;

// Initialize schema
let manager = SchemaManager::new(&conn.conn);
manager.initialize()?;
```

### Monocle Database

```rust
use monocle::database::MonocleDatabase;

// Open the monocle database
let db = MonocleDatabase::open_in_dir("~/.local/share/monocle")?;

// Bootstrap ASInfo data if needed
if db.needs_asinfo_refresh(Duration::from_secs(7 * 24 * 60 * 60)) {
    let count = db.refresh_asinfo()?;
    println!("Loaded {} ASes", count);
}

// Search for AS information
let results = db.asinfo().search_by_name("cloudflare")?;
for r in results {
    println!("AS{}: {}", r.asn, r.name);
}

// Update AS2Rel data
use std::time::Duration;
let ttl = Duration::from_secs(24 * 60 * 60); // 24 hours
if db.needs_as2rel_refresh(ttl) {
    let count = db.update_as2rel()?;
    println!("Loaded {} relationships", count);
}

// Query relationships
let rels = db.as2rel().search_asn(13335)?;
```

### RPKI (SQLite Repository)

RPKI current data (ROAs and ASPAs) is stored in the monocle SQLite database and can be queried locally.

```rust
use monocle::database::{MonocleDatabase, RpkiRepository, DEFAULT_RPKI_CACHE_TTL};

let db = MonocleDatabase::open_in_dir("~/.local/share/monocle")?;
let rpki = db.rpki();

// Check metadata / whether refresh is needed
if rpki.needs_refresh(DEFAULT_RPKI_CACHE_TTL)? {
    // Typically refreshed via CLI (`monocle database refresh rpki`) or higher-level lens logic.
    // This example intentionally does not fetch from the network directly.
}

// Query ROAs by ASN
let roas = rpki.get_roas_by_asn(13335)?;
println!("Loaded {} ROAs for AS13335", roas.len());

// Validate prefix-ASN pair locally (RFC 6811-style)
let result = rpki.validate_detailed(13335, "1.1.1.0/24")?;
println!("{} {} -> {} ({})", result.prefix, result.asn, result.state, result.reason);
```

### Pfx2as Repository (SQLite)

```rust
use monocle::database::{MonocleDatabase, DEFAULT_PFX2AS_CACHE_TTL};

let db = MonocleDatabase::open_in_dir("~/.local/share/monocle")?;
let pfx2as = db.pfx2as();

// Check if refresh is needed
if pfx2as.needs_refresh(DEFAULT_PFX2AS_CACHE_TTL)? {
    // Refresh via CLI: monocle config update --pfx2as
    // Or via WebSocket: database.refresh with source: "pfx2as"
}

// Exact prefix match
let results = pfx2as.lookup_exact("1.1.1.0/24")?;

// Longest prefix match (most specific covering prefix)
let results = pfx2as.lookup_longest("1.1.1.1")?;

// Find all supernets (prefixes that cover the query)
let results = pfx2as.lookup_covering("1.1.1.0/24")?;

// Find all subnets (prefixes covered by the query)
let results = pfx2as.lookup_covered("1.0.0.0/8")?;

// Get all prefixes for an ASN
let results = pfx2as.get_prefixes_by_asn(13335)?;
```

## Schema Definitions

### Meta Table
```sql
CREATE TABLE monocle_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
)
```

### ASInfo Table
```sql
CREATE TABLE asinfo (
    asn INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    country TEXT,
    source TEXT NOT NULL
)
```

### AS2Rel Tables
```sql
CREATE TABLE as2rel_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    file_url TEXT NOT NULL,
    last_updated INTEGER NOT NULL,
    max_peers_count INTEGER NOT NULL DEFAULT 0
)

CREATE TABLE as2rel (
    asn1 INTEGER NOT NULL,
    asn2 INTEGER NOT NULL,
    paths_count INTEGER NOT NULL,
    peers_count INTEGER NOT NULL,
    rel INTEGER NOT NULL,
    PRIMARY KEY (asn1, asn2, rel)
)
```

### Pfx2as Table (Blob-based prefix storage)
```sql
CREATE TABLE pfx2as (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    prefix TEXT NOT NULL,
    prefix_len INTEGER NOT NULL,
    prefix_start BLOB NOT NULL,  -- 16-byte start address
    prefix_end BLOB NOT NULL,    -- 16-byte end address
    asn INTEGER NOT NULL
)

CREATE INDEX idx_pfx2as_range ON pfx2as(prefix_start, prefix_end);
CREATE INDEX idx_pfx2as_asn ON pfx2as(asn);
CREATE INDEX idx_pfx2as_prefix ON pfx2as(prefix);
```

### RPKI Tables (Blob-based prefix storage)
```sql
CREATE TABLE rpki_roas (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    prefix TEXT NOT NULL,
    prefix_len INTEGER NOT NULL,
    prefix_start BLOB NOT NULL,  -- 16-byte start address
    prefix_end BLOB NOT NULL,    -- 16-byte end address
    max_length INTEGER NOT NULL,
    origin_asn INTEGER NOT NULL,
    ta TEXT NOT NULL
)

CREATE TABLE rpki_aspas (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    customer_asn INTEGER NOT NULL,
    provider_asn INTEGER NOT NULL
)

CREATE INDEX idx_rpki_roas_range ON rpki_roas(prefix_start, prefix_end);
CREATE INDEX idx_rpki_roas_asn ON rpki_roas(origin_asn);
```

## Cache TTL Configuration

Default TTL values:

| Cache Type | Default TTL | Constant |
|------------|-------------|----------|
| ASInfo | 7 days | `DEFAULT_ASINFO_TTL` |
| AS2Rel | 7 days | `DEFAULT_AS2REL_TTL` |
| RPKI (SQLite) | 24 hours | `DEFAULT_RPKI_CACHE_TTL` |
| Pfx2as (SQLite) | 24 hours | `DEFAULT_PFX2AS_CACHE_TTL` |

## Testing

```bash
# Run all database tests
cargo test database::

# Run specific module tests
cargo test asinfo
cargo test as2rel
cargo test rpki
cargo test pfx2as
```

In-memory databases are useful for testing:

```rust
#[test]
fn test_my_feature() {
    let db = MonocleDatabase::open_in_memory().unwrap();
    // Test with in-memory database
}
```

For temporary directory tests, use `tempfile`:

```rust
#[test]
fn test_with_temp_db() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = MonocleDatabase::open_in_dir(temp_dir.path().to_str().unwrap()).unwrap();
    // Test with temporary database
}
```

## Related Documentation

- [Architecture Overview](../../ARCHITECTURE.md) - System architecture
- [Lens Module](../lens/README.md) - Lens patterns and conventions
- [DEVELOPMENT.md](../../DEVELOPMENT.md) - Contributor guide
- [Server README](../server/README.md) - WebSocket API (database.status, database.refresh)
