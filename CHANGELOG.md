# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

This is a major release with significant architectural changes, new commands, and breaking changes.

### New Commands

#### `monocle server` - WebSocket API Server

Start a WebSocket server for programmatic access to monocle functionality:
* `monocle server`: Start server on default address (127.0.0.1:8080)
* `monocle server --address 0.0.0.0 --port 3000`: Custom bind address and port
* WebSocket endpoint: `ws://<address>:<port>/ws`
* Health check endpoint: `http://<address>:<port>/health`

**Features:**
* JSON-RPC style request/response protocol with streaming support
* Operation cancellation via `op_id` for long-running tasks
* Progress reporting for parse and search operations
* DB-first policy: queries read from local SQLite cache

**Available methods:**
* `system.info`, `system.methods` - Server introspection
* `time.parse` - Time string parsing
* `ip.lookup`, `ip.public` - IP information lookup
* `rpki.validate`, `rpki.roas`, `rpki.aspas` - RPKI operations
* `as2org.search`, `as2org.bootstrap` - AS-to-Organization mappings
* `as2rel.search`, `as2rel.relationship`, `as2rel.update` - AS relationships
* `pfx2as.lookup` - Prefix-to-ASN mapping
* `country.lookup` - Country code/name lookup
* `parse.start`, `parse.cancel` - MRT file parsing (streaming)
* `search.start`, `search.cancel` - BGP message search (streaming)
* `database.status`, `database.refresh` - Database management

#### `monocle database` - Database Management

Consolidated database management with subcommands:
* `monocle database` (or `monocle database status`): Show database status including record counts, last update times, and cache settings
* `monocle database refresh <source>`: Refresh a specific data source (as2org, as2rel, rpki, pfx2as-cache)
* `monocle database refresh --all`: Refresh all data sources at once
* `monocle database backup <dest>`: Backup the SQLite database to a destination path
* `monocle database clear <source>`: Clear a specific data source (with confirmation prompt)
* `monocle database sources`: List available data sources with their status and last update time

#### `monocle config` - Configuration Display

Show monocle configuration and data paths:
* Displays config file location and data directory
* Shows SQLite database status, size, and record counts
* `--verbose` flag lists all files in the data directory with sizes and modification times

#### `monocle as2rel` - AS Relationship Lookup

Query AS-level relationships between ASNs from BGPKIT's AS relationship data:
* Query relationships for one or two ASNs
* Output columns: connected, peer, as1_upstream, as2_upstream percentages
* Local SQLite caching with automatic updates when data is older than 7 days
* `--show-name` / `--show-full-name`: Show organization name for ASN2
* `--sort-by-asn`: Sort results by ASN2 ascending (default: sort by connected % descending)

### Pfx2as Improvements

* **Pfx2as data now stored in SQLite**: Prefix-to-ASN mappings cached locally for fast queries
  * IP prefixes stored as 16-byte start/end address pairs for efficient range lookups
  * Supports multiple query modes: exact, longest prefix match, covering (supernets), covered (subnets)
  * Cache expires after 24 hours and automatically refreshes
  * Use `database refresh pfx2as` or WebSocket `database.refresh` with `source: "pfx2as"` to populate
  * Backward compatible with file-based cache for existing installations

### RPKI Improvements

* **RPKI data now stored in SQLite**: ROAs and ASPAs cached locally for fast queries
  * IP prefixes stored as 16-byte start/end address pairs for efficient range lookups
  * Cache expires after 24 hours and automatically refreshes
  * Use `--refresh` / `-r` flag to force a cache refresh
* **Local RPKI validation**: Implements RFC 6811 validation logic locally instead of calling external API
* **Renamed `check` to `validate`**: Now takes two positional arguments (prefix and ASN) in any order
* **Updated `roas` subcommand**: Now accepts multiple positional resource arguments (auto-detected)
* **Updated `aspas` subcommand**: Current data now uses SQLite cache
* **Removed `list` subcommand**: Use `rpki roas` instead
* **Removed `summary` subcommand**: Cloudflare GraphQL API no longer available

### Progress Tracking (Library Feature)

* **Parse operations**: `ParseLens` supports callback-based progress reporting
  * `ParseProgress` enum with `Started`, `Update`, and `Completed` variants
  * New methods: `parse_with_progress()` and `parse_with_handler()`
* **Search operations**: `SearchLens` supports callback-based progress reporting
  * `SearchProgress` enum with file-level progress tracking
  * New methods: `search_with_progress()` and `search_and_collect()`
* **Thread-safe callbacks**: `Arc<dyn Fn(...) + Send + Sync>` for parallel processing
* **JSON-serializable**: All progress types derive `Serialize`/`Deserialize`

### Unified Output Format

* **Global `--format` option**: All commands support unified output formats
  * `table` (default): Pretty table with rounded borders
  * `markdown` / `md`: Markdown table format
  * `json`: Compact JSON (single line)
  * `json-pretty`: Pretty-printed JSON (same as `--json` flag)
  * `json-line` / `jsonl` / `ndjson`: JSON Lines format
  * `psv`: Pipe-separated values with header row
* **All informational messages go to stderr**: Enables clean piping of data
* **Removed per-command format flags**: `--pretty`, `--psv` flags removed in favor of `--format`

### Improvements

* **Name truncation in tables**: Long names truncated to 20 characters (use `--show-full-name` to disable)
* **as2org data source**: Now uses `bgpkit-commons` asinfo module instead of custom CAIDA parsing
* **Database performance**: Batch insert operations use optimized SQLite settings
* **Table formatting**: ASPA table output wraps long provider lists at 60 characters

### Bug Fixes

* Fixed AS2Rel data loading (incorrect serde attribute)
* Fixed AS2Rel duplicate rows and percentage calculation
* Optimized AS2Rel queries with SQL JOINs

### Breaking Changes

* **Removed `broker` command**: Use `search --broker-files` instead
* **Removed `radar` command**: Access Cloudflare Radar directly via their API
* **Removed `rpki list` and `rpki summary` commands**: Use `rpki roas` instead
* **Renamed `rpki check` to `rpki validate`**
* **Removed server module**: Web API/WebSocket server removed to focus on library functionality
* **Library API refactoring**: All public functions now accessed through lens structs
* **Default output format**: Changed from markdown to table (pretty borders)

### Code Improvements

* **Lens-based architecture**: All functionality accessed through lens structs
* Refactored CLI command modules for better code organization
* Added `lens/utils.rs` with `OutputFormat` enum and `truncate_name` helper

### Dependencies

* Added `bgpkit-commons` with features: `asinfo`, `rpki`, `countries`
* Removed `rpki` crate dependency
* Removed server-related dependencies: axum, tokio, tower, etc.

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
