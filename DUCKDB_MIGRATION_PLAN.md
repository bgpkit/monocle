# DuckDB Migration Plan

This document outlines the plan to migrate monocle's database backend from SQLite to DuckDB, leveraging DuckDB's native INET type for IP/prefix operations and columnar storage for better compression and analytical query performance.

## Executive Summary

The migration to DuckDB provides three major benefits:
1. **Native INET type** with `<<=` and `>>=` operators for subnet containment checks
2. **Columnar storage with compression** - allows denormalization without storage penalty
3. **Caching opportunities** - persist RPKI data (ROAs, ASPAs) and Pfx2as data between sessions with freshness tracking

**Important**: The `search` command will continue to support SQLite export (default) for user-facing output files, while using DuckDB internally for the monocle database. This maintains compatibility with tools that expect SQLite files.

## Current Architecture Analysis

### Core Database Layer (`database/core/`)

**SQLite-Specific Code to Replace:**

| Component | Current (SQLite) | DuckDB Equivalent |
|-----------|------------------|-------------------|
| Connection | `rusqlite::Connection` | `duckdb::Connection` |
| WAL mode | `PRAGMA journal_mode=WAL` | Built-in (different mechanism) |
| Sync mode | `PRAGMA synchronous=NORMAL` | Not needed |
| Cache size | `PRAGMA cache_size=100000` | `SET memory_limit='100MB'` |
| Temp store | `PRAGMA temp_store=MEMORY` | Automatic |
| Foreign keys | `PRAGMA foreign_keys=ON` | Same syntax works |
| Table check | `sqlite_master` | `information_schema.tables` |
| Unix time | `strftime('%s', 'now')` | `epoch(current_timestamp)` |

### Data Storage Modules

| Module | Current Design | Issues |
|--------|----------------|--------|
| `as2org` | 3 normalized tables + 3 views | Over-normalized for read-heavy workload |
| `as2rel` | 2 tables (data + meta) | Good structure, minimal changes needed |
| `msg_store` | Single denormalized table, TEXT for IPs | Can't do subnet queries efficiently |
| RPKI data | In-memory only, discarded after use | Reloaded on every command, no caching |
| `pfx2as` | In-memory trie, discarded after use | Reloaded on every command, no caching |

## DuckDB Migration Benefits

### 1. Native INET Type Operations

DuckDB's inet extension provides native IP address handling:

```sql
-- Current: Must parse TEXT and use application logic for prefix matching
SELECT * FROM elems WHERE prefix LIKE '10.%';  -- Broken for subnet logic!

-- DuckDB: Native subnet containment operators
SELECT * FROM elems WHERE prefix <<= '10.0.0.0/8'::INET;  -- Sub-prefixes
SELECT * FROM elems WHERE prefix >>= '1.1.1.0/24'::INET;  -- Super-prefixes
```

**Impact on CLI:**
- `--include-super` and `--include-sub` can be implemented in SQL instead of application code
- RPKI validation can be done as SQL JOINs with containment predicates
- Prefix search becomes a simple `WHERE` clause

### 2. Denormalization Opportunities

DuckDB's columnar compression means denormalized tables compress better than normalized ones:

**as2org: 3 tables → 1 table**
```sql
-- Current: 3 tables with JOINs
CREATE TABLE as2org_as (asn INTEGER PRIMARY KEY, name TEXT, org_id TEXT, source TEXT);
CREATE TABLE as2org_org (org_id TEXT PRIMARY KEY, name TEXT, country TEXT, source TEXT);
-- Plus 3 views for JOINs

-- DuckDB: Single denormalized table
CREATE TABLE as2org (
    asn INTEGER PRIMARY KEY,
    as_name TEXT,
    org_id TEXT,
    org_name TEXT,
    country TEXT,
    source TEXT
);
-- Compression handles the redundancy efficiently
```

**BGP messages with INET types:**
```sql
CREATE TABLE elems (
    timestamp TIMESTAMP,
    elem_type TEXT,  -- 'A' or 'W'
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
);
```

### 3. RPKI Data Caching (New Feature)

Currently, RPKI data is loaded into memory and discarded after each command. With DuckDB, we can cache it:

**ROA Cache Table:**
```sql
CREATE TABLE rpki_roas (
    prefix INET NOT NULL,
    max_length INTEGER NOT NULL,
    origin_asn INTEGER NOT NULL,
    ta TEXT,  -- Trust Anchor (RIR)
    cache_id INTEGER NOT NULL REFERENCES rpki_cache_meta(id)
);

-- Index for containment queries
CREATE INDEX idx_rpki_roas_prefix ON rpki_roas(prefix);
CREATE INDEX idx_rpki_roas_cache ON rpki_roas(cache_id);
```

**ASPA Cache Table:**
```sql
CREATE TABLE rpki_aspas (
    customer_asn INTEGER NOT NULL,
    provider_asns INTEGER[] NOT NULL,  -- Array of provider ASNs
    cache_id INTEGER NOT NULL REFERENCES rpki_cache_meta(id)
);

CREATE INDEX idx_rpki_aspas_customer ON rpki_aspas(customer_asn);
CREATE INDEX idx_rpki_aspas_cache ON rpki_aspas(cache_id);
```

**Cache Metadata:**
```sql
CREATE TABLE rpki_cache_meta (
    id INTEGER PRIMARY KEY,
    data_type TEXT NOT NULL,     -- 'roas' or 'aspas'
    data_source TEXT NOT NULL,   -- 'cloudflare', 'ripe', 'rpkiviews:soborost'
    data_date DATE,              -- NULL for 'current'
    loaded_at TIMESTAMP NOT NULL DEFAULT current_timestamp,
    record_count INTEGER NOT NULL,
    UNIQUE (data_type, data_source, data_date)
);
```

**Note on ASPA provider_asns array**: Using `INTEGER[]` array type for provider ASNs:
- Simplifies the data model (no need for join table)
- DuckDB has excellent array support with functions like `list_contains()`, `unnest()`
- Query example: `SELECT * FROM rpki_aspas WHERE list_contains(provider_asns, 13335)`

### 4. Pfx2as Data Caching (New Feature)

Currently, pfx2as data is loaded from BGPKIT's dataset into an in-memory trie and discarded after each command. With DuckDB, we can cache it with INET type support:

**Pfx2as Cache Table:**
```sql
CREATE TABLE pfx2as (
    prefix INET NOT NULL,
    origin_asns INTEGER[] NOT NULL,  -- Array of origin ASNs (MOAS support)
    cache_id INTEGER NOT NULL REFERENCES pfx2as_cache_meta(id)
);

-- Index for containment queries (longest prefix match via >>=)
CREATE INDEX idx_pfx2as_prefix ON pfx2as(prefix);
CREATE INDEX idx_pfx2as_cache ON pfx2as(cache_id);
```

**Pfx2as Cache Metadata:**
```sql
CREATE TABLE pfx2as_cache_meta (
    id INTEGER PRIMARY KEY,
    data_source TEXT NOT NULL,   -- URL or file path
    loaded_at TIMESTAMP NOT NULL DEFAULT current_timestamp,
    record_count INTEGER NOT NULL
);
```

**Key Benefits:**
- **INET type for prefixes**: Native prefix storage and comparison
- **Longest prefix match via SQL**: `WHERE prefix >>= '1.1.1.1/32'::INET ORDER BY masklen(prefix) DESC LIMIT 1`
- **Exact match**: `WHERE prefix = '1.1.1.0/24'::INET`
- **MOAS support**: `origin_asns INTEGER[]` handles multiple origin ASNs per prefix
- **Cross-table joins**: Annotate BGP messages with origin AS info directly in SQL

**Query Examples:**
```sql
-- Longest prefix match for an IP
SELECT prefix, origin_asns 
FROM pfx2as 
WHERE prefix >>= '1.1.1.1/32'::INET
ORDER BY masklen(prefix) DESC 
LIMIT 1;

-- Exact prefix match
SELECT origin_asns FROM pfx2as WHERE prefix = '1.1.1.0/24'::INET;

-- Find all prefixes originated by an ASN
SELECT prefix FROM pfx2as WHERE list_contains(origin_asns, 13335);

-- Annotate BGP messages with pfx2as data
SELECT 
    e.prefix,
    e.origin_asn as announced_origin,
    p.origin_asns as expected_origins,
    list_contains(p.origin_asns, e.origin_asn) as origin_matches
FROM elems e
LEFT JOIN pfx2as p ON p.prefix >>= e.prefix
WHERE p.cache_id = (SELECT MAX(id) FROM pfx2as_cache_meta);
```

**Cache Usage Flow:**
1. Check if cached data exists and is fresh (configurable TTL)
2. If fresh, query from cache table
3. If stale/missing, load from source, store in cache, then query
4. Historical data is never refreshed (permanent cache)

**CLI Integration:**
```bash
# Use cached data if available, otherwise load fresh
monocle rpki roas --origin 13335

# Force refresh the cache
monocle rpki roas --origin 13335 --refresh-cache

# Show cache status
monocle rpki cache-status
```

### 4. Advanced Query Capabilities

With INET type and cached RPKI data, we can do powerful queries:

**RPKI Validation in SQL:**
```sql
-- Validate BGP announcements against ROA cache
SELECT 
    e.prefix,
    e.origin_asn as announced_origin,
    r.origin_asn as roa_origin,
    r.max_length,
    CASE 
        WHEN r.prefix IS NULL THEN 'not-found'
        WHEN e.origin_asn = r.origin_asn 
             AND masklen(e.prefix) <= r.max_length THEN 'valid'
        ELSE 'invalid'
    END as rpki_status
FROM elems e
LEFT JOIN rpki_roas r ON r.prefix >>= e.prefix  -- ROA covers the announced prefix
WHERE r.data_source = 'cloudflare' AND r.data_date IS NULL;
```

**Annotated BGP Data with AS Names:**
```sql
-- Join BGP messages with AS2Org data
SELECT 
    e.timestamp,
    e.prefix,
    e.origin_asn,
    a.org_name as origin_org,
    e.as_path,
    e.collector
FROM elems e
LEFT JOIN as2org a ON e.origin_asn = a.asn
WHERE e.prefix <<= '1.0.0.0/8'::INET
ORDER BY e.timestamp;
```

**ASPA queries with provider array:**
```sql
-- Find all ASPAs where a specific ASN is a provider
SELECT customer_asn, provider_asns 
FROM rpki_aspas 
WHERE list_contains(provider_asns, 13335);

-- Unnest providers for detailed analysis
SELECT customer_asn, unnest(provider_asns) as provider_asn
FROM rpki_aspas;

-- Count how many customers each provider has
SELECT provider_asn, COUNT(*) as customer_count
FROM (SELECT unnest(provider_asns) as provider_asn FROM rpki_aspas)
GROUP BY provider_asn
ORDER BY customer_count DESC;
```

**Prefix Containment Queries:**
```sql
-- Find all announcements for sub-prefixes of a given prefix
SELECT * FROM elems 
WHERE prefix <<= '10.0.0.0/8'::INET;  -- /9, /10, ... /32

-- Find announcements covered by a super-prefix
SELECT * FROM elems 
WHERE prefix >>= '10.1.0.0/24'::INET;  -- /24, /23, ... /8
```

## Schema Design

### Complete DuckDB Schema

```sql
-- =============================================================================
-- Core Metadata
-- =============================================================================
CREATE TABLE monocle_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMP DEFAULT current_timestamp
);

-- =============================================================================
-- AS2Org Data (Denormalized)
-- =============================================================================
CREATE TABLE as2org (
    asn INTEGER PRIMARY KEY,
    as_name TEXT NOT NULL,
    org_id TEXT NOT NULL,
    org_name TEXT NOT NULL,
    country TEXT NOT NULL,
    source TEXT NOT NULL
);

CREATE INDEX idx_as2org_org_name ON as2org(org_name);
CREATE INDEX idx_as2org_country ON as2org(country);

-- =============================================================================
-- AS2Rel Data
-- =============================================================================
CREATE TABLE as2rel_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    file_url TEXT NOT NULL,
    last_updated TIMESTAMP NOT NULL,
    max_peers_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE as2rel (
    asn1 INTEGER NOT NULL,
    asn2 INTEGER NOT NULL,
    paths_count INTEGER NOT NULL,
    peers_count INTEGER NOT NULL,
    rel INTEGER NOT NULL,  -- -1: asn1 is customer, 0: peers, 1: asn1 is provider
    PRIMARY KEY (asn1, asn2, rel)
);

CREATE INDEX idx_as2rel_asn1 ON as2rel(asn1);
CREATE INDEX idx_as2rel_asn2 ON as2rel(asn2);

-- =============================================================================
-- RPKI Cache
-- =============================================================================
CREATE TABLE rpki_cache_meta (
    id INTEGER PRIMARY KEY,
    data_type TEXT NOT NULL,     -- 'roas' or 'aspas'
    data_source TEXT NOT NULL,   -- 'cloudflare', 'ripe', 'rpkiviews:soborost'
    data_date DATE,              -- NULL = current
    loaded_at TIMESTAMP NOT NULL DEFAULT current_timestamp,
    record_count INTEGER NOT NULL,
    UNIQUE (data_type, data_source, data_date)
);

CREATE TABLE rpki_roas (
    prefix INET NOT NULL,
    max_length INTEGER NOT NULL,
    origin_asn INTEGER NOT NULL,
    ta TEXT,
    cache_id INTEGER NOT NULL REFERENCES rpki_cache_meta(id)
);

CREATE INDEX idx_rpki_roas_prefix ON rpki_roas(prefix);
CREATE INDEX idx_rpki_roas_origin ON rpki_roas(origin_asn);
CREATE INDEX idx_rpki_roas_cache ON rpki_roas(cache_id);

CREATE TABLE rpki_aspas (
    customer_asn INTEGER NOT NULL,
    provider_asns INTEGER[] NOT NULL,  -- Array of provider ASNs
    cache_id INTEGER NOT NULL REFERENCES rpki_cache_meta(id)
);

CREATE INDEX idx_rpki_aspas_customer ON rpki_aspas(customer_asn);
CREATE INDEX idx_rpki_aspas_cache ON rpki_aspas(cache_id);

-- =============================================================================
-- Pfx2as Cache
-- =============================================================================
CREATE TABLE pfx2as_cache_meta (
    id INTEGER PRIMARY KEY,
    data_source TEXT NOT NULL,   -- URL or file path
    loaded_at TIMESTAMP NOT NULL DEFAULT current_timestamp,
    record_count INTEGER NOT NULL
);

CREATE TABLE pfx2as (
    prefix INET NOT NULL,
    origin_asns INTEGER[] NOT NULL,  -- Array for MOAS support
    cache_id INTEGER NOT NULL REFERENCES pfx2as_cache_meta(id)
);

CREATE INDEX idx_pfx2as_prefix ON pfx2as(prefix);
CREATE INDEX idx_pfx2as_cache ON pfx2as(cache_id);

-- =============================================================================
-- BGP Message Storage (Session) - Internal DuckDB storage
-- Note: The search command exports to SQLite by default for user output
-- =============================================================================
CREATE TABLE elems (
    timestamp TIMESTAMP,
    elem_type TEXT,  -- 'A' (announce) or 'W' (withdraw)
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
);

CREATE INDEX idx_elems_timestamp ON elems(timestamp);
CREATE INDEX idx_elems_prefix ON elems(prefix);
CREATE INDEX idx_elems_peer_asn ON elems(peer_asn);
CREATE INDEX idx_elems_collector ON elems(collector);
CREATE INDEX idx_elems_elem_type ON elems(elem_type);
CREATE INDEX idx_elems_origin_asn ON elems(origin_asn);
```

## Migration Phases

### Phase 1: Core Infrastructure (Week 1)

**Tasks:**
1. Add `duckdb` crate dependency, keep `rusqlite` temporarily for parallel testing
2. Create `DuckDbConn` wrapper in `database/core/`
3. Implement DuckDB-specific configuration (memory limits, etc.)
4. Update schema management for DuckDB syntax
5. Create feature flag to switch between SQLite and DuckDB

**Files to Modify:**
- `Cargo.toml` - Add duckdb dependency
- `database/core/connection.rs` - Add DuckDB connection wrapper
- `database/core/schema.rs` - Update schema DDL for DuckDB
- `database/core/mod.rs` - Export new types

**Testing:**
- Unit tests for connection and schema management
- Verify in-memory database works
- Verify file-based database works

### Phase 2: Schema Migration (Week 2)

**Tasks:**
1. Convert as2org to denormalized schema
2. Update as2rel schema (minimal changes)
3. Update msg_store schema with INET types
4. Remove SQLite views (no longer needed)
5. Update all repository methods

**Files to Modify:**
- `database/monocle/as2org.rs` - Rewrite for denormalized schema
- `database/monocle/as2rel.rs` - Minor updates
- `database/session/msg_store.rs` - Use INET types internally, keep SQLite export

**Testing:**
- Data integrity tests (load data, verify queries)
- Performance comparison vs SQLite
- INET type operations work correctly
- SQLite export produces valid files

### Phase 3: RPKI and Pfx2as Caching (Week 3)

**Tasks:**
1. Create `database/monocle/rpki_cache.rs` module
2. Implement ROA cache repository
3. Implement ASPA cache repository
4. Create `database/monocle/pfx2as_cache.rs` module
5. Implement Pfx2as cache repository with INET-based queries
6. Add cache metadata management for all cache types
7. Update RpkiLens and Pfx2asLens to use cache

**New Files:**
- `database/monocle/rpki_cache.rs` - ROA/ASPA cache repositories
- `database/monocle/pfx2as_cache.rs` - Pfx2as cache repository

**Files to Modify:**
- `database/monocle/mod.rs` - Export new types
- `database/core/schema.rs` - Add RPKI and Pfx2as cache tables
- `lens/rpki/mod.rs` - Integrate cache
- `lens/pfx2as/mod.rs` - Integrate cache, use SQL for lookups
- `bin/commands/rpki.rs` - Add cache CLI options
- `bin/commands/pfx2as.rs` - Add cache CLI options

**Testing:**
- Cache hit/miss behavior
- Cache freshness checking
- Historical data caching (never expires)
- Pfx2as longest prefix match via SQL
- Pfx2as exact match via SQL

### Phase 4: Query Enhancements (Week 4)

**Tasks:**
1. Implement prefix containment queries using `<<=` and `>>=`
2. Update search filters to use SQL-based prefix matching
3. Add RPKI validation as JOIN queries
4. Optimize query patterns for DuckDB columnar engine

**Files to Modify:**
- `lens/search/mod.rs` - SQL-based prefix filtering
- `lens/rpki/mod.rs` - SQL-based RPKI validation
- `bin/commands/search.rs` - Update `--include-super/sub` behavior

**Testing:**
- Prefix containment queries return correct results
- Performance benchmarks for large datasets
- RPKI validation accuracy

### Phase 5: Cleanup and Optimization (Week 5)

**Tasks:**
1. Remove SQLite dependency completely
2. Remove feature flags (DuckDB only)
3. Update documentation
4. Performance tuning
5. Final testing and validation

**Files to Remove:**
- SQLite-specific code paths
- Unused views and normalized table schemas

**Documentation:**
- Update README with new features
- Update CHANGELOG
- Update database module README

## Dependency Changes

### Cargo.toml Changes

```toml
[dependencies]
# Keep for SQLite export functionality
rusqlite = { version = "0.37", features = ["bundled"] }

# Add for internal database
duckdb = { version = "1.1", features = ["bundled"] }
```

**Note**: We retain `rusqlite` because the `search` command needs to export results to SQLite files (the default and most common use case for sharing search results).

## CLI Changes

### New/Modified Commands

```bash
# RPKI commands with caching
monocle rpki roas --origin 13335              # Uses cache if fresh
monocle rpki roas --origin 13335 --refresh    # Force refresh cache
monocle rpki roas --origin 13335 --no-cache   # Skip cache entirely

monocle rpki aspas --customer 13335           # Uses cache if fresh
# Query by provider using array containment
monocle rpki aspas --provider 13335           # Finds all ASPAs with this provider

monocle rpki cache-status                     # Show cache freshness
monocle rpki cache-clear                      # Clear all cached data

# Pfx2as commands with caching
monocle pfx2as 1.1.1.0/24                     # Uses cache if fresh
monocle pfx2as 1.1.1.0/24 --refresh           # Force refresh cache
monocle pfx2as 1.1.1.0/24 --exact             # Exact match only
monocle pfx2as --origin 13335                 # Find all prefixes by origin ASN (new!)

monocle pfx2as cache-status                   # Show cache freshness
monocle pfx2as cache-clear                    # Clear cached data

# Search with INET-powered prefix matching
monocle search --prefix 10.0.0.0/8 --include-sub   # All sub-prefixes via <<=
monocle search --prefix 1.1.1.0/24 --include-super # All super-prefixes via >>=

# Search output formats (SQLite is default for compatibility)
monocle search --sqlite-path results.db       # Export to SQLite (default format)
monocle search --parquet-path results.parquet # Export to Parquet (new option)
monocle search --duckdb-path results.duckdb   # Export to DuckDB (new option)

# RPKI validation using cached data + SQL joins
monocle search --prefix 1.0.0.0/8 --validate-rpki  # Adds RPKI status column

# Origin validation using pfx2as cached data
monocle search --prefix 1.0.0.0/8 --validate-origin  # Checks origin against pfx2as
```

### Configuration

New config options in `~/.monocle.toml`:

```toml
[cache]
# Cache TTL for "current" RPKI data (default: 1 hour)
rpki_current_ttl = "1h"

# Database path (default: ~/.monocle/monocle-data.duckdb)
database_path = "~/.monocle/monocle-data.duckdb"

# Memory limit for DuckDB (default: auto)
memory_limit = "2GB"
```

## Risk Mitigation

### Potential Risks

1. **Data Migration**: Users have existing SQLite databases
   - **Mitigation**: Provide one-time migration script, or just regenerate (data is from external sources)

2. **DuckDB Crate Stability**: duckdb-rs is relatively new
   - **Mitigation**: Pin to stable version, comprehensive testing

3. **INET Extension Availability**: Extension must be loaded
   - **Mitigation**: Use `duckdb` crate with bundled extensions, auto-install on first use

4. **File Size Changes**: DuckDB files may be different size than SQLite
   - **Mitigation**: Benchmark storage requirements, document changes

### Rollback Plan

If critical issues are discovered:
1. Keep rusqlite as optional dependency during transition
2. Feature flag to revert to SQLite backend
3. Data regeneration from sources (no data loss risk)

## Success Metrics

1. **Query Performance**: Prefix containment queries < 100ms for 1M records
2. **Storage Efficiency**: Similar or smaller file size vs SQLite (due to compression)
3. **Cache Hit Rate**: > 90% for repeated RPKI queries within TTL
4. **API Compatibility**: All existing CLI commands work unchanged
5. **New Capabilities**: SQL-based RPKI validation, prefix containment queries

## Timeline Summary

| Phase | Duration | Deliverable |
|-------|----------|-------------|
| 1. Core Infrastructure | Week 1 | DuckDB connection wrapper, schema management |
| 2. Schema Migration | Week 2 | All tables migrated to DuckDB with INET types |
| 3. RPKI Caching | Week 3 | ROA/ASPA cache with freshness tracking |
| 4. Query Enhancements | Week 4 | SQL-based prefix matching, RPKI validation |
| 5. Cleanup | Week 5 | Remove SQLite, documentation, final testing |

**Total Estimated Time**: 5 weeks