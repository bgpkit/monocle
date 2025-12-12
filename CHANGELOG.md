# Changelog

All notable changes to this project will be documented in this file.

## Unreleased

### RPKI Command Revisions

* **Removed `list` subcommand**: The `rpki list` command was a duplicate of `rpki roas` and has been removed
* **Renamed `check` to `validate`**: The `rpki check` command is now `rpki validate`
  * Now takes two positional arguments (prefix and ASN) in any order
  * Automatically detects which argument is the prefix and which is the ASN
  * Returns an error if both resources are the same type or if parsing fails
* **Updated `roas` subcommand**: Now accepts multiple positional resource arguments
  * Resources (prefixes or ASNs) are auto-detected from the input
  * Results are the union of all matching ROAs (deduplicated)
  * If no resources specified, returns all ROAs
* **Added data source display**: All RPKI commands now display the data source via `eprintln` at the top of output
  * Current data always uses Cloudflare's rpki.json endpoint
  * Historical data uses the specified source (RIPE or RPKIviews)
* **Fixed markdown table formatting**: Removed line wrapping in markdown output for ASPAs to comply with markdown table grammar

### Progress Tracking for GUI Support

* **Added progress tracking for parse operations**: `ParseLens` now supports callback-based progress reporting
  * `ParseProgress` enum with `Started`, `Update`, and `Completed` variants
  * Progress updates emitted every 10,000 messages processed
  * Includes processing rate (messages/second) and elapsed time
  * New methods: `parse_with_progress()` and `parse_with_handler()`

* **Added progress tracking for search operations**: `SearchLens` now supports callback-based progress reporting
  * `SearchProgress` enum with variants: `QueryingBroker`, `FilesFound`, `FileStarted`, `FileCompleted`, `ProgressUpdate`, `Completed`
  * Reports percentage completion based on files processed
  * Includes ETA estimation and per-file success/failure status
  * New methods: `search_with_progress()` and `search_and_collect()`

* **Thread-safe callbacks**: Progress callbacks are `Arc<dyn Fn(...) + Send + Sync>` for safe use in parallel processing

* **JSON-serializable progress types**: All progress types derive `Serialize`/`Deserialize` for easy GUI communication

### Removed Features

* **Removed server module**: The web API and WebSocket server implementation has been removed to keep focus on library-level functionality
  * Removed `src/server/` module and all submodules
  * Removed `server` feature from Cargo.toml
  * Removed server-related dependencies: axum, tokio, tokio-util, tower, tower-http, tokio-tungstenite, futures, uuid
  * Removed `WEB_API_DESIGN.md`
  * Removed `examples/run_server.rs`

### Unified Output Format

* **Global `--format` option**: All commands now support a unified output format option (long form only, no `-f` short form to avoid conflicts with subcommand flags like `whois -f`)
  * `table` (default): Pretty table with rounded borders
  * `markdown` / `md`: Markdown table format
  * `json`: Compact JSON (single line)
  * `json-pretty`: Pretty-printed JSON with indentation (same as `--json` flag)
  * `json-line` / `jsonl` / `ndjson`: JSON Lines format (one JSON object per line, for streaming)
  * `psv`: Pipe-separated values with header row

* **All informational messages now go to stderr**: Debug messages, progress updates, and explanatory text are now printed to stderr instead of stdout, enabling clean piping of data
  * Examples: "Updating AS2org data...", "Found 4407 ROAs (current data)", explanation text for as2rel
  * This allows: `monocle rpki roas --origin 13335 -f json | jq '.[0]'`

* **Removed per-command format flags**: The following flags have been removed in favor of the global `--format` option:
  * `--pretty` flag from `whois`, `as2rel`, `search`, `parse` commands
  * `--psv` / `-P` flag from `whois` command
  * Local `--json` flags (global `--json` still works as shortcut for `--format json-pretty`)

### New Features

* **New `as2rel` command**: AS-level relationship lookup between ASNs
  * Query relationships for one or two ASNs from BGPKIT's AS relationship data
  * Data source: `https://data.bgpkit.com/as2rel/as2rel-latest.json.bz2`
  * Output columns:
    - `connected`: Percentage of peers that see any connection between asn1 and asn2
    - `peer`: Percentage seeing pure peering (connected - as1_upstream - as2_upstream)
    - `as1_upstream`: Percentage of peers that see asn1 as upstream of asn2
    - `as2_upstream`: Percentage of peers that see asn2 as upstream of asn1
  * Local SQLite caching with automatic updates when data is older than 7 days
  * `--update`: Force update the local database
  * `--update-with <PATH>`: Update with a custom data file (local path or URL)
  * `--no-explain`: Hide the explanation text in table output
  * `--sort-by-asn`: Sort results by ASN2 ascending (default: sort by connected % descending)
  * `--show-name`: Show organization name for ASN2 (truncated to 20 chars)
  * `--show-full-name`: Show full organization name without truncation

* **New `config` command**: Show monocle configuration and data paths
  * Displays config file location and data directory
  * Shows SQLite database status, size, and record counts
  * `--verbose` flag lists all files in the data directory with sizes and modification times
  * Supports all output formats via `--format` option

* **New `rpki roas` command**: List ROAs from RPKI data (current or historical)
  * `--origin <ASN>`: Filter by origin ASN
  * `--prefix <PREFIX>`: Filter by prefix
  * `--date <YYYY-MM-DD>`: Load historical data for a specific date
  * `--source <ripe|rpkiviews>`: Select historical data source (default: ripe)
  * `--collector <soborost|massars|attn|kerfuffle>`: Select RPKIviews collector

* **New `rpki aspas` command**: List ASPAs from RPKI data (current or historical)
  * `--customer <ASN>`: Filter by customer ASN
  * `--provider <ASN>`: Filter by provider ASN
  * `--date <YYYY-MM-DD>`: Load historical data for a specific date
  * `--source <ripe|rpkiviews>`: Select historical data source (default: ripe)
  * `--collector <soborost|massars|attn|kerfuffle>`: Select RPKIviews collector

### Bug Fixes

* **Fixed AS2Rel data loading**: Removed incorrect serde attribute that caused JSON deserialization to fail with "missing field `relationship`" error
* **Fixed AS2Rel duplicate rows**: Fixed aggregation logic that showed multiple rows for the same ASN pair
* **Fixed AS2Rel percentage calculation**: 
  * `connected` now correctly uses `rel=0` peer count (total peers seeing any connection)
  * `as1_upstream` / `as2_upstream` are subsets from `rel=1` / `rel=-1` records
  * `peer` is calculated as `connected - as1_upstream - as2_upstream`
  * All percentages divided by `max_peers_count`
* **Optimized AS2Rel queries**: Replaced inefficient two-step aggregation with SQL JOINs and GROUP BY

### Improvements

* **Name truncation in tables**: Long names (AS names, org names) are now truncated to 20 characters in table output
  * New `--show-full-name` flag available on `whois` and `as2rel` commands to disable truncation
  * JSON output never truncates names

* **as2org data source**: Replaced custom CAIDA as2org file parsing with `bgpkit-commons` asinfo module
  * SQLite caching is preserved for fast repeated queries
  * `whois --update` now reloads data from `bgpkit-commons`

* **Table formatting**: ASPA table output now wraps long provider lists at 60 characters for better readability

### Breaking Changes

* **Removed `broker` command**: Use `search --broker-files` instead to list matching MRT files
* **Removed `radar` command**: Users can access Cloudflare Radar data directly via their API
* **Removed `rpki read-roa` and `rpki read-aspa` commands**: Replaced with `rpki roas` and `rpki aspas`
* **Library API refactoring**: All public functions are now accessed through lens structs
* **Default output format**: Changed from markdown to table (pretty borders)

### Code Improvements

* Refactored CLI command modules for better code organization
* **Lens-based architecture**: All functionality accessed through lens structs
* Added `lens/utils.rs` with `OutputFormat` enum and `truncate_name` helper

### Dependencies

* Added `bgpkit-commons` with features: `asinfo`, `rpki`, `countries`
* Removed `rpki` crate dependency

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
