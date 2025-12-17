# Refactor TODO

This document tracks the architectural refactoring work and new feature additions for monocle.

---

## Part 1: Architecture Consistency Refactoring

### Background

The current separation between `database/` (persistence) and `lens/` (use-cases) is conceptually sound, but the implementation has drifted:
- Some lenses use database, some don't
- Some business logic is in database repositories
- Some CLI commands implement logic that should be in lenses

### Goals

1. **Enforce consistency**: All lenses that need persistence go through `database/` repositories
2. **Clear separation**: Database = data access only; Lens = business logic
3. **Thin CLI**: CLI commands become thin wrappers around lenses

### Layering Rules

To prevent logic from leaking between layers, follow these explicit boundaries:

#### Repository Layer (`database/`)
- **Does**:
  - Parsing and storing data from external sources
  - SQL operations (CRUD, indexing, filtering primitives)
  - Schema management and migrations
  - Caching metadata (timestamps, ETags, source URLs)
  - Data retrieval with basic filtering
- **Does NOT**:
  - Policy decisions (e.g., "is this valid?")
  - CLI/output formatting
  - Query type detection or semantic interpretation
  - Cross-repository orchestration

#### Lens Layer (`lens/`)
- **Does**:
  - Combine multiple repositories as needed
  - Apply policy and interpretation (e.g., validation states)
  - Implement query semantics and section selection
  - Return domain-typed results
  - Coordinate refresh/bootstrap across data sources
- **Does NOT**:
  - Direct SQL operations
  - Own caching/refresh metadata (delegates to repositories)
  - CLI argument parsing (that's in `bin/`)

#### CLI Layer (`bin/`)
- **Does**:
  - Parse CLI arguments
  - Detect terminal width
  - Call lens methods
  - Format output for display
- **Does NOT**:
  - Implement business logic
  - Direct database access

---

### Tasks

#### 1.1 Complete `Pfx2asLens` Implementation

**Priority**: High  
**Status**: Not Started

Move logic from `src/bin/commands/pfx2as.rs` into `Pfx2asLens`:

**Repository methods (already exist, verify completeness)**:
- `lookup_exact(prefix) -> Vec<u32>` - Exact prefix match
- `lookup_longest(prefix) -> Pfx2asQueryResult` - Longest prefix match
- `lookup_covering(prefix) -> Vec<Pfx2asQueryResult>` - All supernets
- `lookup_covered(prefix) -> Vec<Pfx2asQueryResult>` - All subnets
- `get_by_asn(asn) -> Vec<Pfx2asDbRecord>` - All prefixes for an ASN

**Lens methods to implement**:
- [ ] Create `Pfx2asLens` struct that takes `&MonocleDatabase`
- [ ] `lookup(&self, prefix: &str, mode: LookupMode) -> Result<Pfx2asResult>`
- [ ] `get_prefixes_for_asn(&self, asn: u32) -> Result<PrefixList>`
- [ ] `needs_refresh(&self) -> bool`
- [ ] `refresh(&self, url: Option<&str>) -> Result<usize>`
- [ ] Formatting methods for table/JSON output
- [ ] Refactor CLI command to use the lens

**Query semantics**:
- `LookupMode::Exact` - Only exact prefix match
- `LookupMode::Longest` - Longest prefix match (default)
- `LookupMode::Covering` - All covering prefixes (supernets)
- `LookupMode::Covered` - All covered prefixes (subnets)

#### 1.2 Refactor `RpkiLens` to Use `RpkiRepository`

**Priority**: High  
**Status**: Not Started

Currently `RpkiLens` loads data directly from bgpkit-commons, bypassing the database cache.

- [ ] Make `RpkiLens` take `&MonocleDatabase` for current data operations
- [ ] Use `RpkiRepository` for caching current ROA/ASPA data
- [ ] Keep direct bgpkit-commons loading only for historical data (with date parameter)
- [ ] Add `refresh_cache(&self) -> Result<()>` method
- [ ] Refactor CLI command to use unified lens interface

#### 1.3 Move Validation Logic from `RpkiRepository` to `RpkiLens`

**Priority**: Medium  
**Status**: Not Started

Database repositories should not contain policy logic like validation.

**Keep in Repository** (data access):
- `get_covering_roas(prefix) -> Vec<RoaRecord>`
- `get_roas_by_asn(asn) -> Vec<RoaRecord>`
- `get_aspa(asn) -> Option<AspaRecord>`

**Move to Lens** (policy/interpretation):
- [ ] `validate(prefix, asn) -> ValidationState` 
- [ ] `validate_detailed(prefix, asn) -> ValidationResult`
- [ ] Update CLI commands to use lens for validation

#### 1.4 Restructure Database Module

**Priority**: Low  
**Status**: Not Started

Simplify the deep nesting while maintaining discoverability.

**Current structure**:
```
database/
├── core/
│   ├── connection.rs
│   └── schema.rs
├── monocle/
│   ├── as2org.rs
│   ├── as2rel.rs
│   ├── rpki.rs
│   └── pfx2as.rs
└── session/
    └── msg_store.rs
```

**Target structure**:
```
database/
├── mod.rs              # Public exports
├── connection.rs       # SQLite connection management
├── schema.rs           # Schema definitions and migrations
├── repositories/       # Data access layer
│   ├── mod.rs
│   ├── asinspect.rs       # New (replaces as2org)
│   ├── as2rel.rs
│   ├── rpki.rs
│   └── pfx2as.rs
└── session.rs          # Temporary storage (MsgStore)
```

- [ ] Create `repositories/` subdirectory
- [ ] Move repository files
- [ ] Update all imports
- [ ] Remove empty directories
- [ ] Update documentation

#### 1.5 Update Architecture Documentation

**Priority**: Low  
**Status**: Not Started

- [ ] Update `ARCHITECTURE.md` to reflect actual patterns
- [ ] Add layering rules section
- [ ] Update `src/database/README.md`
- [ ] Update `src/lens/README.md`
- [ ] Add examples showing correct lens-database interaction

---

## Part 2: Unified `inspect` Command & New Data Modules

### Background

The current command structure requires users to know which command to use for different queries:
- `whois` for AS info (misleading name - not actual IRR whois)
- `pfx2as` for prefix-to-ASN mapping
- `rpki` for RPKI validation
- `as2rel` for AS relationships

**New approach**: A unified `inspect` command that accepts any query (ASN, prefix, name, IP) and returns all relevant information from all available data sources.

### Design Philosophy

1. **One command to rule them all**: `monocle inspect <QUERY>...`
2. **Smart query detection**: Auto-detect ASN vs prefix vs name with deterministic rules
3. **Comprehensive output**: Show all relevant data sources
4. **Selective querying**: `--select` flag to focus on specific data
5. **Sensible defaults**: Summarize large datasets, expand on request
6. **Rich JSON**: Include everything in JSON output
7. **Adaptive display**: Adjust table output to terminal width

### Commands to Remove

The following commands will be **removed** and consolidated into `inspect`:

| Removed Command | Replacement |
|-----------------|-------------|
| `monocle whois <query>` | `monocle inspect <query>` |
| `monocle pfx2as <prefix>` | `monocle inspect <prefix>` |
| `monocle pfx2as --asn <asn>` | `monocle inspect <asn> --select prefixes` |
| `monocle as2rel <asn>` | `monocle inspect <asn> --select connectivity` |

**Note**: `monocle rpki` will be kept for advanced operations (historical data, batch validation) but its core functionality is integrated into `inspect`.

### Commands Kept

| Command | Reason |
|---------|--------|
| `monocle rpki` | Advanced: historical data, batch validation, date ranges |
| `monocle search` | MRT file searching - different purpose |
| `monocle parse` | MRT file parsing - different purpose |
| `monocle time` | Time conversion utility |
| `monocle country` | Country code lookup utility |
| `monocle ip` | IP information lookup |
| `monocle database` | Database management (refresh, status, etc.) |
| `monocle config` | Configuration management |

---

### Query Parsing Rules

The `InspectLens::detect_query_type` function uses these **deterministic rules** (evaluated in order):

| Pattern | Detection | Example |
|---------|-----------|---------|
| Starts with `AS` or `as` followed by digits | ASN | `AS13335`, `as174` |
| Pure digits | ASN | `13335`, `174` |
| Contains `/` | Prefix (CIDR) | `1.1.1.0/24`, `2001:db8::/32` |
| IPv4 address (no `/`) | Prefix (auto `/32`) | `1.1.1.1` → `1.1.1.1/32` |
| IPv6 address (no `/`) | Prefix (auto `/128`) | `2001:db8::1` → `2001:db8::1/128` |
| Everything else | Name search | `cloudflare`, `Google LLC` |

**Multiple queries**: The CLI accepts multiple space-separated queries. Each is parsed independently and results are returned in the `queries[]` array.

```bash
# Mixed query types in one invocation
monocle inspect 13335 1.1.1.0/24 cloudflare AS15169
```

**Forcing query type**: Use flags to override auto-detection:
- `--asn` - Treat all queries as ASNs
- `--prefix` - Treat all queries as prefixes
- `--name` - Treat all queries as name searches

---

### Data Source

**URL**: `http://spaces.bgpkit.org/broker/asninfo.jsonl`  
**Format**: JSON Lines (one JSON object per line)  
**Size**: ~40MB, ~120,443 records

#### Data Structure

```json
{
  "asn": 13335,
  "name": "CLOUDFLARENET",
  "country": "US",
  "as2org": {
    "country": "US",
    "name": "CLOUDFLARENET",
    "org_id": "CLOUD14-ARIN",
    "org_name": "Cloudflare, Inc."
  },
  "peeringdb": {
    "aka": "",
    "asn": 13335,
    "irr_as_set": "AS13335:AS-CLOUDFLARE",
    "name": "Cloudflare",
    "name_long": "",
    "website": "https://www.cloudflare.com"
  },
  "hegemony": {
    "asn": 13335,
    "ipv4": 0.002026,
    "ipv6": 0.008380
  },
  "population": {
    "percent_country": 0.03,
    "percent_global": 0.0,
    "sample_count": 31,
    "user_count": 17
  }
}
```

#### Field Coverage

| Field | Records with Data | Description |
|-------|-------------------|-------------|
| `asn` | 120,443 (100%) | AS number |
| `name` | 120,443 (100%) | AS name |
| `country` | 120,443 (100%) | Country code |
| `as2org` | 115,135 (95.6%) | CAIDA AS2Org data |
| `peeringdb` | 33,490 (27.8%) | PeeringDB network info |
| `hegemony` | 81,152 (67.4%) | IHR AS hegemony scores |
| `population` | 38,686 (32.1%) | APNIC population estimates |

---

### Design Plan

#### 2.1 Normalized Database Schema

**Design Philosophy**: Use normalized tables for cleaner separation and extensibility. Each data source gets its own table, making it easy to add new data sources in the future.

**File**: `src/database/repositories/asinspect.rs`

```sql
-- Core AS table (always populated)
-- Cardinality: 1 row per ASN (primary key)
CREATE TABLE asinfo_core (
    asn INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    country TEXT NOT NULL
);

-- AS2Org data (from CAIDA)
-- Cardinality: 0 or 1 row per ASN (optional, 1:1)
CREATE TABLE asinfo_as2org (
    asn INTEGER PRIMARY KEY REFERENCES asinfo_core(asn),
    name TEXT NOT NULL,
    org_id TEXT NOT NULL,
    org_name TEXT NOT NULL,
    country TEXT NOT NULL
);

-- PeeringDB data
-- Cardinality: 0 or 1 row per ASN (optional, 1:1)
CREATE TABLE asinfo_peeringdb (
    asn INTEGER PRIMARY KEY REFERENCES asinfo_core(asn),
    name TEXT NOT NULL,
    name_long TEXT,
    aka TEXT,
    website TEXT,
    irr_as_set TEXT
);

-- IHR Hegemony scores
-- Cardinality: 0 or 1 row per ASN (optional, 1:1)
CREATE TABLE asinfo_hegemony (
    asn INTEGER PRIMARY KEY REFERENCES asinfo_core(asn),
    ipv4 REAL NOT NULL,
    ipv6 REAL NOT NULL
);

-- APNIC Population estimates
-- Cardinality: 0 or 1 row per ASN (optional, 1:1)
CREATE TABLE asinfo_population (
    asn INTEGER PRIMARY KEY REFERENCES asinfo_core(asn),
    percent_country REAL NOT NULL,
    percent_global REAL NOT NULL,
    sample_count INTEGER NOT NULL,
    user_count INTEGER NOT NULL
);

-- Metadata table
CREATE TABLE asinfo_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    source_url TEXT NOT NULL,
    last_updated INTEGER NOT NULL,
    core_count INTEGER NOT NULL,
    as2org_count INTEGER NOT NULL,
    peeringdb_count INTEGER NOT NULL,
    hegemony_count INTEGER NOT NULL,
    population_count INTEGER NOT NULL
);

-- Indexes for common queries
CREATE INDEX idx_asinfo_core_name ON asinfo_core(name);
CREATE INDEX idx_asinfo_core_country ON asinfo_core(country);
CREATE INDEX idx_asinfo_as2org_org_id ON asinfo_as2org(org_id);
CREATE INDEX idx_asinfo_as2org_org_name ON asinfo_as2org(org_name);
CREATE INDEX idx_asinfo_peeringdb_name ON asinfo_peeringdb(name);
```

**Join semantics**: All tables join on `asn`. The `AsinfoFullRecord` is constructed via LEFT JOINs from `asinfo_core` to all other tables.

**Conflict resolution**: If the source JSONL contains duplicate ASNs, last-write-wins during import.

#### 2.2 Rust Types

**File**: `src/database/repositories/asinspect.rs`

```rust
/// Core AS information (always present)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoCoreRecord {
    pub asn: u32,
    pub name: String,
    pub country: String,
}

/// AS2Org data from CAIDA
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoAs2orgRecord {
    pub asn: u32,
    pub name: String,
    pub org_id: String,
    pub org_name: String,
    pub country: String,
}

/// PeeringDB network information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoPeeringdbRecord {
    pub asn: u32,
    pub name: String,
    pub name_long: Option<String>,
    pub aka: Option<String>,
    pub website: Option<String>,
    pub irr_as_set: Option<String>,
}

/// IHR AS Hegemony scores
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoHegemonyRecord {
    pub asn: u32,
    pub ipv4: f64,
    pub ipv6: f64,
}

/// APNIC Population estimates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoPopulationRecord {
    pub asn: u32,
    /// Percentage of country's users (0.0 - 100.0)
    pub percent_country: f64,
    /// Percentage of global users (0.0 - 100.0)
    pub percent_global: f64,
    pub sample_count: u32,
    pub user_count: u32,
}

/// Complete AS information (joined from all tables)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoFullRecord {
    pub core: AsinfoCoreRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as2org: Option<AsinfoAs2orgRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peeringdb: Option<AsinfoPeeringdbRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hegemony: Option<AsinfoHegemonyRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub population: Option<AsinfoPopulationRecord>,
}
```

**Numeric conventions**:
- Percentages: `0.0` to `100.0` (not 0-1) for human readability
- Hegemony scores: Raw values from source (typically small decimals like `0.002`)
- All floats use `f64`

#### 2.3 Repository API

**File**: `src/database/repositories/asinspect.rs`

Methods marked **[MVP]** are required for initial `inspect` command. Others are **[Later]**.

```rust
pub struct AsinfoRepository<'a> { ... }

impl AsinfoRepository {
    // === Data Loading ===
    
    /// [MVP] Store records from parsed JSONL, returns counts per table
    pub fn store_from_jsonl(&self, records: &[JsonlRecord], source_url: &str) -> Result<AsinfoStoreCounts>;
    
    /// [MVP] Fetch URL and store (convenience wrapper)
    pub fn load_from_url(&self, url: &str) -> Result<AsinfoStoreCounts>;
    
    /// [MVP] Clear all asinfo tables
    pub fn clear(&self) -> Result<()>;
    
    // === Metadata ===
    
    /// [MVP] Check if core table is empty
    pub fn is_empty(&self) -> bool;
    
    /// [MVP] Check if data needs refresh based on TTL
    pub fn needs_refresh(&self, ttl: Duration) -> bool;
    
    /// [MVP] Get metadata (timestamp, counts, source URL)
    pub fn get_metadata(&self) -> Result<Option<AsinfoMetadata>>;
    
    // === Core Queries ===
    
    /// [MVP] Get full record for single ASN (LEFT JOINs all tables)
    pub fn get_full(&self, asn: u32) -> Result<Option<AsinfoFullRecord>>;
    
    /// [MVP] Get full records for multiple ASNs (batch)
    pub fn get_full_batch(&self, asns: &[u32]) -> Result<Vec<AsinfoFullRecord>>;
    
    /// [MVP] Search by AS name OR org name (merged, deduplicated)
    pub fn search_by_text(&self, query: &str) -> Result<Vec<AsinfoCoreRecord>>;
    
    /// [MVP] Search by country code
    pub fn search_by_country(&self, country: &str) -> Result<Vec<AsinfoCoreRecord>>;
    
    // === Batch Lookups (for enrichment) ===
    
    /// [MVP] Batch lookup of AS names
    pub fn lookup_names_batch(&self, asns: &[u32]) -> HashMap<u32, String>;
    
    /// [Later] Batch lookup of org names
    pub fn lookup_orgs_batch(&self, asns: &[u32]) -> HashMap<u32, String>;
    
    // === Individual Table Queries ===
    
    /// [Later] Get just core record
    pub fn get_core(&self, asn: u32) -> Result<Option<AsinfoCoreRecord>>;
    
    /// [Later] Get just AS2Org record
    pub fn get_as2org(&self, asn: u32) -> Result<Option<AsinfoAs2orgRecord>>;
    
    /// [Later] Get just PeeringDB record  
    pub fn get_peeringdb(&self, asn: u32) -> Result<Option<AsinfoPeeringdbRecord>>;
    
    /// [Later] Get just hegemony record
    pub fn get_hegemony(&self, asn: u32) -> Result<Option<AsinfoHegemonyRecord>>;
    
    /// [Later] Get just population record
    pub fn get_population(&self, asn: u32) -> Result<Option<AsinfoPopulationRecord>>;
    
    /// [Later] Search specifically by org_id
    pub fn search_by_org_id(&self, org_id: &str) -> Result<Vec<AsinfoAs2orgRecord>>;
}
```

#### 2.4 AS2Rel Integration - Connectivity Summary

**Enhancement to AS2Rel**: Instead of raw relationship data, provide structured connectivity information.

```rust
/// AS connectivity summary (derived from AS2Rel data)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsConnectivitySummary {
    pub asn: u32,
    
    /// Upstream providers
    pub upstreams: ConnectivityGroup,
    
    /// Peers (lateral connections)
    pub peers: ConnectivityGroup,
    
    /// Downstream customers
    pub downstreams: ConnectivityGroup,
    
    /// Total unique neighbors
    pub total_neighbors: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityGroup {
    /// Total count in this category
    pub count: u32,
    
    /// Percentage of total neighbors (0.0 - 100.0)
    pub percent: f64,
    
    /// Top N neighbors sorted by peers_count DESC, then ASN ASC
    pub top: Vec<ConnectivityEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityEntry {
    pub asn: u32,
    /// Enriched from asinfo (None if not found)
    pub name: Option<String>,
    pub peers_count: u32,
}
```

**Repository method**:
```rust
impl As2relRepository {
    /// [MVP] Get connectivity summary for an ASN
    /// Uses lookup_names_batch internally for enrichment
    pub fn get_connectivity_summary(&self, asn: u32, top_n: usize) -> Result<AsConnectivitySummary>;
}
```

**Sorting rule**: Top entries sorted by `peers_count DESC`, then `asn ASC` for stability.

#### 2.5 RPKI Integration

RPKI data (ROAs, ASPAs) will be integrated into the `inspect` query results.

```rust
/// RPKI information for an ASN
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiAsnInfo {
    /// ROAs where this ASN is the origin
    pub roas: RoaSummary,
    
    /// ASPA record for this ASN (if exists)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspa: Option<AspaInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoaSummary {
    /// Total ROA count for this ASN
    pub total_count: usize,
    
    /// IPv4 ROA count
    pub ipv4_count: usize,
    
    /// IPv6 ROA count
    pub ipv6_count: usize,
    
    /// ROA entries (limited by default, sorted by prefix)
    pub entries: Vec<RpkiRoaRecord>,
    
    /// Whether entries were truncated
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AspaInfo {
    pub customer_asn: u32,
    pub provider_asns: Vec<u32>,
    /// Provider names (enriched from asinfo, None if not found)
    pub provider_names: Vec<Option<String>>,
}

/// RPKI information for a prefix
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiPrefixInfo {
    /// Covering ROAs (sorted by prefix, then max_length, then ASN)
    pub roas: Vec<RpkiRoaRecord>,
    
    /// ROA count
    pub roa_count: usize,
    
    /// Validation state (if single origin ASN known)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_state: Option<String>,
    
    /// Whether ROAs were truncated
    pub truncated: bool,
}
```

#### 2.6 Data Section Selection

```rust
/// Available data sections that can be selected via --select
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectDataSection {
    /// Core AS information (name, country, org)
    Core,
    /// PeeringDB data (website, IRR as-set)
    Peeringdb,
    /// Hegemony scores
    Hegemony,
    /// Population estimates
    Population,
    /// Announced prefixes (from pfx2as) - NOT included in ASN defaults
    Prefixes,
    /// AS connectivity (from as2rel)
    Connectivity,
    /// RPKI ROAs
    Roas,
    /// RPKI ASPA
    Aspa,
}

impl InspectDataSection {
    pub fn all() -> Vec<Self> {
        vec![
            Self::Core,
            Self::Peeringdb,
            Self::Hegemony,
            Self::Population,
            Self::Prefixes,
            Self::Connectivity,
            Self::Roas,
            Self::Aspa,
        ]
    }
    
    /// Default sections for ASN queries
    /// Note: Prefixes excluded by default (can be thousands)
    pub fn default_for_asn() -> Vec<Self> {
        vec![
            Self::Core,
            Self::Peeringdb,
            Self::Hegemony,
            Self::Population,
            Self::Connectivity,
            Self::Roas,
            Self::Aspa,
        ]
    }
    
    /// Default sections for prefix queries
    pub fn default_for_prefix() -> Vec<Self> {
        vec![Self::Core, Self::Roas]
    }
    
    /// Default sections for name search
    pub fn default_for_name() -> Vec<Self> {
        vec![Self::Core]
    }
}
```

**Selection rules**:
- `--select` **overrides** defaults completely (not additive)
- `--select all` includes all sections
- If a section has no data, it is **omitted** from output (not an error)
- Unknown section names are rejected with an error

#### 2.7 Unified Info Result Types

**File**: `src/lens/inspect/types.rs`

**Naming clarification**:
- `prefix` (singular): The queried prefix in a prefix query
- `prefixes` (plural): Prefixes announced by an ASN (from pfx2as)
- `detail`: Full ASInfo for the directly queried ASN
- `origins`: ASInfo for origin ASNs discovered via pfx2as (for prefix queries)

```rust
/// Query type detected from input
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InspectQueryType {
    Asn,
    Prefix,
    Name,
}

/// Result for a single query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectQueryResult {
    /// Original query string
    pub query: String,
    
    /// Detected query type
    pub query_type: InspectQueryType,
    
    /// ASN information section
    /// - For ASN queries: contains `detail` (full record for queried ASN)
    /// - For prefix queries: contains `origins` (records for origin ASNs)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asinfo: Option<AsinfoSection>,
    
    /// Prefix information (for prefix queries only)
    /// Contains pfx2as mapping and RPKI validation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<PrefixSection>,
    
    /// Announced prefixes (for ASN queries with --select prefixes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefixes: Option<AnnouncedPrefixesSection>,
    
    /// Connectivity information (for ASN queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connectivity: Option<ConnectivitySection>,
    
    /// RPKI information (for ASN queries - ROAs originated, ASPA)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpki: Option<RpkiAsnInfo>,
    
    /// Search results (for name queries only)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_results: Option<SearchResultsSection>,
}

/// ASN information section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsinfoSection {
    /// Full AS info for directly queried ASN (ASN queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<AsinfoFullRecord>,
    
    /// AS info for origin ASNs (prefix queries via pfx2as)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origins: Option<Vec<AsinfoFullRecord>>,
}

/// Prefix information section (for prefix queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrefixSection {
    /// Prefix-to-AS mapping result
    pub pfx2as: Pfx2asInfo,
    
    /// RPKI information for this prefix
    pub rpki: RpkiPrefixInfo,
}

/// Announced prefixes section (for ASN queries with --select prefixes)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnouncedPrefixesSection {
    /// Total prefix count
    pub total_count: usize,
    
    /// IPv4 count
    pub ipv4_count: usize,
    
    /// IPv6 count
    pub ipv6_count: usize,
    
    /// Prefix entries (sorted by prefix, limited by default)
    pub prefixes: Vec<String>,
    
    /// Whether prefixes were truncated
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pfx2asInfo {
    pub prefix: String,
    pub origin_asns: Vec<u32>,
    /// "exact" or "longest"
    pub match_type: String,
}

/// Connectivity section (for ASN queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivitySection {
    pub summary: AsConnectivitySummary,
    
    /// Whether neighbor lists were truncated
    pub truncated: bool,
}

/// Search results section (for name queries)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultsSection {
    /// Total matches found
    pub total_matches: usize,
    
    /// Results (sorted by ASN, limited by default)
    pub results: Vec<AsinfoCoreRecord>,
    
    /// Whether results were truncated
    pub truncated: bool,
}

/// Combined result for multiple queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResult {
    /// Individual query results
    pub queries: Vec<InspectQueryResult>,
    
    /// Processing metadata
    pub meta: InspectResultMeta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResultMeta {
    pub query_count: usize,
    pub asn_queries: usize,
    pub prefix_queries: usize,
    pub name_queries: usize,
    pub processing_time_ms: u64,
}
```

#### 2.8 Query Options

```rust
pub struct InspectQueryOptions {
    /// Which data sections to include (None = defaults based on query type)
    pub select: Option<Vec<InspectDataSection>>,
    
    /// Maximum ROAs to return (0 = unlimited)
    pub max_roas: usize,
    
    /// Maximum prefixes to return (0 = unlimited)
    pub max_prefixes: usize,
    
    /// Maximum neighbors per category (0 = unlimited)
    pub max_neighbors: usize,
    
    /// Maximum search results (0 = unlimited)
    pub max_search_results: usize,
}

impl Default for InspectQueryOptions {
    fn default() -> Self {
        Self {
            select: None,  // Use defaults based on query type
            max_roas: 10,
            max_prefixes: 10,
            max_neighbors: 5,
            max_search_results: 20,
        }
    }
}

impl InspectQueryOptions {
    pub fn full() -> Self {
        Self {
            select: Some(InspectDataSection::all()),
            max_roas: 0,
            max_prefixes: 0,
            max_neighbors: 0,
            max_search_results: 0,
        }
    }
    
    pub fn with_select(mut self, sections: Vec<InspectDataSection>) -> Self {
        self.select = Some(sections);
        self
    }
}
```

**Default limits**:

| Data Type | Default Limit | Expand Flag |
|-----------|---------------|-------------|
| ROAs | 10 | `--full-roas` |
| Prefixes | 10 | `--full-prefixes` |
| Neighbors (per category) | 5 | `--full-connectivity` |
| Search results | 20 | `--limit N` |
| Everything | - | `--full` |

#### 2.9 Inspect Lens API

**File**: `src/lens/inspect/mod.rs`

```rust
pub struct InspectLens<'a> {
    db: &'a MonocleDatabase,
    country_lookup: CountryLens,
}

impl InspectLens {
    pub fn new(db: &MonocleDatabase) -> Self;
    
    // === Status ===
    
    pub fn is_data_available(&self) -> bool;
    pub fn needs_bootstrap(&self) -> bool;
    
    // === Data Management ===
    // InspectLens owns refresh policy, delegates to repositories
    
    pub fn bootstrap(&self) -> Result<AsinfoStoreCounts>;
    pub fn refresh(&self) -> Result<AsinfoStoreCounts>;
    
    // === Main Query Interface ===
    
    /// Process multiple mixed queries
    pub fn query(&self, inputs: &[String], options: &InspectQueryOptions) -> Result<InspectResult>;
    
    // === Internal Query Methods ===
    
    fn query_asn(&self, asn: u32, options: &InspectQueryOptions) -> Result<InspectQueryResult>;
    fn query_prefix(&self, prefix: &str, options: &InspectQueryOptions) -> Result<InspectQueryResult>;
    fn query_name(&self, name: &str, options: &InspectQueryOptions) -> Result<InspectQueryResult>;
    
    // === Query Type Detection ===
    
    fn detect_query_type(&self, input: &str) -> InspectQueryType;
    
    // === Section Filtering ===
    
    fn should_include(&self, section: InspectDataSection, query_type: &InspectQueryType, options: &InspectQueryOptions) -> bool;
    
    // === Quick Lookups (for enrichment in other commands) ===
    
    pub fn lookup_name(&self, asn: u32) -> Option<String>;
    pub fn lookup_org(&self, asn: u32) -> Option<String>;
    
    // === Formatting ===
    
    pub fn format(&self, result: &InspectResult, format: &InspectOutputFormat, config: &InspectDisplayConfig) -> String;
}
```

**Refresh policy** (owned by `InspectLens`):
- `needs_bootstrap()`: Returns true if `asinfo_core` is empty
- `needs_refresh()`: Checks metadata timestamp against configured TTL
- On refresh failure: Log error, serve stale data if available, return error only if no data exists

#### 2.10 Dynamic Terminal Width Display

```rust
/// Determines display configuration based on terminal width
pub struct InspectDisplayConfig {
    pub terminal_width: usize,
    pub show_hegemony: bool,
    pub show_population: bool,
    pub show_peeringdb: bool,
    pub truncate_names: bool,
    pub name_max_width: usize,
}

impl InspectDisplayConfig {
    /// Create display config based on terminal width
    pub fn from_terminal_width(width: usize) -> Self {
        match width {
            0..=80 => Self {
                terminal_width: width,
                show_hegemony: false,
                show_population: false,
                show_peeringdb: false,
                truncate_names: true,
                name_max_width: 15,
            },
            81..=120 => Self {
                terminal_width: width,
                show_hegemony: false,
                show_population: false,
                show_peeringdb: false,
                truncate_names: true,
                name_max_width: 25,
            },
            121..=160 => Self {
                terminal_width: width,
                show_hegemony: true,
                show_population: false,
                show_peeringdb: false,
                truncate_names: true,
                name_max_width: 30,
            },
            _ => Self {
                terminal_width: width,
                show_hegemony: true,
                show_population: true,
                show_peeringdb: true,
                truncate_names: false,
                name_max_width: 50,
            },
        }
    }
    
    /// Auto-detect terminal width
    pub fn auto() -> Self {
        let width = terminal_size::terminal_size()
            .map(|(w, _)| w.0 as usize)
            .unwrap_or(80);
        Self::from_terminal_width(width)
    }
}
```

**Column contract by width**:

| Width | Columns Shown | Name Truncation |
|-------|---------------|-----------------|
| ≤80 | ASN, Name, Country | 15 chars |
| 81-120 | + Org Name | 25 chars |
| 121-160 | + Hegemony (IPv4/IPv6) | 30 chars |
| >160 | + Population, PeeringDB | No truncation (50 chars max) |

#### 2.11 CLI Command

**File**: `src/bin/commands/inspect.rs`

```
USAGE:
    monocle inspect [OPTIONS] <QUERY>...

DESCRIPTION:
    Query information about ASNs and IP prefixes. Accepts multiple queries
    of mixed types (ASN, prefix, or name search). Consolidates functionality
    from the former whois, pfx2as, and as2rel commands.

ARGS:
    <QUERY>...    One or more queries:
                  - ASN: "13335" or "AS13335"
                  - Prefix: "1.1.1.0/24" or "2606:4700::/32"
                  - IP: "1.1.1.1" (treated as /32 prefix)
                  - Name: "cloudflare" (searches AS name and org)

QUERY OPTIONS:
    -a, --asn             Force treat queries as ASNs
    -p, --prefix          Force treat queries as prefixes
    -n, --name            Force treat queries as name search
    -c, --country <CC>    Search by country code
    -o, --org <ORG>       Search by organization name/ID

DATA SELECTION:
    -s, --select <SECTION>   Select specific data sections to query.
                             Can be specified multiple times.
                             Overrides defaults (not additive).
                             
                             Available sections:
                               core         - Basic AS info (name, country, org)
                               peeringdb    - PeeringDB data (website, IRR)
                               hegemony     - AS hegemony scores
                               population   - APNIC population estimates
                               prefixes     - Announced prefixes (from pfx2as)
                               connectivity - AS relationships (from as2rel)
                               roas         - RPKI ROAs
                               aspa         - RPKI ASPA records
                               all          - All sections
                             
                             Defaults:
                               ASN query: core,peeringdb,hegemony,population,connectivity,roas,aspa
                               Prefix query: core,roas
                             
                             Example: -s connectivity -s roas

OUTPUT OPTIONS:
    --full                Show all data sections with no limits
    --full-roas           Show all RPKI ROAs (default: top 10)
    --full-prefixes       Show all prefixes (default: top 10)
    --full-connectivity   Show all neighbors (default: top 5 per category)
    --limit <N>           Limit search results (default: 20)

DATA OPTIONS:
    -u, --update          Force refresh the asinfo database

FORMAT OPTIONS:
    -f, --format <FMT>    Output format [default: table]
                          [table, json, json-pretty, json-line, markdown, psv]

EXAMPLES:
    # Single ASN query (shows all default sections)
    monocle inspect 13335
    
    # Multiple mixed queries
    monocle inspect 13335 1.1.1.0/24 AS15169
    
    # Focus on specific data with --select
    monocle inspect 13335 --select connectivity
    monocle inspect 13335 --select roas --select aspa
    monocle inspect 13335 -s prefixes -s connectivity
    
    # Get only announced prefixes for an ASN (replaces pfx2as)
    monocle inspect 13335 --select prefixes --full-prefixes
    
    # Get only connectivity data (replaces as2rel)
    monocle inspect 13335 --select connectivity --full-connectivity
    
    # Name search (merges AS name + org matches)
    monocle inspect cloudflare
    monocle inspect --name "Level 3"
    
    # Country search
    monocle inspect --country US --limit 50
    
    # Prefix query with validation info
    monocle inspect 1.1.1.0/24
    
    # IP address (auto-converted to /32)
    monocle inspect 1.1.1.1
    
    # JSON output for scripting
    monocle inspect 13335 1.1.1.0/24 --format json
    
    # Full output with no truncation
    monocle inspect 13335 --full
    
    # Force refresh and query
    monocle inspect 13335 --update
```

#### 2.12 JSON Output Examples

**ASN Query** (`monocle inspect 13335 --json`):
```json
{
  "queries": [
    {
      "query": "13335",
      "query_type": "asn",
      "asinfo": {
        "detail": {
          "core": {
            "asn": 13335,
            "name": "CLOUDFLARENET",
            "country": "US"
          },
          "as2org": {
            "asn": 13335,
            "name": "CLOUDFLARENET",
            "org_id": "CLOUD14-ARIN",
            "org_name": "Cloudflare, Inc.",
            "country": "US"
          },
          "peeringdb": {
            "asn": 13335,
            "name": "Cloudflare",
            "website": "https://www.cloudflare.com",
            "irr_as_set": "AS13335:AS-CLOUDFLARE"
          },
          "hegemony": {
            "asn": 13335,
            "ipv4": 0.002026,
            "ipv6": 0.008380
          },
          "population": {
            "asn": 13335,
            "percent_country": 3.0,
            "percent_global": 0.0,
            "sample_count": 31,
            "user_count": 17
          }
        }
      },
      "connectivity": {
        "summary": {
          "asn": 13335,
          "upstreams": {
            "count": 12,
            "percent": 0.85,
            "top": [
              {"asn": 174, "name": "COGENT", "peers_count": 450}
            ]
          },
          "peers": {
            "count": 1200,
            "percent": 85.11,
            "top": [
              {"asn": 15169, "name": "GOOGLE", "peers_count": 520}
            ]
          },
          "downstreams": {
            "count": 198,
            "percent": 14.04,
            "top": []
          },
          "total_neighbors": 1410
        },
        "truncated": true
      },
      "rpki": {
        "roas": {
          "total_count": 156,
          "ipv4_count": 120,
          "ipv6_count": 36,
          "entries": [
            {"prefix": "1.1.1.0/24", "max_length": 24, "origin_asn": 13335, "ta": "APNIC"}
          ],
          "truncated": true
        },
        "aspa": {
          "customer_asn": 13335,
          "provider_asns": [174, 3356, 6939],
          "provider_names": ["COGENT", "LUMEN", "HE"]
        }
      }
    }
  ],
  "meta": {
    "query_count": 1,
    "asn_queries": 1,
    "prefix_queries": 0,
    "name_queries": 0,
    "processing_time_ms": 85
  }
}
```

**Prefix Query** (`monocle inspect 1.1.1.0/24 --json`):
```json
{
  "queries": [
    {
      "query": "1.1.1.0/24",
      "query_type": "prefix",
      "asinfo": {
        "origins": [
          {
            "core": {
              "asn": 13335,
              "name": "CLOUDFLARENET",
              "country": "US"
            },
            "as2org": {
              "asn": 13335,
              "org_name": "Cloudflare, Inc."
            }
          }
        ]
      },
      "prefix": {
        "pfx2as": {
          "prefix": "1.1.1.0/24",
          "origin_asns": [13335],
          "match_type": "exact"
        },
        "rpki": {
          "roa_count": 1,
          "validation_state": "valid",
          "roas": [
            {
              "prefix": "1.1.1.0/24",
              "max_length": 24,
              "origin_asn": 13335,
              "ta": "APNIC"
            }
          ],
          "truncated": false
        }
      }
    }
  ],
  "meta": {
    "query_count": 1,
    "asn_queries": 0,
    "prefix_queries": 1,
    "name_queries": 0,
    "processing_time_ms": 32
  }
}
```

#### 2.13 Table Output Examples

**ASN Query** (narrow terminal ≤80):
```
╭─────────────────────────────────────────────────────────────────╮
│                        AS13335 Info                             │
├─────────────────────────────────────────────────────────────────┤
│ ASN         │ 13335                                             │
│ Name        │ CLOUDFLARENET                                     │
│ Country     │ United States (US)                                │
│ Org         │ Cloudflare, Inc.                                  │
│ Website     │ https://www.cloudflare.com                        │
╰─────────────────────────────────────────────────────────────────╯

╭───────────────────── Connectivity ──────────────────────╮
│ Upstreams    │    12 (  0.9%) │ COGENT, LUMEN, ...     │
│ Peers        │ 1,200 ( 85.1%) │ GOOGLE, META, ...      │
│ Downstreams  │   198 ( 14.0%) │                        │
│ Total        │ 1,410 neighbors                         │
╰─────────────────────────────────────────────────────────╯

╭─────────────────────── RPKI ────────────────────────────╮
│ ROAs         │ 156 total (120 IPv4, 36 IPv6)           │
│ ASPA         │ Providers: COGENT, LUMEN, HE            │
╰─────────────────────────────────────────────────────────╯
```

**Prefix Query**:
```
╭─────────────────────────────────────────────────────────────────╮
│                     1.1.1.0/24 Info                             │
├─────────────────────────────────────────────────────────────────┤
│ Prefix      │ 1.1.1.0/24                                        │
│ Origin      │ AS13335 (CLOUDFLARENET)                           │
│ Match       │ exact                                             │
╰─────────────────────────────────────────────────────────────────╯

╭─────────────────────── RPKI ────────────────────────────╮
│ Status      │ ✓ Valid                                   │
│ ROAs        │ 1 covering ROA                            │
├─────────────────────────────────────────────────────────┤
│ 1.1.1.0/24 │ max /24 │ AS13335 │ APNIC                  │
╰─────────────────────────────────────────────────────────╯
```

**Search Results** (`monocle inspect cloudflare`):
```
╭───────────────────────────────────────────────────────────────────────────╮
│ Search: "cloudflare" (5 matches)                                          │
├───────┬─────────────────────────┬──────────────────────────┬─────────────┤
│ ASN   │ Name                    │ Organization             │ Country     │
├───────┼─────────────────────────┼──────────────────────────┼─────────────┤
│ 13335 │ CLOUDFLARENET           │ Cloudflare, Inc.         │ US          │
│132892 │ CLOUDFLARENET-AS-AP     │ Cloudflare, Inc.         │ SG          │
│209242 │ CLOUDFLARENET-EU        │ Cloudflare, Inc.         │ NL          │
│394536 │ CLOUDFLARE-CHINA        │ Cloudflare, Inc.         │ CN          │
│395747 │ CLOUDFLARE-MGMT         │ Cloudflare, Inc.         │ US          │
╰───────┴─────────────────────────┴──────────────────────────┴─────────────╯
```

---

### Tasks

#### 2.1 Create ASInfo Database Module

**Priority**: High  
**Status**: Not Started

- [ ] Create `src/database/repositories/asinspect.rs`
- [ ] Define schema constants for all 5 tables + meta + indexes
- [ ] Define record types with serde derives
- [ ] Implement `AsinfoRepository` struct
- [ ] **[MVP]** Implement `store_from_jsonl()` - parse JSONL and insert into all tables
- [ ] **[MVP]** Implement `load_from_url()` - fetch URL and call store
- [ ] **[MVP]** Implement `clear()` - truncate all tables
- [ ] **[MVP]** Implement `is_empty()`, `needs_refresh()`, `get_metadata()`
- [ ] **[MVP]** Implement `get_full()` - LEFT JOIN all tables
- [ ] **[MVP]** Implement `get_full_batch()` - batch LEFT JOIN
- [ ] **[MVP]** Implement `search_by_text()` - merge name + org searches, deduplicate
- [ ] **[MVP]** Implement `search_by_country()`
- [ ] **[MVP]** Implement `lookup_names_batch()`
- [ ] Add schema initialization to `SchemaManager`
- [ ] Add `asinfo()` method to `MonocleDatabase`
- [ ] Write unit tests

#### 2.2 Enhance AS2Rel with Connectivity Summary

**Priority**: High  
**Status**: Not Started

- [ ] Add `AsConnectivitySummary`, `ConnectivityGroup`, `ConnectivityEntry` types
- [ ] **[MVP]** Implement `get_connectivity_summary()` in `As2relRepository`
- [ ] Group relationships into upstreams/peers/downstreams
- [ ] Calculate percentages (0.0 - 100.0)
- [ ] Return top N per category (sorted by peers_count DESC, then ASN ASC)
- [ ] Use `lookup_names_batch()` for enrichment
- [ ] Write unit tests

#### 2.3 Integrate RPKI into Info Query

**Priority**: High  
**Status**: Not Started

- [ ] Add `RpkiAsnInfo`, `RoaSummary`, `AspaInfo`, `RpkiPrefixInfo` types
- [ ] **[MVP]** Add method to get ROA summary for an ASN (with counts)
- [ ] **[MVP]** Add method to get ASPA record for an ASN
- [ ] **[MVP]** Enrich provider names in ASPA using `lookup_names_batch()`
- [ ] Integrate into `InspectQueryResult`

#### 2.4 Create Inspect Lens Module

**Priority**: High  
**Status**: Not Started

- [ ] Create `src/lens/inspect/mod.rs`
- [ ] Create `src/lens/inspect/args.rs` - `InspectQueryOptions`, `InspectDataSection`
- [ ] Create `src/lens/inspect/types.rs` - All result types
- [ ] Create `src/lens/inspect/display.rs` - `InspectDisplayConfig`, terminal width handling
- [ ] Implement `InspectLens` struct with `db` reference
- [ ] Implement `is_data_available()`, `needs_bootstrap()`
- [ ] Implement `bootstrap()`, `refresh()` with failure handling
- [ ] **[MVP]** Implement `detect_query_type()` with documented rules
- [ ] **[MVP]** Implement `should_include()` - check section selection
- [ ] **[MVP]** Implement `query()` - process multiple mixed queries
- [ ] **[MVP]** Implement `query_asn()` - fetch selected sections
- [ ] **[MVP]** Implement `query_prefix()` - fetch pfx2as + asinfo + RPKI
- [ ] **[MVP]** Implement `query_name()` - search, merge, deduplicate
- [ ] Implement result limiting logic
- [ ] **[MVP]** Implement `lookup_name()`, `lookup_org()` - quick lookups
- [ ] **[MVP]** Implement `format()` with dynamic width
- [ ] Integrate with `CountryLens` for country name expansion
- [ ] Export from `src/lens/mod.rs`
- [ ] Write unit tests

#### 2.5 Create CLI Command

**Priority**: High  
**Status**: Not Started

- [ ] Create `src/bin/commands/inspect.rs`
- [ ] Define `InspectArgs` with clap derives
- [ ] Implement `--select` flag (multiple values, overrides defaults)
- [ ] Implement `run()` function
- [ ] Handle multiple mixed queries
- [ ] Handle search modes (name, country, org)
- [ ] Implement `--full*` flags for result expansion
- [ ] Implement dynamic terminal width detection
- [ ] Add to command enum in `src/bin/monocle.rs`
- [ ] Test CLI functionality

#### 2.6 Remove Old Modules and Commands

**Priority**: Medium  
**Status**: Not Started

- [ ] Remove `src/bin/commands/whois.rs`
- [ ] Remove `src/bin/commands/pfx2as.rs`
- [ ] Remove `src/bin/commands/as2rel.rs`
- [ ] Remove `src/database/monocle/as2org.rs`
- [ ] Remove `src/lens/as2org/` directory
- [ ] Update `MonocleDatabase` to remove `as2org()` method
- [ ] Update schema to remove old as2org tables
- [ ] Update all imports and exports in `src/bin/monocle.rs`
- [ ] Update any code using old lenses to use `InspectLens`
- [ ] Update enrichment code in search/parse to use `InspectLens.lookup_name()`

#### 2.7 Integration with Remaining Commands

**Priority**: Medium  
**Status**: Not Started

- [ ] Update `search` command to use `InspectLens` for AS name enrichment
- [ ] Update `parse` command to use `InspectLens` for AS name enrichment
- [ ] Ensure consistent lookup behavior across commands

---

## Part 3: Future Extensibility

### Planned Additional Data Sources

The normalized schema design allows easy addition of new data sources:

| Future Table | Data Source | Description |
|--------------|-------------|-------------|
| `asinfo_rir` | RIR stats | Registration info (RIR, date, status) |
| `asinfo_geoloc` | MaxMind/IPInfo | Geographic location |
| `asinfo_ranking` | CAIDA AS-Rank | AS ranking/customer cone |
| `asinfo_type` | CAIDA AS-Type | AS classification (transit, content, etc.) |
| `asinfo_sibling` | CAIDA AS2Org | Sibling ASNs (same org) |

### Adding a New Data Section

To add a new data section:

1. Add new table to schema (with `asn` as primary key)
2. Add record type struct
3. Add variant to `InspectDataSection` enum
4. Add to appropriate defaults (`default_for_asn()`, etc.)
5. Add repository methods (follow MVP pattern)
6. Update `InspectLens::query_asn()` to fetch if section selected
7. Add to JSON output structure
8. Add table display formatting
9. Update CLI help text

### Section Provider Pattern (Future)

For better extensibility, consider a provider trait pattern:

```rust
trait InfoSectionProvider {
    fn section(&self) -> InspectDataSection;
    fn supports_query_type(&self, query_type: &InspectQueryType) -> bool;
    fn fetch(&self, query: &str, options: &InspectQueryOptions) -> Result<Option<serde_json::Value>>;
}
```

`InspectLens` would iterate providers based on `--select`, making it easy to add new sections without modifying core query logic.

---

## Part 4: Testing Strategy

### Unit Tests

- [ ] `detect_query_type()` - Test all patterns in Query Parsing Rules table
- [ ] `should_include()` - Test default selection per query type
- [ ] Repository CRUD operations with in-memory SQLite
- [ ] Connectivity summary calculation and sorting
- [ ] Result limiting and truncation flags

### Golden Output Tests

Create golden files for regression testing:

- [ ] JSON output for fixed ASN query (mocked data)
- [ ] JSON output for fixed prefix query (mocked data)
- [ ] JSON output for mixed queries
- [ ] Table output at width 80
- [ ] Table output at width 120
- [ ] Table output at width 160+

### Integration Tests

- [ ] End-to-end CLI test with test database
- [ ] Refresh/bootstrap with mocked HTTP responses
- [ ] Error handling (missing data, network failures)

---

## Progress Tracking

### Milestones

1. **M1**: ASInfo database module complete (normalized schema)
2. **M2**: AS2Rel connectivity summary complete
3. **M3**: RPKI integration complete
4. **M4**: Inspect lens module complete with --select support
5. **M5**: CLI `inspect` command complete
6. **M6**: Old modules/commands removed (whois, pfx2as, as2rel, as2org)
7. **M7**: Pfx2as and RPKI lens refactoring complete
8. **M8**: Database structure reorganized
9. **M9**: Documentation updated

### Completed

- [x] Architecture review completed
- [x] ASInfo data source analyzed
- [x] Normalized schema designed
- [x] Unified `inspect` command designed
- [x] Result limiting strategy defined
- [x] AS2Rel connectivity summary designed
- [x] Multiple query support designed
- [x] `--select` flag designed
- [x] RPKI integration designed
- [x] Command consolidation plan finalized
- [x] Layering rules documented
- [x] Query parsing rules documented
- [x] Result type semantics clarified
- [x] MVP vs Later methods identified
- [x] Testing strategy defined
- [x] Design document created

**Implementation Progress:**

- [x] **M1**: ASInfo database module (`src/database/monocle/asinfo.rs`)
  - Normalized tables: `asinfo_core`, `asinfo_as2org`, `asinfo_peeringdb`, `asinfo_hegemony`, `asinfo_population`, `asinfo_meta`
  - All MVP repository methods implemented
  - Schema integrated into `SchemaManager` (version bumped to 3)
  - Exported via `MonocleDatabase::asinfo()`
  - Unit tests passing

- [x] **M4**: Inspect lens module (`src/lens/inspect/`)
  - `types.rs`: All result types, section selection, query options, display config
  - `mod.rs`: `InspectLens` with `query()`, `query_asn()`, `query_prefix()`, `query_name()`
  - Query type detection (`detect_query_type()`)
  - AS2Rel connectivity summary integration
  - RPKI (ROAs, ASPA) integration
  - Pfx2as integration
  - Table and JSON formatting
  - Unit tests passing

- [x] **M5**: CLI `inspect` command (`src/bin/commands/inspect.rs`)
  - `--asn`, `--prefix`, `--name` type forcing flags
  - `--country` search option
  - `--select` section selection (multiple values)
  - `--full`, `--full-roas`, `--full-prefixes`, `--full-connectivity` expansion flags
  - `--limit` for search results
  - `--update` for data refresh
  - All output formats supported (table, json, json-pretty, json-line)

### Remaining Work

- [x] **M2**: Enhance AS2Rel with dedicated `get_connectivity_summary()` method
  - Added `AsConnectivitySummary`, `ConnectivityGroup`, `ConnectivityEntry` types to `as2rel.rs`
  - Implemented `get_connectivity_summary()` in `As2relRepository`
  - Implemented `would_truncate_connectivity()` helper
  - Updated `InspectLens` to use repository method instead of building in lens
  - Types re-exported from database module
- [x] **M3**: RPKI methods for ASN info queries (already implemented)
  - `get_roas_by_asn()` - returns ROAs where ASN is origin
  - `get_aspas_by_customer()` - returns ASPA records
  - `get_covering_roas()` - returns covering ROAs for prefix
  - `validate()` - validates prefix/ASN pair
  - InspectLens integrates all RPKI methods with provider name enrichment
- [x] **Display Improvements**:
  - ASPA now displays as a table with Provider ASN and Provider Name columns
  - "No ASPA record for this AS" message shown when ASPA is not available
  - Expanded name column widths based on terminal width (25→35→45→60)
- [x] **Auto-refresh Data**:
  - Added `ensure_data_available()` method to InspectLens
  - Checks and refreshes all data sources if empty or expired (asinfo, as2rel, rpki, pfx2as)
  - CLI shows messages about data refreshes
  - Added `DataRefreshSummary` and `DataSourceRefresh` types
- [x] **WebSocket Handlers** (`src/server/handlers/inspect.rs`):
  - `inspect.query` - Unified query handler with auto-refresh and progress notifications
  - `inspect.refresh` - Manual data refresh handler
  - `InspectDataRefreshProgress` progress notification type
  - Handlers registered in router
- [x] **M6**: Old modules/commands removed
  - `as2org` module removed from database and lens
  - `whois` CLI command removed
  - `pfx2as` CLI command removed
  - Note: `as2rel` CLI command kept as standalone (by design decision)
- [x] **M7**: Pfx2as and RPKI lens refactoring (architecture consistency)
  - [x] **1.1**: Create `Pfx2asLens` struct that wraps `Pfx2asRepository`
    - `src/lens/pfx2as/mod.rs` now has full `Pfx2asLens` struct
    - Implements: `lookup()`, `lookup_exact()`, `lookup_longest()`, `lookup_covering()`, `lookup_covered()`
    - Implements: `get_prefixes_for_asn()`, `needs_refresh()`, `refresh()`, `get_metadata()`
    - Includes formatting methods for output
  - [x] **1.2**: Refactor `RpkiLens` to use `RpkiRepository` for caching
    - `RpkiLens` now takes `&MonocleDatabase` reference
    - Uses `RpkiRepository` for current data operations (cache-based)
    - Uses bgpkit-commons directly only for historical queries (with date parameter)
    - `refresh()` method loads from Cloudflare and stores in repository
  - [x] **1.3**: Move validation logic from `RpkiRepository` to `RpkiLens`
    - `validate()` method now in `RpkiLens` (lens layer)
    - Returns `RpkiValidationResult` with state, reason, and covering ROAs
    - Repository retains only data access methods (`get_covering_roas()`, etc.)
    - CLI updated to use lens for validation
- [ ] **M8**: Database structure reorganization (optional, low priority)
  - Restructure to `database/repositories/` pattern
- [x] **M9**: Documentation updates
  - [x] Update `ARCHITECTURE.md` - now reflects current state with layering rules, updated directory structure, key modules documentation
  - [ ] Update `src/database/README.md` (optional)
  - [ ] Update `src/lens/README.md` (optional)

### WebSocket Server Refactor Status

See `src/server/REFACTOR_PLAN.md` for full details. Summary:

**Completed:**
- [x] Phase A: `WsOpSink` terminal-guarded sink wrapper
- [x] Phase B: `op_id` systematic enforcement (streaming methods get op_id, non-streaming do not)
- [x] Phase C: `WsSink` reduced to minimal transport-only API (`send_envelope`, `send_message_raw`)
- [x] Phase D: `WsContext` is resource-only (no transport policy fields)
- [x] Phase D: Connection lifecycle enforced in `handle_socket`:
  - Max message size check
  - Periodic ping keepalive
  - Idle timeout / connection timeout
- [x] Phase D: Concurrency limits wired (`ServerConfig.max_concurrent_ops` → `OperationRegistry`)
- [x] Phase E: `protocol.rs` contains only core protocol types (no `Pagination`/`QueryFilters`)
- [x] Phase F: `OperationRegistry` with O(1) concurrency check and cleanup methods

**Remaining (minor):**
- [ ] Wire up periodic cleanup task for `OperationRegistry` (cleanup method exists but not called periodically)

---

## Notes

### Data Source URL

```
http://spaces.bgpkit.org/broker/asninfo.jsonl
```

### Example