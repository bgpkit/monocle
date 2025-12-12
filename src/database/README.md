# Database Module

The database module provides all database functionality for monocle, organized into three sub-modules with dual-backend support.

## Architecture

```
database/
├── core/               # Foundation layer
│   ├── connection      # SQLite DatabaseConn wrapper (for exports)
│   ├── duckdb_conn     # DuckDB DuckDbConn wrapper (primary backend)
│   ├── duckdb_query    # DuckDB query helpers for prefix operations
│   ├── duckdb_schema   # DuckDB schema definitions and management
│   └── schema          # SQLite schema definitions and management
│
├── session/            # Temporary/session storage
│   └── msg_store       # BGP message storage for search results (SQLite export)
│
└── monocle/            # Main monocle database
    ├── as2org          # AS-to-Organization (SQLite - legacy)
    ├── duckdb_as2org   # AS-to-Organization (DuckDB - primary)
    ├── as2rel          # AS-level relationships (SQLite - legacy)
    ├── duckdb_as2rel   # AS-level relationships (DuckDB - primary)
    ├── rpki_cache      # RPKI ROA/ASPA cache (DuckDB)
    └── pfx2as_cache    # Prefix-to-AS cache (DuckDB)
```

## Database Backend Strategy

Monocle uses a **dual-database approach**:

- **DuckDB** is used as the primary internal database, providing:
  - Native INET type support for IP/prefix operations
  - Columnar storage for better compression
  - Efficient prefix containment queries (`<<=` and `>>=` operators)
  - In-memory and persistent operation modes

- **SQLite** is retained for export functionality:
  - Search results export (backward compatibility)
  - Tools that expect SQLite files can use exported data

## Module Overview

### Core (`core/`)

The foundation layer providing database connections, schema management, and query helpers.

#### SQLite Types (for exports)
- `DatabaseConn` - SQLite connection wrapper
- `SchemaManager` - SQLite schema management
- `SchemaStatus` - Schema state enumeration

#### DuckDB Types (primary backend)
- `DuckDbConn` - DuckDB connection wrapper with INET extension
- `DuckDbSchemaManager` - DuckDB schema management
- `DuckDbSchemaStatus` - DuckDB schema state enumeration

#### Query Helpers
- `PrefixQueryBuilder` - Build prefix containment queries
- `RpkiValidationQuery` - Build RPKI validation JOIN queries
- `Pfx2asQuery` - Build pfx2as lookup queries
- `build_prefix_containment_clause()` - Generate `<<=`/`>>=` clauses
- `order_by_prefix_length()` - Sort by prefix specificity

### Session (`session/`)

Temporary storage for one-time operations like BGP message search results.

- `MsgStore` - SQLite-backed storage for BGP elements (for export compatibility)

### Monocle Database (`monocle/`)

The main persistent database for monocle data.

#### SQLite (legacy/backward compatibility)
- `MonocleDatabase` - Legacy SQLite interface
- `As2orgRepository` - AS-to-Organization (SQLite)
- `As2relRepository` - AS-level relationships (SQLite)

#### DuckDB (primary)
- `DuckDbMonocleDatabase` - DuckDB interface
- `DuckDbAs2orgRepository` - AS-to-Organization (denormalized)
- `DuckDbAs2relRepository` - AS-level relationships

#### Cache Repositories
- `RpkiCacheRepository` - RPKI ROA/ASPA cache with TTL
- `Pfx2asCacheRepository` - Prefix-to-AS mappings cache

## Usage Examples

### DuckDB Connection

```rust
use monocle::database::{DuckDbConn, DuckDbSchemaManager};

// Create in-memory database
let conn = DuckDbConn::open_in_memory()?;

// Or open persistent database
let conn = DuckDbConn::open_path("/path/to/monocle.duckdb")?;

// Initialize schema
let manager = DuckDbSchemaManager::new(&conn);
manager.initialize()?;

// Set memory limit (default 2GB)
conn.set_memory_limit("4GB")?;
```

### Prefix Containment Queries

```rust
use monocle::database::{PrefixQueryBuilder, build_prefix_containment_clause};

// Build a query to find sub-prefixes of 10.0.0.0/8
let query = PrefixQueryBuilder::new("elems", "prefix")
    .include_sub("10.0.0.0/8")
    .with_condition("origin_asn = 13335")
    .order_by("timestamp DESC")
    .limit(100)
    .build();

// Or use the simple clause builder
let clause = build_prefix_containment_clause(
    "prefix",
    "10.0.0.0/8",
    true,   // include_sub
    false   // include_super
);
// Result: "prefix <<= '10.0.0.0/8'::INET"
```

### RPKI Cache

```rust
use monocle::database::{DuckDbConn, DuckDbSchemaManager, RpkiCacheRepository, RoaRecord};
use std::time::Duration;

let conn = DuckDbConn::open_in_memory()?;
DuckDbSchemaManager::new(&conn).initialize()?;

let cache = RpkiCacheRepository::new(&conn);

// Check if cache is fresh (within 1 hour TTL)
if !cache.is_cache_fresh("roa", "cloudflare", None, Duration::from_secs(3600)) {
    // Store new ROA data
    let roas = vec![
        RoaRecord {
            prefix: "1.0.0.0/24".to_string(),
            max_length: 24,
            origin_asn: 13335,
            ta: Some("ARIN".to_string()),
        },
    ];
    cache.store_roas("cloudflare", None, &roas)?;
}

// Query ROAs
let results = cache.query_roas_covering_prefix("1.0.0.100/32")?;
```

### Pfx2as Cache

```rust
use monocle::database::{DuckDbConn, DuckDbSchemaManager, Pfx2asCacheRepository, Pfx2asRecord};

let conn = DuckDbConn::open_in_memory()?;
DuckDbSchemaManager::new(&conn).initialize()?;

let cache = Pfx2asCacheRepository::new(&conn);

// Store prefix-to-AS mappings
let records = vec![
    Pfx2asRecord {
        prefix: "1.0.0.0/24".to_string(),
        origin_asns: vec![13335],
    },
];
cache.store("bgpkit", &records)?;

// Lookup by prefix (longest match)
let result = cache.lookup_longest_match("1.0.0.100/32")?;

// Lookup by origin ASN
let prefixes = cache.lookup_by_origin(13335)?;
```

## Schema Definitions

### DuckDB Tables

#### Meta Table
```sql
CREATE TABLE monocle_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMP DEFAULT current_timestamp
)
```

#### AS2Org (Denormalized)
```sql
CREATE TABLE as2org (
    asn INTEGER PRIMARY KEY,
    as_name TEXT NOT NULL,
    org_id TEXT NOT NULL,
    org_name TEXT NOT NULL,
    country TEXT NOT NULL,
    source TEXT NOT NULL
)
```

#### RPKI ROAs
```sql
CREATE TABLE rpki_roas (
    prefix INET NOT NULL,
    max_length INTEGER NOT NULL,
    origin_asn INTEGER NOT NULL,
    ta TEXT,
    cache_id INTEGER NOT NULL REFERENCES rpki_cache_meta(id)
)
```

#### Pfx2as
```sql
CREATE TABLE pfx2as (
    prefix INET NOT NULL,
    origin_asns INTEGER[] NOT NULL,
    cache_id INTEGER NOT NULL REFERENCES pfx2as_cache_meta(id)
)
```

#### BGP Elements (Internal)
```sql
CREATE TABLE elems (
    timestamp TIMESTAMP,
    elem_type TEXT,
    collector TEXT,
    peer_ip INET,
    peer_asn INTEGER,
    prefix INET,
    next_hop INET,
    as_path TEXT,
    origin_asn INTEGER,
    origin TEXT,
    local_pref INTEGER,
    med INTEGER,
    communities TEXT,
    atomic BOOLEAN,
    aggr_asn INTEGER,
    aggr_ip INET
)
```

## Prefix Containment Operators

DuckDB's inet extension supports these containment operators:

| Operator | Description | Example |
|----------|-------------|---------|
| `<<=` | Is contained by or equal | `'10.1.0.0/16'::INET <<= '10.0.0.0/8'::INET` → true |
| `>>=` | Contains or is equal to | `'10.0.0.0/8'::INET >>= '10.1.0.0/16'::INET` → true |

### Query Patterns

**Find sub-prefixes (more specific):**
```sql
SELECT * FROM elems WHERE prefix <<= '10.0.0.0/8'::INET
```

**Find super-prefixes (less specific):**
```sql
SELECT * FROM elems WHERE prefix >>= '10.1.1.0/24'::INET
```

**Find all related prefixes:**
```sql
SELECT * FROM elems WHERE prefix <<= '10.0.0.0/16'::INET OR prefix >>= '10.0.0.0/16'::INET
```

**RPKI validation via JOIN:**
```sql
SELECT e.*, 
    CASE
        WHEN EXISTS (
            SELECT 1 FROM rpki_roas r
            WHERE e.prefix <<= r.prefix
              AND e.origin_asn = r.origin_asn
              AND CAST(split_part(e.prefix::TEXT, '/', 2) AS INTEGER) <= r.max_length
        ) THEN 'valid'
        WHEN EXISTS (
            SELECT 1 FROM rpki_roas r WHERE e.prefix <<= r.prefix
        ) THEN 'invalid'
        ELSE 'unknown'
    END AS rpki_status
FROM elems e
```

## Cache TTL Configuration

Default TTL values:

| Cache Type | Default TTL | Constant |
|------------|-------------|----------|
| RPKI (current) | 1 hour | `DEFAULT_RPKI_CURRENT_TTL` |
| RPKI (historical) | Never expires | N/A |
| Pfx2as | 24 hours | `DEFAULT_PFX2AS_TTL` |

## DuckDB-Specific Notes

1. **INET Extension**: Automatically loaded on connection. Required for IP/prefix operations.

2. **No INET Indexes**: DuckDB doesn't support indexes on INET columns. Indexes are created on non-INET columns used for filtering (cache_id, origin_asn, etc.).

3. **Array Types**: DuckDB supports native arrays (`INTEGER[]`). When reading, arrays may need parsing from text format `[1, 2, 3]`.

4. **DateTime Handling**: DateTime values are stored as TIMESTAMP and may need text conversion for reading.

5. **Memory Limit**: Default 2GB, configurable via `conn.set_memory_limit("4GB")`.

## Testing

```bash
# Run all database tests
cargo test database::

# Run DuckDB-specific tests
cargo test duckdb

# Run cache tests
cargo test cache
```

In-memory databases are useful for testing:

```rust
#[test]
fn test_my_feature() {
    let conn = DuckDbConn::open_in_memory().unwrap();
    let manager = DuckDbSchemaManager::new(&conn);
    manager.initialize().unwrap();
    // Test with in-memory database
}
```

## Related Documentation

- [DuckDB Migration Plan](../../DUCKDB_MIGRATION_PLAN.md) - Full migration roadmap
- [Services Module](../services/README.md) - Business logic layer
- [Lens Module](../lens/README.md) - Data access patterns