# Changelog

All notable changes to this project will be documented in this file.

## Unreleased changes

### Breaking Changes

* **CLI flag renamed**: `--no-refresh` renamed to `--no-update` for consistency with "update" terminology
  * Old: `monocle --no-refresh <command>`
  * New: `monocle --no-update <command>`
  
* **Config subcommands renamed**: Removed `db-` prefix from config subcommands for cleaner syntax
  * `monocle config db-refresh` → `monocle config update`
  * `monocle config db-backup` → `monocle config backup`
  * `monocle config db-sources` → `monocle config sources`
  
* **Configurable TTL for all data sources**: All data sources now have configurable cache TTL with 7-day default
  * Added `asinfo_cache_ttl_secs` config option (default: 7 days)
  * Added `as2rel_cache_ttl_secs` config option (default: 7 days)
  * Changed `rpki_cache_ttl_secs` default from 1 hour to 7 days
  * Changed `pfx2as_cache_ttl_secs` default from 24 hours to 7 days
  * Configure via `~/.monocle/monocle.toml` or environment variables (`MONOCLE_ASINFO_CACHE_TTL_SECS`, etc.)

* **Simplified feature flags**: Replaced 6-tier feature system with 3 clear features
  * Old: `database`, `lens-core`, `lens-bgpkit`, `lens-full`, `display`, `cli`
  * New: `lib`, `server`, `cli`
  * Quick guide:
    - Need CLI binary? Use `cli` (includes everything)
    - Need WebSocket server without CLI? Use `server` (includes lib)
    - Need only library/data access? Use `lib` (database + all lenses + display)
  * Display (tabled) now always included with `lib` feature

### New Features

* **`monocle config sources`**: Shows staleness status based on TTL for all data sources
  * "Stale" column shows whether each source needs updating based on its configured TTL
  * Configuration section shows current TTL values for all sources

### Bug Fixes

* Avoid creating a new SQLite database when `monocle config sources` inspects staleness

### Code Improvements

* **Data refresh logging**: CLI now shows specific reason for data refresh ("data is empty" vs "data is outdated") instead of generic "empty or outdated" message

### Breaking Changes

* **ParseFilters**: Changed filter field types to support multiple values with OR logic
  * `origin_asn`: `Option<u32>` → `Vec<String>`
  * `prefix`: `Option<String>` → `Vec<String>`
  * `peer_asn`: `Option<u32>` → `Vec<String>`
  * Empty `Vec` is equivalent to no filter (previous `None`)
  * Values can be prefixed with `!` for negation (exclusion)
  * Library users will need to update code: `Some(13335)` → `vec!["13335".to_string()]`

### New Features

* **RTR protocol support**: Added support for fetching ROAs via RTR (RPKI-to-Router) protocol
  * Configure RTR endpoint in `~/.monocle/monocle.toml`:
    ```toml
    rpki_rtr_host = "rtr.rpki.cloudflare.com"
    rpki_rtr_port = 8282
    rpki_rtr_timeout_secs = 10
    rpki_rtr_no_fallback = false
    ```
  * Or use environment variables: `MONOCLE_RPKI_RTR_HOST`, `MONOCLE_RPKI_RTR_PORT`, `MONOCLE_RPKI_RTR_TIMEOUT_SECS`, `MONOCLE_RPKI_RTR_NO_FALLBACK`
  * Or use CLI flag for one-time override: `monocle config db-refresh --rpki --rtr-endpoint rtr.rpki.cloudflare.com:8282`
  * ROAs are fetched via RTR, ASPAs always from Cloudflare (RTR v1 per RFC 8210 doesn't support ASPA)
  * Automatic fallback to Cloudflare if RTR connection fails, with warning message (set `rpki_rtr_no_fallback = true` to disable fallback and error out instead)
  * Connection timeout defaults to 10 seconds
  * Supports RTR protocol version negotiation (v1 with v0 fallback)
* **`--cache-dir`**: Added local caching support to the `search` command
  * Download MRT files to a local directory before parsing
  * Files are cached as `{cache-dir}/{collector}/{path}` (e.g., `cache/rrc00/2024.01/updates.20240101.0000.gz`)
  * Cached files are reused on subsequent runs, avoiding redundant downloads
  * Uses `.partial` extension during downloads to handle interrupted transfers
  * Cache directory access is validated upfront before processing begins
  * **Broker query caching**: When `--cache-dir` is specified, broker API query results are cached in SQLite
    * Cache stored at `{cache-dir}/broker-cache.sqlite3`
    * Only queries with end time >2 hours in the past are cached (recent data may still change)
    * Subsequent identical queries use cached results, enabling offline operation
    * Tested: run search once, disable network, run same search again - results returned from cache
  * Example: `monocle search -t 2024-01-01 -d 1h --cache-dir /tmp/mrt-cache`
* **Multi-value filters**: `parse` and `search` commands now support filtering by multiple values with OR logic
  * Example: `-o 13335,15169,8075` matches elements from ANY of the specified origin ASNs
  * Example: `-p 1.1.1.0/24,8.8.8.0/24` matches ANY of the specified prefixes
  * Example: `-J 174,2914` matches elements from ANY of the specified peer ASNs
* **Negative filters**: Support for exclusion filters using `!` prefix
  * Example: `-o '!13335'` excludes elements from AS13335
  * Example: `-o '!13335,!15169'` excludes elements from AS13335 AND AS15169
  * Note: Cannot mix positive and negative values in the same filter
* Added validation for ASN format, prefix CIDR notation, and negation consistency
* **`--time-format`**: Added timestamp output format option to `parse` and `search` commands
  * `--time-format unix` (default): Output timestamps as Unix epoch (integer/float)
  * `--time-format rfc3339`: Output timestamps in ISO 8601/RFC3339 format (e.g., `2023-10-11T17:00:00+00:00`)
  * Applies to non-JSON output formats (table, psv, markdown)
  * JSON output always uses numeric Unix timestamps for backward compatibility
  * Example: `monocle parse file.mrt --time-format rfc3339`
  * Example: `monocle search -t 2024-01-01 -d 1h -p 1.1.1.0/24 --time-format rfc3339`

### Code Improvements

* **Feature gate cleanup**: Simplified feature gating for the `database` module
  * The entire `database` module is now gated at `lib.rs` level with `#[cfg(feature = "database")]`
  * Removed redundant feature gates from internal submodules
  * Added detailed feature documentation to `ARCHITECTURE.md` with use case scenarios

* Added `--fields` (`-f`) option to `parse` and `search` commands for selecting output fields ([#99](https://github.com/bgpkit/monocle/issues/99), [#101](https://github.com/bgpkit/monocle/pull/101))
  * Choose which columns to display with comma-separated field names
  * Available fields: `type`, `timestamp`, `peer_ip`, `peer_asn`, `prefix`, `as_path`, `origin`, `next_hop`, `local_pref`, `med`, `communities`, `atomic`, `aggr_asn`, `aggr_ip`, `collector`
  * Parse command defaults exclude `collector` field (not applicable)
  * Search command defaults include `collector` field
  * Example: `monocle search -t 2024-01-01 -d 1h -f prefix,as_path,collector`

* Added proper table formatting with borders using `tabled` crate for `--format table` ([#99](https://github.com/bgpkit/monocle/issues/99), [#101](https://github.com/bgpkit/monocle/pull/101))
  * Table output now uses rounded borders instead of tab-separated values
  * Markdown format includes proper header row with separator

* Added `--order-by` and `--order` parameters to `parse` and `search` commands ([#98](https://github.com/bgpkit/monocle/issues/98))
  * Sort output by: `timestamp`, `prefix`, `peer_ip`, `peer_asn`, `as_path`, or `next_hop`
  * Direction: `asc` (ascending, default) or `desc` (descending)
  * When ordering is specified, output is buffered and sorted before display
  * Example: `monocle parse file.mrt --order-by timestamp --order asc`
  * Example: `monocle search -t 2024-01-01 -d 1h -p 1.1.1.0/24 --order-by timestamp --order desc`

## v1.0.2 - 2025-12-18

### New Features

* Added new `monocle pfx2as` command for prefix-to-ASN mapping lookups
  * **Search by prefix**: Query prefixes to find their origin ASNs
    * Example: `monocle pfx2as 1.1.1.0/24`
  * **Search by ASN**: Query an ASN to find all its announced prefixes
    * Example: `monocle pfx2as 13335` or `monocle pfx2as AS13335`
  * **RPKI validation**: Shows RPKI validation status (valid/invalid/not_found) for each prefix-ASN pair
  * **`--show-name`**: Display AS organization name for each origin ASN
  * **`--include-sub`**: Include sub-prefixes (more specific) in results
    * Example: `monocle pfx2as 8.0.0.0/8 --include-sub --limit 20`
  * **`--include-super`**: Include super-prefixes (less specific) in results
    * Example: `monocle pfx2as 1.1.1.0/24 --include-super`
  * **`--limit`**: Limit the number of results
  * Supports all standard output formats (`--format table/json/psv/etc.`)

* Enhanced `monocle as2rel` command with advanced filtering and multi-ASN support
  * **`--min-visibility <PERCENT>`**: Filter results by minimum visibility percentage (0-100)
    * Available for all as2rel queries
    * Filters out relationships seen by fewer than the specified percentage of peers
  * **`--single-homed`**: Find ASNs that are single-homed to the queried ASN
    * Shows only ASNs where the queried ASN is their ONLY upstream provider
    * Useful for identifying customers with no redundancy
    * Example: `monocle as2rel 2914 --single-homed`
  * **`--is-upstream`**: Filter to show only downstream customers of the queried ASN
    * Shows relationships where the queried ASN is the upstream (provider)
  * **`--is-downstream`**: Filter to show only upstream providers of the queried ASN
    * Shows relationships where the queried ASN is a downstream (customer)
  * **`--is-peer`**: Filter to show only peer relationships (settlement-free interconnection)
  * **Multi-ASN support**: Query relationships among multiple ASNs at once
    * When more than two ASNs are provided, shows all pair combinations
    * Results sorted by asn1, with asn1 < asn2 for each pair
    * Example: `monocle as2rel 174 2914 3356` shows all three pair relationships

* Added global `--no-refresh` flag to disable automatic data refresh
  * Use `monocle --no-refresh <command>` to skip all automatic data loading/refresh
  * Useful when you want to use existing cached data only
  * Shows warnings when data is missing or stale instead of auto-refreshing

* Added Docker support with multi-stage build
  * `Dockerfile` with two-stage build process for minimal image size (~176MB final image)
  * Uses Rust 1.92 and Debian trixie-slim as runtime base
  * `docker-compose.yml` for easy container orchestration
  * `.dockerignore` to optimize build context
  * Runs as non-root user for security
  * Persistent data volume at `/data`
  * Default server mode with port 8080 exposed

### Bug Fixes

* Fixed "database is locked" error in `monocle config db-refresh` command (Issue #90)
  * The `do_refresh` function was opening redundant database connections for ASInfo and AS2Rel data sources
  * Now correctly uses the already-passed database connection parameter

### Improvements

* Added visual `...` row indicator in tables when results are truncated
  * Search results table now shows a `...` row when more matches exist
  * RPKI ROA tables show truncation indicator
  * Announced prefixes table shows truncation indicator
  * Connectivity section (upstreams/peers/downstreams) tables show truncation indicator
  * Makes it much more visible that additional results are available

* Added `[monocle]` prefix to all auto-refresh log messages
  * Makes it easier to distinguish monocle's internal logging from main output
  * Especially useful when refresh operations run automatically during commands

* RPKI ASPA command now ensures ASInfo data is available for AS name enrichment
  * Automatically loads ASInfo data before showing ASPA output
  * AS names and countries are displayed in ASPA results

* Added comprehensive tests for database initialization with mock data
  * Tests for all repositories being accessible after initialization
  * Tests for schema version verification
  * Tests for RPKI and Pfx2as mock data storage/retrieval

## v1.0.1 - 2025-12-17

### Bug Fixes

* Fixed cross-compilation issue on Linux platforms caused by OpenSSL dependency
  * Updated `bgpkit-commons` to v0.10.1 which uses `rustls` instead of `native-tls`
  * All TLS operations now use `rustls`, eliminating the need for OpenSSL development packages

## v1.0.0 - 2025-12-18

This is a major release with significant architectural changes, new commands, and breaking changes.

### Breaking Changes

* **Command Removals & Renames**:
  * Removed `broker` command (use `search --broker-files` instead).
  * Removed `radar` command (access Cloudflare Radar directly via their API).
  * Removed `rpki list` and `rpki summary` commands (use `rpki roas` instead).
  * Renamed `rpki check` to `rpki validate`.
  * Renamed `whois` to `inspect` (unified AS/prefix lookup command).
* **Library API**:
  * All public functions are now accessed through lens structs (e.g., `InspectLens`, `RpkiLens`).
* **Output**:
  * Default output format changed from markdown to table (pretty borders).

### New Features

#### New Commands
* **`monocle inspect`**: Unified AS/prefix information lookup.
  * Replaces `whois` and `pfx2as`.
  * Auto-detects query type (ASN, prefix, IP address, or name).
  * Combines data from ASInfo, AS2Rel, RPKI, and Pfx2as.
* **`monocle server`**: WebSocket API Server.
  * JSON-RPC style protocol with streaming support.
  * Endpoints for all major monocle operations.
* **`monocle config`**: Consolidated configuration and database management.
  * Manage data sources, refresh data, and backup database.
* **`monocle as2rel`**: AS Relationship lookup.
  * Query relationships, peers, and upstreams.

#### ASPA Support
* **Enrichment**:
  * Enriched customer/provider names and countries via SQL JOINs.
  * Unified provider structure in JSON output (`providers` array with `{asn, name}` objects).

#### Core Enhancements
* **Unified Output Format**: Global `--format` option for all commands (`table`, `markdown`, `json`, `json-pretty`, `json-line`, `psv`).
* **SQLite Integration**:
  * **ASInfo**: Unified AS information stored in SQLite (replaces as2org).
  * **Pfx2as**: Prefix-to-ASN mappings cached in SQLite for fast range lookups.
  * **RPKI**: ROAs and ASPAs cached in SQLite.
* **Progress Tracking**: Library support for callback-based progress reporting in `ParseLens` and `SearchLens`.
* **Feature Flags**: Reorganized into tiers (`database`, `lens-core`, `lens-bgpkit`, `lens-full`, `display`, `cli`).

### Improvements

* **`monocle inspect`**:
  * Progress messages during data loading.
  * Improved output formatting with section headers.
  * Performance optimization (lazy loading of data sources).
* **General**:
  * **Name truncation**: Long names in tables are truncated to 20 chars (disable with `--show-full-name`).
  * **Database performance**: Optimized batch insert operations.
  * **Broken pipe handling**: Graceful exit when piping output (e.g., to `head`).

### Bug Fixes

* Handle SIGPIPE gracefully to prevent panics when piping output.

### Documentation

* Added WebSocket server documentation.
* Updated all documentation references and examples.

### Code Improvements

* **Lens-based architecture**: Centralized logic in `src/lens/`.
* **Refactoring**: Improved CLI command organization.
* **Examples**: Added comprehensive examples for all feature tiers.

### Dependencies

* Added `bgpkit-commons` (asinfo, rpki, countries).
* Added server dependencies (`axum`, `tokio`, etc.).
* Added `libc` (used for SIGPIPE handling on Unix systems).
* Removed `rpki` crate.

## v0.9.1 - 2025-11-05

### Maintenance

* update dependencies
    * `oneio` -> v0.20.0
    * `bgpkit-parser` -> v0.12.0
    * `bgpkit-broker` -> v0.9.1

### Bug fixes

* Fix an issue where monocle fails to locate the latest CAIDA as2org dataset file

## v0.9.0 - 2025-09-04

### New features

* Added retry mechanism for failed search operations with exponential backoff
* Implemented real-time success/failure progress tracking during search
* Added paginated search processing for large time ranges to handle memory efficiently

### Performance improvements

* Database bootstrap performance improvements
    * Added proper transaction management for bulk inserts
    * Replaced string-based SQL with prepared statements
    * Added database indexes for common query patterns
    * Enabled SQLite performance optimizations (WAL mode, cache tuning)
    * **Impact**: BGP data insertion ~10x faster, as2org bootstrap ~100x faster (3+ minutes → 1-2 seconds)

### Bug fixes

* Fixed network error handling in multi-file processing to prevent thread panics

### Code improvements

* Replaced unwrap/expect calls with proper error handling
* Added clippy lints to deny unsafe unwrap_used and expect_used patterns
* Updated CI workflow to include formatting and clippy checks
* Enhanced database operations with proper Result types
* Improved RPKI validator error handling

## v0.8.0 - 2025-03-04

### New subcommand

* added `monocle pfx2as` subcommand to allow bulk prefix-to-asn mapping using BGPKIT dataset
    * it takes a list of prefixes or prefix files (one prefix per line)

Example:

```bash
monocle pfx2as 1.1.1.0/24 8.8.8.0/24 --json
[
  {
    "origin": 13335,
    "prefix": "1.1.1.0/24"
  },
  {
    "origin": 15169,
    "prefix": "8.8.8.0/24"
  }
]
```

### Maintenance

* update dependencies
    * note that we upgraded to `bgpkit-parser` v0.11 and community values are now without prefixes such as `lg:` `ecv6`

## v0.7.2 - 2025-01-08

### Improvements

* support searching data from RIB dumps by specifying `--dump-type` argument
    * `--dump-type updates`: search updates files only
    * `--dump-type rib`: search RIB files only
    * `--dump-type rib-updates`: search RIB dump and updates
* improved internal handling of filters and time string parsing
* improved documentation

## v0.7.1 - 2024-12-27

### Maintenance

* add back `Cargo.lock` file to reproducible builds

## v0.7.0 - 2024-12-27

### New Features

#### `monocle ip` command

Add a new `monocle ip` command to retrieve information for the current IP of the machine or any specified IP address,
including location, network (ASN, network name) and the covering IP prefix of the given IP address.

The command triggers an API call to [BGPKIT API][bgpkit-api],
and it retrieves the information based on the incoming requester IP address with additional BGP information for the
enclosing IP prefixes.

[bgpkit-api]: https://api.bgpkit.com/docs

```text
➜  ~ monocle ip
+----------+--------------------------+
| ip       | 104.48.0.0               |
+----------+--------------------------+
| location | US                       |
+----------+---------+----------------+
| network  | asn     | 7018           |
|          +---------+----------------+
|          | country | US             |
|          +---------+----------------+
|          | name    | AT&T US - 7018 |
|          +---------+----------------+
|          | prefix  | 104.48.0.0/12  |
|          +---------+----------------+
|          | rpki    | valid          |
+----------+---------+----------------+

➜  ~ monocle ip 1.1.1.1
+----------+----------------------+
| ip       | 1.1.1.1              |
+----------+----------------------+
| location | US                   |
+----------+---------+------------+
| network  | asn     | 13335      |
|          +---------+------------+
|          | country | US         |
|          +---------+------------+
|          | name    | Cloudflare |
|          +---------+------------+
|          | prefix  | 1.1.1.0/24 |
|          +---------+------------+
|          | rpki    | valid      |
+----------+---------+------------+

➜  ~ monocle ip 1.1.1.1 --json
{
  "ip": "1.1.1.1",
  "location": "US",
  "network": {
    "asn": 13335,
    "country": "US",
    "name": "Cloudflare",
    "prefix": "1.1.1.0/24",
    "rpki": "valid"
  }
}
```

#### MRT export for `monocle parse` command

The `monocle parse` command now supports
exporting filtered BGP messages into MRT files by supplying an MRT file path with `--mrt-path` argument.

#### Improved time string parsing

The parsing of time strings in `monocle time` and `monocle search` now utilizes [`dateparser`][dateparser] for natural
date strings like `May 6 at 9:24 PM` or `2019-11-29 08:08-08`.
It now also allows specifying a `duration` like `1h` or `"2 hours"` to replace `--start-ts` or `--end-ts`.

### Other improvements

* Updated documentation for various commands
* Cleaned up dependencies in the `Cargo.toml` file

[dateparser]: https://github.com/waltzofpearls/dateparser

## v0.6.2 - 2024-10-28

### Dependency updates

* `bgpkit-broker` to v0.7.0 -> v0.7.5
* `bgpkit-parser` to v0.10.9 -> v0.10.11

`bgpkit-parser` version `v0.10.11` fixes the improper handling of `AS23456` (`AS_TRANS`). If you previously see
`AS23456` incorrectly showing on the path, it should no-longer showing up after this patchshould no-longer show up after
this patch.

### Fixes

* fixed a bug where `psv` format output does not actually print out content.

## v0.6.1 - 2024-08-05

This is a maintenance release that updates the following dependencies.

* `bgpkit-broker` to v0.7.0 -> v0.7.1
* `bgpkit-parser` to v0.10.9 -> v0.10.10
* `oneio` to v0.16.7 -> v0.17.0

With the updated dependencies, `monocle` now supports using `ONEIO_ACCEPT_INVALID_CERTS=true` env variable
to run search within a network that uses self-signed certificates.

## v0.6.0 - 2024-06-28

### Highlights

* `monocle time` now supports querying multiple time strings in a single query
* `monocle search` with `--sqlite-path` now adds found messages to the progress bar during search
* `monocle search` now shows the collector IDs in the results, included in the plaintext, json output as well as the
  sqlite database
* `monocle search` now supports exporting to MRT files using `--mrt-path` parameter

## v0.5.5 - 2024-03-29

### Highlights

* update `bgpkit-parser` to v0.10.5 and `oneio` to v0.16.7
    * not depends on `lz` and `xz` features anymore
    * this change allows `monocle` to work on fresh systems with no xz library installed (e.g. more recent macOS)

## v0.5.4 - 2024-02-23

### Highlights

* update `bgpkit-parser` to v0.10.1, which includes a non-trivial performance boost for processing gzip compressed MRT
  files.
* added a new `--simple` option to `monocle time` command to allow simple time conversion, suitable for use in scripts.

## v0.5.3 - 2024-02-03

### Highlights

* remove openssl dependency, switching to rustls as TLS backend
* support installation via `cargo-binstall`

## v0.5.2 - 2023-12-18

* add GitHub actions config to build `monocle` binary for macOS (Universal), and linux (arm and amd64)
* add `vendored-openssl` optional feature flag to enable GitHub actions builds for different systems.
* move `monocle` binary to `bin` directory
* install `monocle` with `brew install bgpkit/tap/monocle`
