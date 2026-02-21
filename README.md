# Monocle

[![Rust](https://github.com/bgpkit/monocle/actions/workflows/rust.yml/badge.svg)](https://github.com/bgpkit/monocle/actions/workflows/rust.yml)
[![Crates.io](https://img.shields.io/crates/v/monocle)](https://crates.io/crates/monocle)
[![Docs.rs](https://docs.rs/monocle/badge.svg)](https://docs.rs/monocle)
[![License](https://img.shields.io/crates/l/monocle)](https://raw.githubusercontent.com/bgpkit/monocle/main/LICENSE)

See through all Border Gateway Protocol (BGP) data with a monocle.

![](https://spaces.bgpkit.org/assets/monocle/monocle-emoji.png)

## Table of Contents

- [Install](#install)
  - [Using `cargo`](#using-cargo)
  - [Using `homebrew` on macOS](#using-homebrew-on-macos)
  - [Using `cargo-binstall`](#using-cargo-binstall)
- [Library Usage](#library-usage)
- [Documentation](#documentation)
- [Usage](#usage)
  - [`monocle parse`](#monocle-parse)
    - [Output Format](#output-format)
  - [`monocle search`](#monocle-search)
  - [`monocle time`](#monocle-time)
  - [`monocle inspect`](#monocle-inspect)
  - [`monocle country`](#monocle-country)
  - [`monocle as2rel`](#monocle-as2rel)
  - [`monocle pfx2as`](#monocle-pfx2as)
  - [`monocle rpki`](#monocle-rpki)
    - [`monocle rpki validate`](#monocle-rpki-validate)
    - [`monocle rpki roas`](#monocle-rpki-roas)
    - [`monocle rpki aspas`](#monocle-rpki-aspas)
  - [`monocle ip`](#monocle-ip)
  - [`monocle config`](#monocle-config)
  - [`monocle server`](#monocle-server)

## Install

### Using `cargo`

```bash
cargo install monocle
```

### Using `homebrew` on macOS

```bash
brew install monocle
```

### Using [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall)

Install `cargo-binstall` first:

```bash
cargo install cargo-binstall
```

Then install `monocle` using `cargo binstall`

```bash
cargo binstall monocle
```

### Using Docker

Pull the pre-built image or build locally:

```bash
# Build the image locally
docker build -t bgpkit/monocle:latest .

# Or use docker compose
docker compose build
```

Run monocle commands:

```bash
# Show help
docker run --rm bgpkit/monocle:latest

# Run a command (e.g., inspect an ASN)
docker run --rm bgpkit/monocle:latest inspect 13335

# Run with persistent data directory
docker run --rm -v monocle-data:/data bgpkit/monocle:latest inspect 13335

# Start the WebSocket server
docker run --rm -p 8080:8080 -v monocle-data:/data bgpkit/monocle:latest server --address 0.0.0.0 --port 8080

# Using docker compose for server mode
docker compose up -d
```

## Library Usage

Monocle can also be used as a library in your Rust projects. Add it to your `Cargo.toml`:

```toml
[dependencies]
# Default: full CLI binary with all features
monocle = "1.1"

# Library only - all lenses and database operations
monocle = { version = "1.1", default-features = false, features = ["lib"] }

# Library + WebSocket server
monocle = { version = "1.1", default-features = false, features = ["server"] }
```

### Feature Tiers

Monocle uses a simplified feature system with three options:

| Feature | Description | Implies |
|---------|-------------|---------|
| `lib` | Complete library (database + all lenses + display) | - |
| `server` | WebSocket server for programmatic API access | `lib` |
| `cli` (default) | Full CLI binary with all functionality | `lib`, `server` |

### Documentation

The following documentation files are available in the repository:

| File | Description |
|------|-------------|
| [`README.md`](README.md) (this file) | User-facing CLI and library overview |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Overall project structure and design principles |
| [`DEVELOPMENT.md`](DEVELOPMENT.md) | Contributor guide for adding lenses and fixing bugs |
| [`AGENTS.md`](AGENTS.md) | AI coding agent guidelines and code style |
| [`CHANGELOG.md`](CHANGELOG.md) | Version history and breaking changes |
| [`src/server/README.md`](src/server/README.md) | WebSocket API specification |
| [`src/lens/README.md`](src/lens/README.md) | Lens module patterns and conventions |
| [`src/database/README.md`](src/database/README.md) | Database module overview |
| [`examples/README.md`](examples/README.md) | Usage examples by feature tier |

### Architecture

The library is organized into the following core modules:

- **`database`**: All database functionality (requires `lib` feature)
  - `core`: Connection management and schema definitions
  - `session`: One-time storage for search results
  - `monocle`: Main monocle database with ASInfo, AS2Rel, RPKI, and Pfx2as caching

- **`lens`**: High-level business logic (requires `lib` feature)
  - `time`: Time parsing and formatting lens
  - `country`: Country code/name lookup lens
  - `ip`: IP information lookup lens
  - `parse`: MRT file parsing lens with progress tracking
  - `search`: BGP message search lens with progress tracking
  - `rpki`: RPKI validation and data lens
  - `pfx2as`: Prefix-to-AS mapping types
  - `as2rel`: AS-level relationships lens
  - `inspect`: Unified AS/prefix inspection lens

- **`server`**: WebSocket API server (requires `server` feature)

For detailed architecture documentation, see [`ARCHITECTURE.md`](ARCHITECTURE.md).

### Example: Using Lenses

```rust
use monocle::database::MonocleDatabase;
use monocle::lens::inspect::{InspectLens, InspectQueryOptions};

fn main() -> anyhow::Result<()> {
    // Open the monocle database
    let db = MonocleDatabase::open_in_dir("~/.local/share/monocle")?;
    
    // Create a lens
    let lens = InspectLens::new(&db);
    
    // Query AS information
    let options = InspectQueryOptions::default();
    let results = lens.query_asn(13335, &options)?;
    
    println!("AS{}: {}", results.asn, results.name.unwrap_or_default());
    
    Ok(())
}
```

### Example: Parse MRT Files with Progress

```rust
use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    let lens = ParseLens::new();
    let filters = ParseFilters::default();
    
    // Define a progress callback
    let callback = Arc::new(|progress: ParseProgress| {
        match progress {
            ParseProgress::Started { file_path } => {
                eprintln!("Started parsing: {}", file_path);
            }
            ParseProgress::Update { messages_processed, rate, .. } => {
                eprintln!("Processed {} messages ({:.0} msg/s)", 
                    messages_processed, rate.unwrap_or(0.0));
            }
            ParseProgress::Completed { total_messages, duration_secs, .. } => {
                eprintln!("Completed: {} messages in {:.2}s", total_messages, duration_secs);
            }
        }
    });
    
    // Parse with progress tracking
    let elems = lens.parse_with_progress(
        &filters, 
        "path/to/file.mrt", 
        Some(callback)
    )?;
    
    for elem in elems {
        println!("{:?}", elem);
    }
    
    Ok(())
}
```

## Usage

Subcommands:

- `parse`: parse individual MRT files
- `search`: search for matching messages from all available public MRT files
- `server`: start a WebSocket server for programmatic access
- `inspect`: unified AS and prefix information lookup
- `country`: utility to look up country name and code
- `time`: utility to convert time between unix timestamp and RFC3339 string
- `as2rel`: AS-level relationship lookup between ASNs
- `pfx2as`: prefix-to-ASN mapping lookup with RPKI validation
- `rpki`: RPKI validation and ROA/ASPA listing
- `ip`: IP information lookup
- `config`: configuration display and database management (refresh, backup, sources)

### Global Options

All commands support the following global options:

- `--format <FORMAT>`: Output format (table, markdown, json, json-pretty, json-line, psv)
- `--json`: Shortcut for `--format json-pretty`
- `--debug`: Print debug information

Top-level help menu:

```text
➜  monocle --help
A commandline application to search, parse, and process BGP information in public sources.


Usage: monocle [OPTIONS] <COMMAND>

Commands:
  parse    Parse individual MRT files given a file path, local or remote
  search   Search BGP messages from all available public MRT files
  server   Start the WebSocket server (ws://<address>:<port>/ws, health: http://<address>:<port>/health)
  inspect  Unified AS and prefix information lookup
  country  Country name and code lookup utilities
  time     Time conversion utilities
  rpki     RPKI utilities
  ip       IP information lookup
  as2rel   AS-level relationship lookup between ASNs
  pfx2as   Prefix-to-ASN mapping lookup
  config   Show monocle configuration, data paths, and database management
  help     Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>  configuration file path (default: $XDG_CONFIG_HOME/monocle/monocle.toml)
      --debug            Print debug information
      --format <FORMAT>  Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json             Output as JSON objects (shortcut for --format json-pretty)
      --no-update        Disable automatic database updates (use existing cached data only)
  -h, --help             Print help
  -V, --version          Print version
```

### `monocle parse`

Parsing a single MRT file given a local path or a remote URL.

```text
➜  monocle parse --help
Parse individual MRT files given a file path, local or remote

Usage: monocle parse [OPTIONS] <FILE>

Arguments:
  <FILE>
          File path to an MRT file, local or remote

Options:
      --pretty
          Pretty-print JSON output

      --debug
          Print debug information

  -M, --mrt-path <MRT_PATH>
          MRT output file path

  -f, --fields <FIELDS>
          Comma-separated list of fields to output. Available fields: type, timestamp, peer_ip, peer_asn, prefix, as_path, origin, next_hop, local_pref, med, communities, atomic, aggr_asn, aggr_ip, collector

      --format <FORMAT>
          Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)

      --json
          Output as JSON objects (shortcut for --format json-pretty)

      --order-by <ORDER_BY>
          Order output by field (enables buffering)

          Possible values:
          - timestamp: Order by timestamp (default)
          - prefix:    Order by network prefix
          - peer_ip:   Order by peer IP address
          - peer_asn:  Order by peer AS number
          - as_path:   Order by AS path (string comparison)
          - next_hop:  Order by next hop IP address

      --no-update
          Disable automatic database updates (use existing cached data only)

      --order <ORDER>
          Order direction (asc or desc, default: asc)

          Possible values:
          - asc:  Ascending order (smallest/oldest first)
          - desc: Descending order (largest/newest first)
          
          [default: asc]

      --time-format <TIME_FORMAT>
          Timestamp output format for non-JSON output (unix or rfc3339)

          Possible values:
          - unix:    Unix timestamp (integer or float) - default for backward compatibility
          - rfc3339: RFC3339/ISO 8601 format (e.g., "2023-10-11T15:00:00Z")
          
          [default: unix]

  -o, --origin-asn <ORIGIN_ASN>
          Filter by origin AS Number(s), comma-separated. Prefix with ! to exclude

  -p, --prefix <PREFIX>
          Filter by network prefix(es), comma-separated. Prefix with ! to exclude

  -s, --include-super
          Include super-prefixes when filtering

  -S, --include-sub
          Include sub-prefixes when filtering

  -j, --peer-ip <PEER_IP>
          Filter by peer IP address(es)

  -J, --peer-asn <PEER_ASN>
          Filter by peer ASN(s), comma-separated. Prefix with ! to exclude

  -C, --community <COMMUNITIES>
          Filter by BGP community value(s), comma-separated (`A:B` or `A:B:C`). Each part can be a number or `*` wildcard (e.g., `*:100`, `13335:*`, `57866:104:31`). Prefix with ! to exclude
          
          [aliases: --communities]

  -m, --elem-type <ELEM_TYPE>
          Filter by elem type: announce (a) or withdraw (w)

          Possible values:
          - a: BGP announcement
          - w: BGP withdrawal

  -t, --start-ts <START_TS>
          Filter by start unix timestamp inclusive

  -T, --end-ts <END_TS>
          Filter by end unix timestamp inclusive

  -d, --duration <DURATION>
          Duration from the start-ts or end-ts, e.g. 1h

  -a, --as-path <AS_PATH>
          Filter by AS path regex string

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

#### Multi-value Filters

The `parse` and `search` commands support filtering by multiple values with OR logic:

```bash
# Match elements from ANY of the specified origin ASNs
monocle parse file.mrt -o 13335,15169,8075

# Match ANY of the specified prefixes
monocle parse file.mrt -p 1.1.1.0/24,8.8.8.0/24

# Match elements from ANY of the specified peer ASNs
monocle parse file.mrt -J 174,2914
```

#### Negative Filters

Use the `!` prefix to exclude values:

```bash
# Exclude elements from AS13335
monocle parse file.mrt -o '!13335'

# Exclude elements from AS13335 AND AS15169
monocle parse file.mrt -o '!13335,!15169'
```

Note: Cannot mix positive and negative values in the same filter.

#### Field Selection

Use `-f` or `--fields` to select which columns to display:

```bash
# Show only prefix, as_path, and origin
monocle parse file.mrt -f prefix,as_path,origin

# Available fields: type, timestamp, peer_ip, peer_asn, prefix, as_path, origin,
#   next_hop, local_pref, med, communities, atomic, aggr_asn, aggr_ip, collector
```

#### Output Sorting

Use `--order-by` and `--order` to sort the output:

```bash
# Sort by timestamp ascending (default)
monocle parse file.mrt --order-by timestamp

# Sort by prefix descending
monocle parse file.mrt --order-by prefix --order desc
```

#### Timestamp Format

Use `--time-format` to change timestamp output format:

```bash
# Unix timestamp (default)
monocle parse file.mrt --time-format unix

# RFC3339/ISO 8601 format
monocle parse file.mrt --time-format rfc3339
```

Example: parse a remote MRT file and show only announcements for a specific prefix:

```text
➜  monocle parse https://data.ris.ripe.net/rrc00/2024.01/updates.20240101.0000.gz \
    -p 1.1.1.0/24 -m a | head -5
┌──────────┬─────────────────────┬───────────────────────────┬──────────┬────────────┬───────────────────────────────────────────┬────────┬─────────────┬───────────────────────────┬────────────┬─────┬─────────────┬────────┬──────────┬─────────┬──────────────────┬─────────┬────────────┬───────────┐
│ type     │ timestamp           │ peer_ip                   │ peer_asn │ prefix     │ as_path                                   │ origin │ origin_asns │ next_hop                  │ local_pref │ med │ communities │ atomic │ aggr_asn │ aggr_ip │ only_to_customer │ unknown │ deprecated │ collector │
├──────────┼─────────────────────┼───────────────────────────┼──────────┼────────────┼───────────────────────────────────────────┼────────┼─────────────┼───────────────────────────┼────────────┼─────┼─────────────┼────────┼──────────┼─────────┼──────────────────┼─────────┼────────────┼───────────┤
│ announce │ 2024-01-01 00:00:44 │ 2001:7f8:4::9d85:1        │ 40325    │ 1.1.1.0/24 │ 40325 13335                               │ IGP    │ 13335       │ 2001:7f8:4::9d85:1        │            │     │             │ false  │          │         │                  │         │            │           │
│ announce │ 2024-01-01 00:00:50 │ 2001:7f8:4::3:2e8b:1      │ 208571   │ 1.1.1.0/24 │ 208571 6939 13335                         │ IGP    │ 13335       │ 2001:7f8:4::3:2e8b:1      │            │     │             │ false  │          │         │                  │         │            │           │
```

#### Output Format

The output contains the following fields:

| Field | Description |
|-------|-------------|
| `type` | Message type: `announce` or `withdraw` |
| `timestamp` | Message timestamp in UTC |
| `peer_ip` | IP address of the BGP peer |
| `peer_asn` | ASN of the BGP peer |
| `prefix` | Network prefix being announced/withdrawn |
| `as_path` | AS path (space-separated) |
| `origin` | Origin type: IGP, EGP, or INCOMPLETE |
| `origin_asns` | Origin AS number(s) |
| `next_hop` | Next hop IP address |
| `local_pref` | Local preference value |
| `med` | Multi-exit discriminator |
| `communities` | BGP communities |
| `atomic` | Atomic aggregate flag |
| `aggr_asn` | Aggregator ASN |
| `aggr_ip` | Aggregator IP |
| `only_to_customer` | OTC attribute (RFC 9234) |
| `unknown` | Unknown attributes |
| `deprecated` | Deprecated attributes |
| `collector` | Collector name (for search results) |

JSON output example:

```json
{
  "type": "announce",
  "timestamp": "2024-01-01T00:00:44Z",
  "peer_ip": "2001:7f8:4::9d85:1",
  "peer_asn": 40325,
  "prefix": "1.1.1.0/24",
  "as_path": "40325 13335",
  "origin": "IGP",
  "origin_asns": [13335],
  "next_hop": "2001:7f8:4::9d85:1",
  "local_pref": null,
  "med": null,
  "communities": [],
  "atomic": false,
  "aggr_asn": null,
  "aggr_ip": null,
  "only_to_customer": null,
  "unknown": null,
  "deprecated": null,
  "collector": null
}
```

### `monocle search`

Search for BGP messages from all available public MRT files using [BGPKIT Broker](https://github.com/bgpkit/bgpkit-broker).

```text
➜  monocle search --help
Search BGP messages from all available public MRT files

Usage: monocle search [OPTIONS]

Options:
      --dry-run
          Dry-run, do not download or parse

      --debug
          Print debug information

      --sqlite-path <SQLITE_PATH>
          SQLite output file path

      --format <FORMAT>
          Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)

  -M, --mrt-path <MRT_PATH>
          MRT output file path

      --json
          Output as JSON objects (shortcut for --format json-pretty)

      --sqlite-reset
          SQLite reset database content if exists

      --broker-files
          Output matching broker files (URLs) and exit without searching

      --no-update
          Disable automatic database updates (use existing cached data only)

  -f, --fields <FIELDS>
          Comma-separated list of fields to output. Available fields: type, timestamp, peer_ip, peer_asn, prefix, as_path, origin, next_hop, local_pref, med, communities, atomic, aggr_asn, aggr_ip, collector

      --order-by <ORDER_BY>
          Order output by field (enables buffering)

          Possible values:
          - timestamp: Order by timestamp (default)
          - prefix:    Order by network prefix
          - peer_ip:   Order by peer IP address
          - peer_asn:  Order by peer AS number
          - as_path:   Order by AS path (string comparison)
          - next_hop:  Order by next hop IP address

      --order <ORDER>
          Order direction (asc or desc, default: asc)

          Possible values:
          - asc:  Ascending order (smallest/oldest first)
          - desc: Descending order (largest/newest first)
          
          [default: asc]

      --time-format <TIME_FORMAT>
          Timestamp output format for non-JSON output (unix or rfc3339)

          Possible values:
          - unix:    Unix timestamp (integer or float) - default for backward compatibility
          - rfc3339: RFC3339/ISO 8601 format (e.g., "2023-10-11T15:00:00Z")
          
          [default: unix]

      --use-cache
          Use the default XDG cache directory ($XDG_CACHE_HOME/monocle) for MRT files. Overridden by --cache-dir if both are specified

      --cache-dir <CACHE_DIR>
          Override cache directory for downloaded MRT files. Files are stored as {cache-dir}/{collector}/{path}. If a file already exists in cache, it will be used instead of downloading

  -o, --origin-asn <ORIGIN_ASN>
          Filter by origin AS Number(s), comma-separated. Prefix with ! to exclude

  -p, --prefix <PREFIX>
          Filter by network prefix(es), comma-separated. Prefix with ! to exclude

  -s, --include-super
          Include super-prefixes when filtering

  -S, --include-sub
          Include sub-prefixes when filtering

  -j, --peer-ip <PEER_IP>
          Filter by peer IP address(es)

  -J, --peer-asn <PEER_ASN>
          Filter by peer ASN(s), comma-separated. Prefix with ! to exclude

  -C, --community <COMMUNITIES>
          Filter by BGP community value(s), comma-separated (`A:B` or `A:B:C`). Each part can be a number or `*` wildcard (e.g., `*:100`, `13335:*`, `57866:104:31`). Prefix with ! to exclude
          
          [aliases: --communities]

  -m, --elem-type <ELEM_TYPE>
          Filter by elem type: announce (a) or withdraw (w)

          Possible values:
          - a: BGP announcement
          - w: BGP withdrawal

  -t, --start-ts <START_TS>
          Filter by start unix timestamp inclusive

  -T, --end-ts <END_TS>
          Filter by end unix timestamp inclusive

  -d, --duration <DURATION>
          Duration from the start-ts or end-ts, e.g. 1h

  -a, --as-path <AS_PATH>
          Filter by AS path regex string

  -c, --collector <COLLECTOR>
          Filter by collector, e.g., rrc00 or route-views2

  -P, --project <PROJECT>
          Filter by route collection project, i.e., riperis or routeviews

  -D, --dump-type <DUMP_TYPE>
          Specify data dump type to search (updates or RIB dump)

          Possible values:
          - updates:     BGP updates only
          - rib:         BGP RIB dump only
          - rib-updates: BGP RIB dump and BGP updates
          
          [default: updates]

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

#### Local Caching

Enable local caching with `--use-cache` (default XDG cache path) or `--cache-dir` (custom path):

```bash
# Cache MRT files under $XDG_CACHE_HOME/monocle
monocle search -t 2024-01-01 -d 1h -p 1.1.1.0/24 --use-cache

# Cache MRT files to a custom directory
monocle search -t 2024-01-01 -d 1h -p 8.8.8.0/24 --cache-dir /tmp/mrt-cache

# --cache-dir overrides --use-cache when both are specified
monocle search -t 2024-01-01 -d 1h -p 8.8.8.0/24 --use-cache --cache-dir /tmp/mrt-cache
```

Features:
- Files are cached as `{cache-dir}/{collector}/{path}` (e.g., `cache/rrc00/2024.01/updates.20240101.0000.gz`)
- Uses `.partial` extension during downloads to handle interrupted transfers
- **Broker query caching**: When caching is enabled (`--use-cache` or `--cache-dir`), broker API results are cached in SQLite at `{cache-dir}/broker-cache.sqlite3`
- Default cache path for `--use-cache` is `$XDG_CACHE_HOME/monocle` (fallback: `~/.cache/monocle`)
- Only queries with end time >2 hours in the past are cached (recent data may still change)
- Enables offline operation: run search once with network, then run same search again without network using cached results

Example: search for BGP announcements for a prefix during a specific time window:

```text
➜  monocle search -t 2024-01-01T00:00:00Z -T 2024-01-01T00:01:00Z \
    -c rrc00 -p 1.1.1.0/24 -m a
```

Use `--broker-files` to see the list of MRT files that would be queried without actually parsing them:

```text
➜  monocle search -t 2024-01-01T00:00:00Z -T 2024-01-01T01:00:00Z \
    -c rrc00 --broker-files
```

### `monocle time`

Parse and convert time strings between various formats.

```text
➜  monocle time --help
Time conversion utilities

Usage: monocle time [OPTIONS] [TIME]...

Arguments:
  [TIME]...  Time stamp or time string to convert

Options:
  -s, --simple           Simple output, only print the converted time (RFC3339 format)
      --debug            Print debug information
      --format <FORMAT>  Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json             Output as JSON objects (shortcut for --format json-pretty)
      --no-update        Disable automatic database updates (use existing cached data only)
  -h, --help             Print help
  -V, --version          Print version
```

Examples:

```text
➜  monocle time 1704067200
┌────────────┬──────────────────────┬─────────────────────────────────────┐
│ unix       │ rfc3339              │ human                               │
├────────────┼──────────────────────┼─────────────────────────────────────┤
│ 1704067200 │ 2024-01-01T00:00:00Z │ Mon, Jan 1, 2024 at 12:00:00 AM UTC │
└────────────┴──────────────────────┴─────────────────────────────────────┘

➜  monocle time "2024-01-01T00:00:00Z"
┌────────────┬──────────────────────┬─────────────────────────────────────┐
│ unix       │ rfc3339              │ human                               │
├────────────┼──────────────────────┼─────────────────────────────────────┤
│ 1704067200 │ 2024-01-01T00:00:00Z │ Mon, Jan 1, 2024 at 12:00:00 AM UTC │
└────────────┴──────────────────────┴─────────────────────────────────────┘

➜  monocle time "yesterday" "last week"
```

### `monocle inspect`

Unified AS and prefix information lookup. Replaces the former `whois` and `pfx2as` commands.

By default, `inspect` shows all available information for ASN and prefix queries, including:
- **Basic**: AS name, country, organization, and PeeringDB info (website, IRR AS-SET)
- **Prefixes**: Announced prefixes with RPKI validation status
- **Connectivity**: AS relationships (upstreams, peers, downstreams)
- **RPKI**: ROAs and ASPA records

When querying multiple ASNs, a **glance table** is automatically shown first, providing a quick overview of all queried ASNs before the detailed per-ASN information.

```text
➜  monocle inspect --help
Unified AS and prefix information lookup

Usage: monocle inspect [OPTIONS] [QUERY]...

Arguments:
  [QUERY]...  One or more queries: ASN (13335, AS13335), prefix (1.1.1.0/24), IP (1.1.1.1), or name (cloudflare)

Options:
  -a, --asn                Force treat queries as ASNs
      --debug              Print debug information
  -p, --prefix             Force treat queries as prefixes
      --format <FORMAT>    Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
  -n, --name               Force treat queries as name search
  -c, --country <COUNTRY>  Search by country code (e.g., US, DE)
      --json               Output as JSON objects (shortcut for --format json-pretty)
      --no-update          Disable automatic database updates (use existing cached data only)
      --show <SECTION>     Select data sections to display (can be repeated). Overrides defaults. Available: basic, prefixes, connectivity, rpki, all
      --full               Show all data sections with no limits
      --full-roas          Show all RPKI ROAs (default: top 10)
      --full-prefixes      Show all prefixes (default: top 10)
      --full-connectivity  Show all neighbors (default: top 5 per category)
      --limit <N>          Limit search results (default: 20)
  -u, --update             Force refresh the asinfo database
  -h, --help               Print help
  -V, --version            Print version
```

Examples:

```text
# Look up AS by number (shows all information by default)
➜  monocle inspect 13335
Query: 13335 (type: asn)
─── Basic Information ───
ASN:     AS13335
Name:    CLOUDFLARENET
Country: US
Org:     Cloudflare, Inc.
Org ID:  CLOUD14-ARIN
Website: https://www.cloudflare.com
AS-SET:     AS13335:AS-CLOUDFLARE

─── Announced Prefixes ───
Total: 5526 (2409 IPv4, 3117 IPv6)
RPKI Validation: valid 5071 (91.8%), invalid 1 (0.0%), unknown 454 (8.2%)
╭─────────────────────┬────────────╮
│ Prefix              │ Validation │
├─────────────────────┼────────────┤
│ 103.186.74.0/24     │ unknown    │
│ ...                 │ ...        │
╰─────────────────────┴────────────╯
(showing 10 of 5526 prefixes, use --full-prefixes to show all)

─── Connectivity ───
...
(results truncated, use --full-connectivity to show all)

─── RPKI ───
ROAs: 4420 total (2754 IPv4, 1666 IPv6)
...
(ROA list truncated, use --full-roas to show all)

# Query multiple ASNs (glance table shown first)
➜  monocle inspect 13335 15169
─── Glance ───
╭─────────┬───────────────┬─────────┬──────────────────╮
│ ASN     │ Name          │ Country │ Org              │
├─────────┼───────────────┼─────────┼──────────────────┤
│ AS13335 │ CLOUDFLARENET │ US      │ Cloudflare, Inc. │
│ AS15169 │ GOOGLE        │ US      │ Google LLC       │
╰─────────┴───────────────┴─────────┴──────────────────╯

════════════════════════════════════════════════════════════════════════════════

Query: 13335 (type: asn)
─── Basic Information ───
...

# Search by name
➜  monocle inspect -n cloudflare
Query: cloudflare (type: name)

─── Search Results ───
Found: 5 matches
╭────────┬────────────────────────────┬─────────╮
│ ASN    │ Name                       │ Country │
├────────┼────────────────────────────┼─────────┤
│ 13335  │ CLOUDFLARENET              │ US      │
│ ...    │ ...                        │ ...     │
╰────────┴────────────────────────────┴─────────╯

# Look up prefix
➜  monocle inspect 1.1.1.0/24
Query: 1.1.1.0/24 (type: prefix)

─── Announced Prefix ───
╭────────────────┬────────────┬─────────┬────────────╮
│ Matched Prefix │ Match Type │ ASN     │ Validation │
├────────────────┼────────────┼─────────┼────────────┤
│ 1.1.1.0/24     │ exact      │ AS13335 │ valid      │
╰────────────────┴────────────┴─────────┴────────────╯

─── Covering ROAs ───
╭────────────┬────────────┬────────────┬───────╮
│ Prefix     │ Max Length │ Origin ASN │ TA    │
├────────────┼────────────┼────────────┼───────┤
│ 1.1.1.0/24 │ 24         │ AS13335    │ APNIC │
╰────────────┴────────────┴────────────┴───────╯

# Show only basic information
➜  monocle inspect 13335 --show basic
```

### `monocle country`

Look up country names and codes.

```text
➜  monocle country --help
Country name and code lookup utilities

Usage: monocle country [OPTIONS] [QUERY]

Arguments:
  [QUERY]  Search query: country code (e.g., "US") or partial name (e.g., "united")

Options:
  -a, --all              List all countries
      --debug            Print debug information
  -s, --simple           Output as simple text (code: name)
      --format <FORMAT>  Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json             Output as JSON objects (shortcut for --format json-pretty)
      --no-update        Disable automatic database updates (use existing cached data only)
  -h, --help             Print help
  -V, --version          Print version
```

Examples:

```text
➜  monocle country US
┌──────┬───────────────┐
│ code │ name          │
├──────┼───────────────┤
│ US   │ United States │
└──────┴───────────────┘

➜  monocle country germany
┌──────┬─────────┐
│ code │ name    │
├──────┼─────────┤
│ DE   │ Germany │
└──────┴─────────┘
```

### `monocle as2rel`

Look up AS-level relationships between ASNs using BGPKIT's AS relationship data.

```text
➜  monocle as2rel --help
AS-level relationship lookup between ASNs

Usage: monocle as2rel [OPTIONS] <ASNS>...

Arguments:
  <ASNS>...
          One or more ASNs to query relationships for
          
          - Single ASN: shows all relationships for that ASN - Two ASNs: shows the relationship between them - Multiple ASNs: shows relationships for all pairs (asn1 < asn2)

Options:
  -u, --update
          Force update the local as2rel database

      --debug
          Print debug information

      --update-with <UPDATE_WITH>
          Update with a custom data file (local path or URL)

      --format <FORMAT>
          Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)

      --no-explain
          Hide the explanation text

      --json
          Output as JSON objects (shortcut for --format json-pretty)

      --sort-by-asn
          Sort by ASN2 ascending instead of connected percentage descending

      --no-update
          Disable automatic database updates (use existing cached data only)

      --show-name
          Show organization name for ASN2 (from asinfo database)

      --show-full-name
          Show full organization name without truncation (default truncates to 20 chars)

      --min-visibility <PERCENT>
          Minimum visibility percentage (0-100) to include in results
          
          Filters out relationships seen by fewer than this percentage of peers.

      --single-homed
          Only show ASNs that are single-homed to the queried ASN
          
          An ASN is single-homed if it has exactly one upstream provider. This finds ASNs where the queried ASN is their ONLY upstream.
          
          Only applicable when querying a single ASN.

      --is-upstream
          Only show relationships where the queried ASN is an upstream (provider)
          
          Shows the downstream customers of the queried ASN. Only applicable when querying a single ASN.

      --is-downstream
          Only show relationships where the queried ASN is a downstream (customer)
          
          Shows the upstream providers of the queried ASN. Only applicable when querying a single ASN.

      --is-peer
          Only show peer relationships
          
          Only applicable when querying a single ASN.

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

Output columns:
- `asn1` / `asn2`: The two ASNs being compared
- `connected`: Percentage of peers that see any connection between the ASNs
- `peer`: Percentage seeing pure peering relationship
- `as1_upstream`: Percentage seeing ASN1 as upstream of ASN2
- `as2_upstream`: Percentage seeing ASN2 as upstream of ASN1

Examples:

```text
# Look up relationship between two ASNs
➜  monocle as2rel 13335 174
┌───────┬──────┬───────────┬───────┬─────────────┬─────────────┐
│ asn1  │ asn2 │ connected │ peer  │ as1_upstream│ as2_upstream│
├───────┼──────┼───────────┼───────┼─────────────┼─────────────┤
│ 13335 │ 174  │ 95.2%     │ 85.1% │ 2.3%        │ 7.8%        │
└───────┴──────┴───────────┴───────┴─────────────┴─────────────┘

# Show all relationships for an ASN with names
➜  monocle as2rel 13335 --show-name | head -10

# Find ASNs that are single-homed to AS2914 (NTT)
➜  monocle as2rel 2914 --single-homed --show-name

# Find single-homed ASNs with at least 10% visibility
➜  monocle as2rel 2914 --single-homed --min-visibility 10

# Show only downstream customers of an ASN
➜  monocle as2rel 2914 --is-upstream --show-name

# Show only upstream providers of an ASN
➜  monocle as2rel 13335 --is-downstream --show-name

# Show relationships among multiple ASNs (all pairs)
➜  monocle as2rel 174 2914 3356 --show-name
```

### `monocle pfx2as`

Look up prefix-to-ASN mappings. Query by prefix to find origin ASNs, or by ASN to find announced prefixes.
Results include RPKI validation status for each prefix-ASN pair.

```text
➜  monocle pfx2as --help
Prefix-to-ASN mapping lookup

Query by prefix to find origin ASNs, or by ASN to find announced prefixes. Includes RPKI validation status for each prefix-ASN pair.

Usage: monocle pfx2as [OPTIONS] <QUERY>

Arguments:
  <QUERY>
          Query: an IP prefix (e.g., 1.1.1.0/24) or ASN (e.g., 13335, AS13335)

Options:
  -u, --update
          Force update the local pfx2as database

      --debug
          Print debug information

      --include-sub
          Include sub-prefixes (more specific) in results when querying by prefix

      --format <FORMAT>
          Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)

      --include-super
          Include super-prefixes (less specific) in results when querying by prefix

      --json
          Output as JSON objects (shortcut for --format json-pretty)

      --show-name
          Show AS name for each origin ASN

      --no-update
          Disable automatic database updates (use existing cached data only)

      --show-full-name
          Show full AS name without truncation (default truncates to 20 chars)

  -l, --limit <N>
          Limit the number of results (default: no limit)

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

Examples:

```text
# Look up a prefix - shows origin ASN and RPKI validation status
➜  monocle pfx2as 1.1.1.0/24
╭────────────┬────────────┬───────╮
│ prefix     │ origin_asn │ rpki  │
├────────────┼────────────┼───────┤
│ 1.1.1.0/24 │ 13335      │ valid │
╰────────────┴────────────┴───────╯

# Look up with AS name
➜  monocle pfx2as 1.1.1.0/24 --show-name
╭────────────┬────────────┬───────────────┬───────╮
│ prefix     │ origin_asn │ as_name       │ rpki  │
├────────────┼────────────┼───────────────┼───────┤
│ 1.1.1.0/24 │ 13335      │ CLOUDFLARENET │ valid │
╰────────────┴────────────┴───────────────┴───────╯

# Look up by ASN - shows all prefixes announced by the ASN
➜  monocle pfx2as 13335 --limit 5 --show-name
╭─────────────────────┬────────────┬───────────────┬───────────╮
│ prefix              │ origin_asn │ as_name       │ rpki      │
├─────────────────────┼────────────┼───────────────┼───────────┤
│ 172.69.7.0/24       │ 13335      │ CLOUDFLARENET │ valid     │
│ 2606:4700:839a::/48 │ 13335      │ CLOUDFLARENET │ valid     │
│ 8.36.218.0/24       │ 13335      │ CLOUDFLARENET │ not_found │
│ 2400:cb00:b8e6::/48 │ 13335      │ CLOUDFLARENET │ valid     │
│ 172.68.134.0/24     │ 13335      │ CLOUDFLARENET │ valid     │
╰─────────────────────┴────────────┴───────────────┴───────────╯

# Include sub-prefixes (more specific prefixes)
➜  monocle pfx2as 8.8.0.0/16 --include-sub --limit 5 --show-name
╭──────────────┬────────────┬────────────┬───────────╮
│ prefix       │ origin_asn │ as_name    │ rpki      │
├──────────────┼────────────┼────────────┼───────────┤
│ 8.0.0.0/12   │ 3356       │ LEVEL3     │ not_found │
│ 8.8.8.0/24   │ 15169      │ GOOGLE     │ valid     │
│ 8.8.249.0/24 │ 989        │ ANAXA3-ASN │ valid     │
│ 8.8.216.0/24 │ 13781      │ ENERGYNET  │ valid     │
│ 8.8.64.0/24  │ 3356       │ LEVEL3     │ not_found │
╰──────────────┴────────────┴────────────┴───────────╯

# Include super-prefixes (less specific prefixes)
➜  monocle pfx2as 1.1.1.0/24 --include-super

# JSON output
➜  monocle pfx2as 13335 --limit 3 --json
[
  {
    "prefix": "172.69.7.0/24",
    "origin_asn": 13335,
    "rpki": "valid"
  },
  {
    "prefix": "2606:4700:839a::/48",
    "origin_asn": 13335,
    "rpki": "valid"
  },
  {
    "prefix": "8.36.218.0/24",
    "origin_asn": 13335,
    "rpki": "not_found"
  }
]
```

### `monocle rpki`

RPKI utilities for validation and listing ROAs/ASPAs.

Data sources:
- Current data: [Cloudflare's rpki.json](https://rpki.cloudflare.com/rpki.json) (cached locally in SQLite)
- Historical data: [RIPE NCC RPKI archives](https://ftp.ripe.net/rpki/) and [RPKIviews](https://rpkiviews.org/)

```text
➜  monocle rpki --help
RPKI utilities

Usage: monocle rpki [OPTIONS] <COMMAND>

Commands:
  validate  validate a prefix-asn pair using cached RPKI data
  roas      list ROAs from RPKI data (current or historical via bgpkit-commons)
  aspas     list ASPAs from RPKI data (current or historical via bgpkit-commons)
  help      Print this message or the help of the given subcommand(s)

Options:
      --debug            Print debug information
      --format <FORMAT>  Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json             Output as JSON objects (shortcut for --format json-pretty)
      --no-update        Disable automatic database updates (use existing cached data only)
  -h, --help             Print help
  -V, --version          Print version
```

#### `monocle rpki validate`

Validate a prefix-ASN pair against cached RPKI data. Implements RFC 6811 validation logic:
- **Valid**: Covering ROA exists with matching ASN and prefix length ≤ max_length
- **Invalid**: Covering ROA exists but ASN doesn't match or prefix length exceeds max_length
- **NotFound**: No covering ROA exists for the prefix

```text
➜  monocle rpki validate --help
validate a prefix-asn pair using cached RPKI data

Usage: monocle rpki validate [OPTIONS] [RESOURCES] [RESOURCES]...

Arguments:
  [RESOURCES] [RESOURCES]...  Two resources: one prefix and one ASN (order does not matter)

Options:
  -r, --refresh          Force refresh the RPKI cache before validation
      --debug            Print debug information
      --format <FORMAT>  Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json             Output as JSON objects (shortcut for --format json-pretty)
      --no-update        Disable automatic database updates (use existing cached data only)
  -h, --help             Print help
  -V, --version          Print version
```

Examples:

```text
➜  monocle rpki validate 1.1.1.0/24 13335
┌────────────┬───────┬────────┬───────────────────────────────────┐
│ prefix     │ asn   │ status │ reason                            │
├────────────┼───────┼────────┼───────────────────────────────────┤
│ 1.1.1.0/24 │ 13335 │ Valid  │ Covered by ROA: 1.1.1.0/24-24     │
└────────────┴───────┴────────┴───────────────────────────────────┘

➜  monocle rpki validate 1.1.1.0/24 12345
┌────────────┬───────┬─────────┬────────────────────────────────────────────┐
│ prefix     │ asn   │ status  │ reason                                     │
├────────────┼───────┼─────────┼────────────────────────────────────────────┤
│ 1.1.1.0/24 │ 12345 │ Invalid │ ASN mismatch: ROA allows 13335, got 12345  │
└────────────┴───────┴─────────┴────────────────────────────────────────────┘
```

#### `monocle rpki roas`

List ROAs from RPKI data. Supports both current (cached from Cloudflare) and historical data.

```text
➜  monocle rpki roas --help
list ROAs from RPKI data (current or historical via bgpkit-commons)

Usage: monocle rpki roas [OPTIONS] [RESOURCES]...

Arguments:
  [RESOURCES]...  Filter by resources (prefixes or ASNs, auto-detected)

Options:
      --date <DATE>            Load historical data for this date (YYYY-MM-DD)
      --debug                  Print debug information
      --source <SOURCE>        Historical data source: ripe, rpkiviews (default: ripe) [default: ripe]
      --collector <COLLECTOR>  RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost) [default: soborost]
      --format <FORMAT>        Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json                   Output as JSON objects (shortcut for --format json-pretty)
  -r, --refresh                Force refresh the RPKI cache (only applies to current data)
      --no-update              Disable automatic database updates (use existing cached data only)
  -h, --help                   Print help
  -V, --version                Print version
```

Examples:

```text
# List ROAs for an ASN (current data)
➜  monocle rpki roas 13335
┌───────┬─────────────────────┬────────────┐
│ asn   │ prefix              │ max_length │
├───────┼─────────────────────┼────────────┤
│ 13335 │ 1.0.0.0/24          │ 24         │
│ 13335 │ 1.1.1.0/24          │ 24         │
│ ...   │ ...                 │ ...        │
└───────┴─────────────────────┴────────────┘

# List ROAs for a prefix
➜  monocle rpki roas 1.1.1.0/24
┌───────┬────────────┬────────────┐
│ asn   │ prefix     │ max_length │
├───────┼────────────┼────────────┤
│ 13335 │ 1.1.1.0/24 │ 24         │
└───────┴────────────┴────────────┘

# Historical data from a specific date
➜  monocle rpki roas 13335 --date 2024-01-01 --source ripe
```

#### `monocle rpki aspas`

List ASPAs (Autonomous System Provider Authorizations) from RPKI data.

```text
➜  monocle rpki aspas --help
list ASPAs from RPKI data (current or historical via bgpkit-commons)

Usage: monocle rpki aspas [OPTIONS]

Options:
      --customer <CUSTOMER>    Filter by customer ASN
      --debug                  Print debug information
      --provider <PROVIDER>    Filter by provider ASN
      --date <DATE>            Load historical data for this date (YYYY-MM-DD)
      --format <FORMAT>        Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json                   Output as JSON objects (shortcut for --format json-pretty)
      --source <SOURCE>        Historical data source: ripe, rpkiviews (default: ripe) [default: ripe]
      --collector <COLLECTOR>  RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost) [default: soborost]
      --no-update              Disable automatic database updates (use existing cached data only)
  -r, --refresh                Force refresh the RPKI cache (only applies to current data)
  -h, --help                   Print help
  -V, --version                Print version
```

Examples:

```text
# List all ASPAs
➜  monocle rpki aspas | head -10

# Filter by customer ASN
➜  monocle rpki aspas --customer 13335

# Filter by provider ASN
➜  monocle rpki aspas --provider 174
```

### `monocle ip`

Look up information about IP addresses.

```text
➜  monocle ip --help
IP information lookup

Usage: monocle ip [OPTIONS] [IP]

Arguments:
  [IP]  IP address to look up (optional)

Options:
      --simple           Print IP address only (e.g., for getting the public IP address quickly)
      --debug            Print debug information
      --format <FORMAT>  Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json             Output as JSON objects (shortcut for --format json-pretty)
      --no-update        Disable automatic database updates (use existing cached data only)
  -h, --help             Print help
  -V, --version          Print version
```

Examples:

```text
# Look up a specific IP
➜  monocle ip 1.1.1.1
┌─────────────┬─────────────────────────────────────────────────┐
│ Field       │ Value                                           │
├─────────────┼─────────────────────────────────────────────────┤
│ ip          │ 1.1.1.1                                         │
│ asn         │ 13335                                           │
│ as_name     │ CLOUDFLARENET                                   │
│ country     │ AU                                              │
│ ...         │ ...                                             │
└─────────────┴─────────────────────────────────────────────────┘

# Get your public IP info
➜  monocle ip
```

### `monocle config`

Show monocle configuration, data paths, and manage the database.

```text
➜  monocle config --help
Show monocle configuration, data paths, and database management

Usage: monocle config [OPTIONS] [COMMAND]

Commands:
  update   Update data source(s)
  backup   Backup the database to a destination
  sources  List available data sources and their status
  help     Print this message or the help of the given subcommand(s)

Options:
      --debug            Print debug information
      --format <FORMAT>  Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)
      --json             Output as JSON objects (shortcut for --format json-pretty)
  -v, --verbose          Show detailed information about all data files
      --no-update        Disable automatic database updates (use existing cached data only)
  -h, --help             Print help
  -V, --version          Print version
```

Examples:

```text
# Show configuration and database status
➜  monocle config
Configuration:
  Config file: ~/.config/monocle/monocle.toml
  Data directory: ~/.local/share/monocle
  Cache directory: ~/.cache/monocle

SQLite Database: ~/.local/share/monocle/monocle-data.sqlite3
  Size: 45.2 MB
  ASInfo: 120415 ASes
  AS2Rel: 1234567 relationships
  RPKI: 784188 ROAs, 388 ASPAs (updated 2 hours ago)
  Pfx2as: 1000000 prefixes

# Update all data sources
➜  monocle config update

# Update a specific source
➜  monocle config update --asinfo
➜  monocle config update --rpki

# Backup the database
➜  monocle config backup ~/monocle-backup.sqlite3

# List available data sources
➜  monocle config sources
```

Notes:
- Persistent data is stored in a single SQLite file at `{data-dir}/monocle-data.sqlite3`
- The default cache directory (`$XDG_CACHE_HOME/monocle`) is created and used on demand when running `monocle search --use-cache`

### `monocle server`

Start a WebSocket server for programmatic access to monocle functionality.

```text
➜  monocle server --help
Start the WebSocket server (ws://<address>:<port>/ws, health: http://<address>:<port>/health)

Note: This requires building with the `server` feature enabled.

Usage: monocle server [OPTIONS]

Options:
      --address <ADDRESS>
          Address to bind to (default: 127.0.0.1)
          
          [default: 127.0.0.1]

      --debug
          Print debug information

      --port <PORT>
          Port to listen on (default: 8080)
          
          [default: 8080]

      --data-dir <DATA_DIR>
          Monocle data directory (default: $XDG_DATA_HOME/monocle)

      --format <FORMAT>
          Output format: table, markdown, json, json-pretty, json-line, psv (default varies by command)

      --json
          Output as JSON objects (shortcut for --format json-pretty)

      --max-concurrent-ops <MAX_CONCURRENT_OPS>
          Maximum concurrent operations per connection (0 = unlimited)

      --max-message-size <MAX_MESSAGE_SIZE>
          Maximum websocket message size in bytes

      --no-update
          Disable automatic database updates (use existing cached data only)

      --connection-timeout-secs <CONNECTION_TIMEOUT_SECS>
          Idle timeout in seconds

      --ping-interval-secs <PING_INTERVAL_SECS>
          Ping interval in seconds

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

**Endpoints:**
- WebSocket: `ws://<address>:<port>/ws`
- Health check: `http://<address>:<port>/health`

**Features:**
- JSON-RPC style request/response protocol
- Streaming support with progress reporting for parse/search operations
- Operation cancellation via `op_id`
- DB-first policy: queries read from local SQLite cache

**Available methods:**
- `system.info`, `system.methods` - Server introspection
- `time.parse` - Time string parsing
- `ip.lookup`, `ip.public` - IP information lookup
- `rpki.validate`, `rpki.roas`, `rpki.aspas` - RPKI operations
- `as2rel.search`, `as2rel.relationship`, `as2rel.update` - AS relationships
- `pfx2as.lookup` - Prefix-to-ASN mapping
- `country.lookup` - Country code/name lookup
- `inspect.query`, `inspect.refresh` - Unified AS/prefix inspection
- `parse.start`, `parse.cancel` - MRT file parsing (streaming)
- `search.start`, `search.cancel` - BGP message search (streaming)
- `database.status`, `database.refresh` - Database management

For detailed protocol specification, see [`src/server/README.md`](src/server/README.md).

Example:

```text
➜  monocle server
Starting WebSocket server on 127.0.0.1:8080
  WebSocket: ws://127.0.0.1:8080/ws
  Health: http://127.0.0.1:8080/health

➜  monocle server --address 0.0.0.0 --port 3000
Starting WebSocket server on 0.0.0.0:3000
```
