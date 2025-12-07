# Monocle

[![Rust](https://github.com/bgpkit/monocle/actions/workflows/rust.yml/badge.svg)](https://github.com/bgpkit/monocle/actions/workflows/rust.yml)
[![Crates.io](https://img.shields.io/crates/v/monocle)](https://crates.io/crates/monocle)
[![Docs.rs](https://docs.rs/monocle/badge.svg)](https://docs.rs/monocle)
[![License](https://img.shields.io/crates/l/monocle)](https://raw.githubusercontent.com/bgpkit/monocle/main/LICENSE)

See through all Border Gateway Protocol (BGP) data with a monocle.

![](https://spaces.bgpkit.org/assets/monocle/monocle-emoji.png)

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

## Usage

Subcommands:

- `parse`: parse individual MRT files
- `search`: search for matching messages from all available public MRT files
- `broker`: query BGPKIT Broker for available MRT metadata
- `whois`: search AS and organization information by ASN or name
- `country`: utility to look up country name and code
- `time`: utility to convert time between unix timestamp and RFC3339 string
- `rpki`: check RPKI validation for given ASNs or prefixes

Top-level help menu:

```text
➜  ~ monocle                      
A commandline application to search, parse, and process BGP information in public sources.


Usage: monocle [OPTIONS] <COMMAND>

Commands:
  parse    Parse individual MRT files given a file path, local or remote
  search   Search BGP messages from all available public MRT files
  whois    ASN and organization lookup utility
  country  ASN and organization lookup utility
  time     Time conversion utilities
  rpki     RPKI utilities
  help     Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>  configuration file path, by default $HOME/.monocle.toml is used
      --debug            Print debug information
  -h, --help             Print help
  -V, --version          Print version
```

### `monocle parse`

Parsing a single MRT file given a local path or a remote URL.

```text
➜  monocle git:(main) ✗ monocle parse --help
Parse individual MRT files given a file path, local or remote

Usage: monocle parse [OPTIONS] <FILE>

Arguments:
  <FILE>  File path to a MRT file, local or remote

Options:
      --json                     Output as JSON objects
      --debug                    Print debug information
      --pretty                   Pretty-print JSON output
  -M, --mrt-path <MRT_PATH>      MRT output file path
  -o, --origin-asn <ORIGIN_ASN>  Filter by origin AS Number
  -p, --prefix <PREFIX>          Filter by network prefix
  -s, --include-super            Include super-prefix when filtering
  -S, --include-sub              Include sub-prefix when filtering
  -j, --peer-ip <PEER_IP>        Filter by peer IP address
  -J, --peer-asn <PEER_ASN>      Filter by peer ASN
  -m, --elem-type <ELEM_TYPE>    Filter by elem type: announce (a) or withdraw (w)
  -t, --start-ts <START_TS>      Filter by start unix timestamp inclusive
  -T, --end-ts <END_TS>          Filter by end unix timestamp inclusive
  -a, --as-path <AS_PATH>        Filter by AS path regex string
  -h, --help                     Print help
  -V, --version                  Print version
```

### `monocle search`

Search for BGP messages across publicly available BGP route collectors and parse relevant
MRT files in parallel. More filters can be used to search for messages that match your criteria.

```text
➜  monocle git:(main) ✗ monocle search --help
Search BGP messages from all available public MRT files

Usage: monocle search [OPTIONS]

Options:
      --dry-run                    Dry-run, do not download or parse
      --debug                      Print debug information
      --json                       Output as JSON objects
      --pretty                     Pretty-print JSON output
      --sqlite-path <SQLITE_PATH>  SQLite output file path
  -M, --mrt-path <MRT_PATH>        MRT output file path
      --sqlite-reset               SQLite reset database content if exists
  -t, --start-ts <START_TS>        Filter by start unix timestamp inclusive
  -T, --end-ts <END_TS>            Filter by end unix timestamp inclusive
  -d, --duration <DURATION>        
  -c, --collector <COLLECTOR>      Filter by collector, e.g. rrc00 or route-views2
  -P, --project <PROJECT>          Filter by route collection project, i.e. riperis or routeviews
  -o, --origin-asn <ORIGIN_ASN>    Filter by origin AS Number
  -p, --prefix <PREFIX>            Filter by network prefix
  -s, --include-super              Include super-prefix when filtering
  -S, --include-sub                Include sub-prefix when filtering
  -j, --peer-ip <PEER_IP>          Filter by peer IP address
  -J, --peer-asn <PEER_ASN>        Filter by peer ASN
  -m, --elem-type <ELEM_TYPE>      Filter by elem type: announce (a) or withdraw (w)
  -a, --as-path <AS_PATH>          Filter by AS path regex string
  -h, --help                       Print help
  -V, --version                    Print version
```

### `monocle broker`

Query BGPKIT Broker for the metadata of available MRT files. This subcommand returns file listings
that match your filters and time window. By default, results are printed as a table; use `--json` to
print JSON.

Usage:

```text
monocle broker \
  --start-ts <START_TS> \
  --end-ts <END_TS> \
  [--collector <COLLECTOR>] \
  [--project <PROJECT>] \
  [--data-type <DATA_TYPE>] \
  [--page <PAGE>] \
  [--page-size <PAGE_SIZE>] \
  [--json]
```

Notes:

- <START_TS> and <END_TS> accept either RFC3339 time strings (e.g., 2024-01-01T00:00:00Z)
  or Unix timestamps in seconds. The same flexible time parsing is used by the `search` subcommand.
- If `--page` is specified (1-based), monocle queries only that single page using Broker's
  `query_single_page()`.
- If `--page` is NOT specified, monocle uses Broker's `query()` and returns at most 10 pages worth
  of results (calculated as `page-size * 10`). Use `--page` and `--page-size` to manually iterate
  if you need more than 10 pages.
- `--page-size` defaults to `1000` if not provided.
- `--data-type` can be `updates` or `rib` (case-insensitive behavior depends on Broker),
  or omitted to retrieve both.

Examples:

- List RIPE RIS rrc00 updates during the first hour of 2024 (table output):

```bash
monocle broker \
  --start-ts 2024-01-01T00:00:00Z \
  --end-ts 2024-01-01T01:00:00Z \
  --collector rrc00 \
  --project riperis \
  --data-type updates
```

- Fetch page 2 with a custom page size (manual iteration):

```bash
monocle broker \
  --start-ts 2024-01-01T00:00:00Z \
  --end-ts 2024-01-01T02:00:00Z \
  --page 2 \
  --page-size 500
```

- Output as JSON:

```bash
monocle broker \
  --start-ts 1704067200 \
  --end-ts 1704070800 \
  --project routeviews \
  --json
```

### `monocle time`

Convert between UNIX timestamp and RFC3339 time strings.
We use the [`dateparser`][dateparser] crate for parsing time
strings.

[dateparser]:https://github.com/waltzofpearls/dateparser

```text
➜  ~ monocle time --help              
Time conversion utilities

USAGE:
    monocle time [TIME]

ARGS:
    <TIME>    Time stamp or time string to convert

OPTIONS:
    -s, --simple   Simple output, only print the converted time
    -h, --help       Print help information
    -V, --version    Print version information
```

Example runs:

```text
➜  monocle time
╭────────────┬───────────────────────────┬───────╮
│ unix       │ rfc3339                   │ human │
├────────────┼───────────────────────────┼───────┤
│ 1659135226 │ 2022-07-29T22:53:46+00:00 │ now   │
╰────────────┴───────────────────────────┴───────╯

➜  monocle time 2022-01-01T00:00:00Z
╭────────────┬───────────────────────────┬──────────────╮
│ unix       │ rfc3339                   │ human        │
├────────────┼───────────────────────────┼──────────────┤
│ 1640995200 │ 2022-01-01T00:00:00+00:00 │ 6 months ago │
╰────────────┴───────────────────────────┴──────────────╯

➜  monocle time 2022-01-01T00:00:00 
Input time must be either Unix timestamp or time string compliant with RFC3339
```

### `monocle whois`

Search AS/organization-level information with ASN or organization name.

Data source:

- The CAIDA AS Organizations Dataset, http://www.caida.org/data/as-organizations
- Please also cite the data source above if you use this tool for your public work.

```text
➜  ~ monocle whois --help
ASN and organization lookup utility

Usage: monocle whois [OPTIONS] [QUERY]...

Arguments:
  [QUERY]...  Search query, an ASN (e.g. "400644") or a name (e.g. "bgpkit")

Options:
  -n, --name-only     Search AS and Org name only
  -a, --asn-only      Search by ASN only
  -C, --country-only  Search by country only
  -u, --update        Refresh local as2org database
  -p, --pretty        Output to pretty table, default markdown table
  -F, --full-table    Display full table (with ord_id, org_size)
  -P, --psv           Export to pipe-separated values
  -f, --full-country  Show full country names instead of 2-letter code
  -h, --help          Print help
  -V, --version       Print version
```

Example queries:

```text
➜  ~ monocle whois 400644
| asn    | as_name    | org_name   | org_country |
|--------|------------|------------|-------------|
| 400644 | BGPKIT-LLC | BGPKIT LLC | US          |

➜  ~ monocle whois bgpkit
| asn    | as_name    | org_name   | org_country |
|--------|------------|------------|-------------|
| 400644 | BGPKIT-LLC | BGPKIT LLC | US          |
```

You can specify multiple queries:

```text
➜  monocle whois 13335 bgpkit               
| asn    | as_name       | org_name         | org_country |
|--------|---------------|------------------|-------------|
| 13335  | CLOUDFLARENET | Cloudflare, Inc. | US          |
| 400644 | BGPKIT-LLC    | BGPKIT LLC       | US          |
```

Use `--pretty` to output the table with a pretty rounded corner

```text
➜  monocle whois 13335 bgpkit --pretty
╭────────┬───────────────┬──────────────────┬─────────────╮
│ asn    │ as_name       │ org_name         │ org_country │
├────────┼───────────────┼──────────────────┼─────────────┤
│ 13335  │ CLOUDFLARENET │ Cloudflare, Inc. │ US          │
│ 400644 │ BGPKIT-LLC    │ BGPKIT LLC       │ US          │
╰────────┴───────────────┴──────────────────┴─────────────╯
```

### `monocle country`

Country name and code lookup utilities.

```text
➜  ~ monocle country --help              
Country name and code lookup utilities

Usage: monocle country <QUERY>

Arguments:
  <QUERY>  Search query, e.g. "US" or "United States"

Options:
  -h, --help     Print help
  -V, --version  Print version

```

Example runs:

```text
➜  monocle country US    
╭──────┬──────────────────────────╮
│ code │ name                     │
├──────┼──────────────────────────┤
│ US   │ United States of America │
╰──────┴──────────────────────────╯

➜  monocle country united
╭──────┬──────────────────────────────────────────────────────╮
│ code │ name                                                 │
├──────┼──────────────────────────────────────────────────────┤
│ TZ   │ Tanzania, United Republic of                         │
│ GB   │ United Kingdom of Great Britain and Northern Ireland │
│ AE   │ United Arab Emirates                                 │
│ US   │ United States of America                             │
│ UM   │ United States Minor Outlying Islands                 │
╰──────┴──────────────────────────────────────────────────────╯

➜  monocle country "United States" 
╭──────┬──────────────────────────────────────╮
│ code │ name                                 │
├──────┼──────────────────────────────────────┤
│ UM   │ United States Minor Outlying Islands │
│ US   │ United States of America             │
╰──────┴──────────────────────────────────────╯
```

### `monocle rpki`:

RPKI utilities for checking validity, listing ROAs/ASPAs, and querying historical RPKI data.

Data sources:
- Real-time validation: [Cloudflare RPKI validator](https://rpki.cloudflare.com)
- Historical data: [RIPE NCC RPKI archives](https://ftp.ripe.net/rpki/) and [RPKIviews](https://rpkiviews.org/)

```text
➜  monocle rpki --help
RPKI utilities

Usage: monocle rpki <COMMAND>

Commands:
  check    validate a prefix-asn pair with a RPKI validator (Cloudflare)
  list     list ROAs by ASN or prefix (Cloudflare real-time)
  summary  summarize RPKI status for a list of given ASNs (Cloudflare)
  roas     list ROAs from RPKI data (current or historical via bgpkit-commons)
  aspas    list ASPAs from RPKI data (current or historical via bgpkit-commons)
  help     Print this message or the help of the given subcommand(s)
```

#### `monocle rpki check`

Check RPKI validity for a given prefix-ASN pair using Cloudflare's RPKI validator.

```text
➜  monocle rpki check --help
validate a prefix-asn pair with a RPKI validator (Cloudflare)

Usage: monocle rpki check [OPTIONS] --asn <ASN> --prefix <PREFIX>

Options:
  -a, --asn <ASN>        
  -p, --prefix <PREFIX>  
      --debug            Print debug information
      --json             Output as JSON objects
  -h, --help             Print help
  -V, --version          Print version
```

```text
➜  monocle rpki check --asn 400644 --prefix 2620:AA:A000::/48 
RPKI validation result:
| asn    | prefix            | validity |
|--------|-------------------|----------|
| 400644 | 2620:aa:a000::/48 | valid    |

Covering prefixes:
| asn    | prefix            | max_length |
|--------|-------------------|------------|
| 400644 | 2620:aa:a000::/48 | 48         |

➜  monocle rpki check --asn 400644 --prefix 2620:AA:A000::/49 
RPKI validation result:
| asn    | prefix            | validity |
|--------|-------------------|----------|
| 400644 | 2620:aa:a000::/49 | invalid  |

Covering prefixes:
| asn    | prefix            | max_length |
|--------|-------------------|------------|
| 400644 | 2620:aa:a000::/48 | 48         |
```

JSON output:

```text
➜  monocle rpki check --asn 400644 --prefix 2620:AA:A000::/48 --json
{"covering_roas":[{"asn":400644,"max_length":48,"prefix":"2620:aa:a000::/48"}],"validation":{"asn":400644,"prefix":"2620:aa:a000::/48","validity":"Valid"}}
```

#### `monocle rpki list`

List signed ROAs for a given ASN or prefix.

```text
➜ monocle rpki list 13335
| asn   | prefix              | max_length |
|-------|---------------------|------------|
| 13335 | 197.234.240.0/22    | 22         |
| 13335 | 197.234.240.0/24    | 24         |
| 13335 | 197.234.241.0/24    | 24         |
| 13335 | 197.234.242.0/24    | 24         |
| 13335 | 197.234.243.0/24    | 24         |
| 13335 | 2c0f:f248::/32      | 32         |
| 13335 | 210.17.44.0/24      | 24         |
| 13335 | 103.22.200.0/23     | 23         |
...
```

```text
➜ monocle rpki list 1.1.1.0/24
| asn   | prefix     | max_length |
|-------|------------|------------|
| 13335 | 1.1.1.0/24 | 24         |
```

#### `monocle rpki summary`

Summarize RPKI status for a list of given ASNs.

```text
➜ monocle rpki summary 701 13335 15169 400644                 
| asn    | signed | routed_valid | routed_invalid | routed_unknown |
|--------|--------|--------------|----------------|----------------|
| 701    | 956    | 890          | 35             | 361            |
| 13335  | 1184   | 1000         | 4              | 221            |
| 15169  | 1372   | 989          | 0              | 5              |
| 400644 | 1      | 0            | 0              | 0              |
```

**NOTE**: due to Cloudflare API's current limitation, the maximum number of entries per `routed_` category is `1000`.

#### `monocle rpki roas`

List ROAs from RPKI data. Supports both current (Cloudflare) and historical (RIPE NCC, RPKIviews) data sources.

```text
➜  monocle rpki roas --help
list ROAs from RPKI data (current or historical via bgpkit-commons)

Usage: monocle rpki roas [OPTIONS]

Options:
      --origin <ORIGIN>        Filter by origin ASN
      --prefix <PREFIX>        Filter by prefix
      --date <DATE>            Load historical data for this date (YYYY-MM-DD)
      --source <SOURCE>        Historical data source: ripe, rpkiviews (default: ripe)
      --collector <COLLECTOR>  RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost)
      --debug                  Print debug information
      --json                   Output as JSON objects
  -h, --help                   Print help
  -V, --version                Print version
```

List ROAs for a specific origin ASN from historical data:

```text
➜  monocle rpki roas --origin 400644 --date 2024-01-04
Found 1 ROAs (historical data from 2024-01-04)
| prefix            | max_length | origin_asn | ta   |
|-------------------|------------|------------|------|
| 2620:aa:a000::/48 | 48         | 400644     | ARIN |
```

List ROAs from RPKIviews historical data:

```text
➜  monocle rpki roas --origin 400644 --date 2024-01-04 --source rpkiviews
Found 1 ROAs (historical data from 2024-01-04)
| prefix            | max_length | origin_asn | ta   |
|-------------------|------------|------------|------|
| 2620:aa:a000::/48 | 48         | 400644     | ARIN |
```

JSON output:

```text
➜  monocle rpki roas --origin 400644 --date 2024-01-04 --json
[{"prefix":"2620:aa:a000::/48","max_length":48,"origin_asn":400644,"ta":"ARIN"}]
```

#### `monocle rpki aspas`

List ASPAs (AS Provider Authorizations) from RPKI data. Supports both current and historical data sources.

```text
➜  monocle rpki aspas --help
list ASPAs from RPKI data (current or historical via bgpkit-commons)

Usage: monocle rpki aspas [OPTIONS]

Options:
      --customer <CUSTOMER>    Filter by customer ASN
      --provider <PROVIDER>    Filter by provider ASN
      --date <DATE>            Load historical data for this date (YYYY-MM-DD)
      --source <SOURCE>        Historical data source: ripe, rpkiviews (default: ripe)
      --collector <COLLECTOR>  RPKIviews collector: soborost, massars, attn, kerfuffle (default: soborost)
      --debug                  Print debug information
      --json                   Output as JSON objects
  -h, --help                   Print help
  -V, --version                Print version
```

List ASPAs for a specific customer ASN:

```text
➜  monocle rpki aspas --customer 15562 --date 2024-01-04
Found 1 ASPAs (historical data from 2024-01-04)
| customer_asn | providers                 |
|--------------|---------------------------|
| 15562        | 2914, 8283, 51088, 206238 |
```

List all ASPAs that have a specific provider:

```text
➜  monocle rpki aspas --provider 6939 --date 2024-01-04
Found 27 ASPAs (historical data from 2024-01-04)
| customer_asn | providers |
|--------------|-----------|
| 7480         | 6939      |
| 40544        | 6939      |
| 47272        | 6939      |
| 47311        | 6939      |
...
```

JSON output (providers as array):

```text
➜  monocle rpki aspas --customer 15562 --date 2024-01-04 --json
[{"customer_asn":15562,"providers":[2914,8283,51088,206238]}]
```

### `monocle radar`:

Lookup BGP information using [Cloudflare Radar](https://radar.cloudflare.com/) API
using [`radar-rs`](https://github.com/bgpkit/radar-rs) crate.

Using this command requires setting up the `CF_API_TOKEN` environment variable. See
the [Cloudflare Radar API getting started guide](https://developers.cloudflare.com/radar/get-started/first-request/) for
detailed steps on getting an API token.

#### `monocle radar stats`: routing statistics

Global routing overview:

```text
➜  monocle radar stats   
┌─────────────┬─────────┬──────────┬─────────────────┬───────────────┬─────────────────┐
│ scope       │ origins │ prefixes │ rpki_valid      │ rpki_invalid  │ rpki_unknown    │
├─────────────┼─────────┼──────────┼─────────────────┼───────────────┼─────────────────┤
│ global      │ 81769   │ 1204488  │ 551831 (45.38%) │ 15652 (1.29%) │ 648462 (53.33%) │
├─────────────┼─────────┼──────────┼─────────────────┼───────────────┼─────────────────┤
│ global ipv4 │ 74990   │ 1001973  │ 448170 (44.35%) │ 11879 (1.18%) │ 550540 (54.48%) │
├─────────────┼─────────┼──────────┼─────────────────┼───────────────┼─────────────────┤
│ global ipv6 │ 31971   │ 202515   │ 103661 (50.48%) │ 3773 (1.84%)  │ 97922 (47.68%)  │
└─────────────┴─────────┴──────────┴─────────────────┴───────────────┴─────────────────┘
```

Country-level routing overview:

```text
➜  monocle radar stats us
┌─────────┬─────────┬──────────┬────────────────┬──────────────┬─────────────────┐
│ scope   │ origins │ prefixes │ rpki_valid     │ rpki_invalid │ rpki_unknown    │
├─────────┼─────────┼──────────┼────────────────┼──────────────┼─────────────────┤
│ us      │ 18151   │ 304200   │ 97102 (31.39%) │ 2466 (0.80%) │ 209820 (67.82%) │
├─────────┼─────────┼──────────┼────────────────┼──────────────┼─────────────────┤
│ us ipv4 │ 17867   │ 262022   │ 73846 (27.81%) │ 1042 (0.39%) │ 190689 (71.80%) │
├─────────┼─────────┼──────────┼────────────────┼──────────────┼─────────────────┤
│ us ipv6 │ 4218    │ 42178    │ 23256 (53.08%) │ 1424 (3.25%) │ 19131 (43.67%)  │
└─────────┴─────────┴──────────┴────────────────┴──────────────┴─────────────────┘

Data generated at 2023-07-24T16:00:00 UTC.
```

AS-level routing overview:

```text
➜  monocle git:(main) ✗ monocle radar stats 174
┌────────────┬─────────┬──────────┬─────────────┬──────────────┬───────────────┐
│ scope      │ origins │ prefixes │ rpki_valid  │ rpki_invalid │ rpki_unknown  │
├────────────┼─────────┼──────────┼─────────────┼──────────────┼───────────────┤
│ as174      │ 1       │ 4425     │ 216 (4.88%) │ 15 (0.34%)   │ 4194 (94.78%) │
├────────────┼─────────┼──────────┼─────────────┼──────────────┼───────────────┤
│ as174 ipv4 │ 1       │ 3684     │ 201 (5.46%) │ 9 (0.24%)    │ 3474 (94.30%) │
├────────────┼─────────┼──────────┼─────────────┼──────────────┼───────────────┤
│ as174 ipv6 │ 1       │ 741      │ 15 (2.02%)  │ 6 (0.81%)    │ 720 (97.17%)  │
└────────────┴─────────┴──────────┴─────────────┴──────────────┴───────────────┘

Data generated at 2023-07-24T16:00:00 UTC.
```

#### `monocle radar pfx2asn`: prefix-to-ASN mapping

Lookup prefix origin for a given prefix (using Cloudflare `1.1.1.0/24` as an example):

```text
➜  monocle radar pfx2as 1.1.1.0/24
┌────────────┬─────────┬───────┬───────────────┐
│ prefix     │ origin  │ rpki  │ visibility    │
├────────────┼─────────┼───────┼───────────────┤
│ 1.1.1.0/24 │ as13335 │ valid │ high (98.78%) │
└────────────┴─────────┴───────┴───────────────┘
```

Lookup prefixes originated by a given AS (using BGPKIT AS400644 as an example):

```text
➜  monocle radar pfx2as 400644    
┌───────────────────┬──────────┬───────┬───────────────┐
│ prefix            │ origin   │ rpki  │ visibility    │
├───────────────────┼──────────┼───────┼───────────────┤
│ 2620:aa:a000::/48 │ as400644 │ valid │ high (93.90%) │
└───────────────────┴──────────┴───────┴───────────────┘

Data generated at 2023-07-24T16:00:00 UTC.
```

Lookup RPKI invalid (with flag `--rpki-status invalid`) prefixes originated by a given AS:

```text
➜  monocle radar pfx2as 174 --rpki-status invalid
┌─────────────────────┬────────┬─────────┬──────────────┐
│ prefix              │ origin │ rpki    │ visibility   │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 2606:d640::/40      │ as174  │ invalid │ low (7.32%)  │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 194.76.218.0/24     │ as174  │ invalid │ mid (29.27%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 154.81.223.0/24     │ as174  │ invalid │ mid (31.71%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 178.171.100.0/24    │ as174  │ invalid │ low (10.98%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 212.69.135.0/24     │ as174  │ invalid │ low (10.98%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 2602:fd92:900::/40  │ as174  │ invalid │ mid (23.17%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 2606:d640:a000::/36 │ as174  │ invalid │ mid (23.17%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 2606:d640:11::/48   │ as174  │ invalid │ mid (23.17%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 172.83.86.0/23      │ as174  │ invalid │ low (13.41%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 2606:d640:100::/40  │ as174  │ invalid │ low (7.32%)  │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 154.93.28.0/24      │ as174  │ invalid │ mid (31.71%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 2606:d640:200::/40  │ as174  │ invalid │ low (7.32%)  │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 173.0.3.0/24        │ as174  │ invalid │ mid (31.71%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 138.34.56.0/22      │ as174  │ invalid │ mid (31.71%) │
├─────────────────────┼────────┼─────────┼──────────────┤
│ 67.159.59.0/24      │ as174  │ invalid │ mid (31.71%) │
└─────────────────────┴────────┴─────────┴──────────────┘

Data generated at 2023-07-24T16:00:00 UTC.
```

### `monocle ip`

Retrieve information for the current IP of the machine or any specified IP address.
The information includes location,
network (ASN, network name) and the covering IP prefix of the given IP address.

Get information for the machine's public IP address:

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
```

Look up IP and network information for a given IP:

```text
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
```

Displaying the information in JSON format:

```text
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

## Built with ❤️ by BGPKIT Team

<a href="https://bgpkit.com"><img src="https://bgpkit.com/Original%20Logo%20Cropped.png" alt="https://bgpkit.com/favicon.ico" width="200"/></a>

