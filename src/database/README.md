# Database Module

The database module provides all database functionality for monocle, organized into three sub-modules.

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
└── monocle/            # Main monocle database
    ├── as2org          # AS-to-Organization mappings (SQLite)
    ├── as2rel          # AS-level relationships (SQLite)
    └── file_cache      # File-based caches for RPKI and Pfx2as (JSON)
```

## Database Backend Strategy

Monocle uses **SQLite** for structured data that doesn't require INET operations:
- AS2Org mappings (AS-to-Organization)
- AS2Rel relationships (AS-level relationships)
- Search result exports

For data requiring **INET operations** (prefix matching, containment queries), monocle uses **file-based JSON caching**:
- RPKI ROA/ASPA data
- Pfx2as prefix mappings

This approach is used because SQLite doesn't natively support INET operations. The cached data is loaded into memory using trie data structures for efficient prefix lookups.

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
- `As2orgRepository` - AS-to-Organization queries
- `As2relRepository` - AS-level relationship queries

#### File-based Caches
- `RpkiFileCache` - RPKI ROA/ASPA cache with TTL support
- `Pfx2asFileCache` - Prefix-to-AS mappings cache with TTL support

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
let db = MonocleDatabase::open_in_dir("~/.monocle")?;

// Bootstrap AS2Org data if needed
if db.needs_as2org_bootstrap() {
    let (as_count, org_count) = db.bootstrap_as2org()?;
    println!("Loaded {} ASes, {} orgs", as_count, org_count);
}

// Search for AS information
let results = db.as2org().search_by_name("cloudflare")?;
for r in results {
    println!("AS{}: {} ({})", r.asn, r.as_name, r.org_name);
}

// Update AS2Rel data
if db.needs_as2rel_update() {
    let count = db.update_as2rel()?;
    println!("Loaded {} relationships", count);
}

// Query relationships
let rels = db.as2rel().search_asn(13335)?;
```

### RPKI File Cache

```rust
use monocle::database::{RpkiFileCache, RoaRecord, DEFAULT_RPKI_TTL};
use std::time::Duration;

let cache = RpkiFileCache::new("~/.monocle")?;

// Check if cache is fresh
if !cache.is_fresh("cloudflare", None, DEFAULT_RPKI_TTL) {
    // Store new ROA data
    let roas = vec![
        RoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: Some("ARIN".to_string()),
        },
    ];
    cache.store("cloudflare", None, roas, vec![])?;
}

// Load cached data
let data = cache.load("cloudflare", None)?;
println!("Loaded {} ROAs", data.roas.len());
```

### Pfx2as File Cache

```rust
use monocle::database::{Pfx2asFileCache, Pfx2asRecord, DEFAULT_PFX2AS_TTL};

let cache = Pfx2asFileCache::new("~/.monocle")?;

// Store prefix-to-AS mappings
let records = vec![
    Pfx2asRecord {
        prefix: "1.0.0.0/24".to_string(),
        origin_asns: vec![13335],
    },
];
cache.store("bgpkit", records)?;

// Load and build prefix map for lookups
let prefix_map = cache.build_prefix_map("bgpkit")?;
if let Some(asns) = prefix_map.get("1.0.0.0/24") {
    println!("Origin ASNs: {:?}", asns);
}
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

### AS2Org Tables
```sql
CREATE TABLE as2org_as (
    asn INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    org_id TEXT NOT NULL,
    source TEXT NOT NULL
)

CREATE TABLE as2org_org (
    org_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    country TEXT NOT NULL,
    source TEXT NOT NULL
)

-- Views for convenient queries
CREATE VIEW as2org_both AS ...
CREATE VIEW as2org_count AS ...
CREATE VIEW as2org_all AS ...
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

## File Cache Format

### RPKI Cache

Files are stored at: `{data_dir}/cache/rpki/rpki_{source}_{date}.json`

```json
{
  "meta": {
    "source": "cloudflare",
    "data_date": null,
    "cached_at": "2024-01-15T12:00:00Z",
    "roa_count": 500000,
    "aspa_count": 1000
  },
  "roas": [
    {
      "prefix": "1.0.0.0/24",
      "max_length": 24,
      "origin_asn": 13335,
      "ta": "ARIN"
    }
  ],
  "aspas": [
    {
      "customer_asn": 65001,
      "provider_asns": [13335, 15169]
    }
  ]
}
```

### Pfx2as Cache

Files are stored at: `{data_dir}/cache/pfx2as/pfx2as_{source}.json`

```json
{
  "meta": {
    "source": "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2",
    "cached_at": "2024-01-15T12:00:00Z",
    "record_count": 1000000
  },
  "records": [
    {
      "prefix": "1.0.0.0/24",
      "origin_asns": [13335]
    }
  ]
}
```

## Cache TTL Configuration

Default TTL values:

| Cache Type | Default TTL | Constant |
|------------|-------------|----------|
| RPKI (current) | 1 hour | `DEFAULT_RPKI_TTL` |
| RPKI (historical) | 7 days | `DEFAULT_RPKI_HISTORICAL_TTL` |
| Pfx2as | 24 hours | `DEFAULT_PFX2AS_TTL` |

## Testing

```bash
# Run all database tests
cargo test database::

# Run specific module tests
cargo test as2org
cargo test as2rel
cargo test file_cache
```

In-memory databases are useful for testing:

```rust
#[test]
fn test_my_feature() {
    let db = MonocleDatabase::open_in_memory().unwrap();
    // Test with in-memory database
}
```

For file cache tests, use `tempfile`:

```rust
#[test]
fn test_cache() {
    let temp_dir = tempfile::tempdir().unwrap();
    let cache = RpkiFileCache::new(temp_dir.path().to_str().unwrap()).unwrap();
    // Test with temporary cache directory
}
```

## Related Documentation

- [Architecture Overview](../../ARCHITECTURE.md) - System architecture
- [Lens Module](../lens/README.md) - Data access patterns