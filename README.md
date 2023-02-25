# Monocle

[![Rust](https://github.com/bgpkit/monocle/actions/workflows/rust.yml/badge.svg)](https://github.com/bgpkit/monocle/actions/workflows/rust.yml)
[![Crates.io](https://img.shields.io/crates/v/monocle)](https://crates.io/crates/monocle)
[![Docs.rs](https://docs.rs/monocle/badge.svg)](https://docs.rs/monocle)
[![License](https://img.shields.io/crates/l/monocle)](https://raw.githubusercontent.com/bgpkit/monocle/main/LICENSE)

See through all BGP data with a monocle.

![](https://spaces.bgpkit.org/assets/monocle/monocle-emoji.png)

*Still in early prototype phase. You are warned.*

## Install

```bash
cargo install monocle
```

## Usage

Subcommands:
- `parse`: parse individual MRT files
- `search`: search for matching messages from all available public MRT files
- `whois`: search AS and organization information by ASN or name
- `time`: utility to convert time between unix timestamp and RFC3339 string
- `country`: utility to lookup country name and code

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
  help     Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>  configuration file path, by default $HOME/.monocle.toml is used
      --debug            Print debug information
  -h, --help             Print help
  -V, --version          Print version
```

### `monocle parse`

Parsing single MRT file given a local path or a remote URL.

```text
➜  monocle git:(main) ✗ monocle parse --help
monocle-parse 0.0.1
Parse individual MRT files given a file path, local or remote

USAGE:
    monocle parse [OPTIONS] <FILE>

ARGS:
    <FILE>    File path to a MRT file, local or remote

OPTIONS:
    -a, --as-path <AS_PATH>          Filter by AS path regex string
    -h, --help                       Print help information
    -j, --peer-ip <PEER_IP>          Filter by peer IP address
    -J, --peer-asn <PEER_ASN>        Filter by peer ASN
        --json                       Output as JSON objects
    -m, --elem-type <ELEM_TYPE>      Filter by elem type: announce (a) or withdraw (w)
    -o, --origin-asn <ORIGIN_ASN>    Filter by origin AS Number
    -p, --prefix <PREFIX>            Filter by network prefix
        --pretty                     Pretty-print JSON output
    -s, --include-super              Include super-prefix when filtering
    -S, --include-sub                Include sub-prefix when filtering
    -t, --start-ts <START_TS>        Filter by start unix timestamp inclusive
    -T, --end-ts <END_TS>            Filter by end unix timestamp inclusive
    -V, --version                    Print version information
```

### `monocle search`

Search for BGP messages across publicly available BGP route collectors and parse relevant
MRT files in parallel. More filters can be used to search for messages that match your criteria.

```text
➜  monocle git:(main) ✗ monocle search --help
Search BGP messages from all available public MRT files

USAGE:
    monocle search [OPTIONS] --start-ts <START_TS> --end-ts <END_TS>

OPTIONS:
    -a, --as-path <AS_PATH>          Filter by AS path regex string
    -c, --collector <COLLECTOR>      Filter by collector, e.g. rrc00 or route-views2
    -d, --debug                      Print debug information
    -d, --dry-run                    Dry-run, do not download or parse
    -h, --help                       Print help information
    -j, --peer-ip <PEER_IP>          Filter by peer IP address
    -J, --peer-asn <PEER_ASN>        Filter by peer ASN
    -m, --elem-type <ELEM_TYPE>      Filter by elem type: announce (a) or withdraw (w)
    -o, --origin-asn <ORIGIN_ASN>    Filter by origin AS Number
    -p, --prefix <PREFIX>            Filter by network prefix
    -P, --project <PROJECT>          Filter by route collection project, i.e. riperis or routeviews
    -s, --include-super              Include super-prefix when filtering
    -S, --include-sub                Include sub-prefix when filtering
    -t, --start-ts <START_TS>        Filter by start unix timestamp inclusive
    -T, --end-ts <END_TS>            Filter by end unix timestamp inclusive
    -V, --version                    Print version information
```

### `monocle time`

Convert between UNIX timestamp and RFC3339 time strings.

```text
➜  ~ monocle time --help              
Time conversion utilities

USAGE:
    monocle time [TIME]

ARGS:
    <TIME>    Time stamp or time string to convert

OPTIONS:
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

➜  monocle time 0
╭──────┬───────────────────────────┬──────────────╮
│ unix │ rfc3339                   │ human        │
├──────┼───────────────────────────┼──────────────┤
│ 0    │ 1970-01-01T00:00:00+00:00 │ 52 years ago │
╰──────┴───────────────────────────┴──────────────╯

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

Use `--pretty` to output the table with pretty rounded corner
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
Check RPKI validity for given prefix-ASN pair and provide utility to read ROA and ASPA files from the RPKI archive.

```text
➜  monocle rpki --help
RPKI utility module

Usage: monocle rpki <COMMAND>

Commands:
  roa    parse a RPKI ROA file
  aspa   parse a RPKI ASPA file
  check  validate a prefix-asn pair with a RPKI validator
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

```

#### `monocle rpki check`

Check RPKI validity for given prefix-ASN pair. We use RIPE NCC's [routinator instance](https://rpki-validator.ripe.net) as the data source.

```text
➜  monocle rpki check --help
validate a prefix-asn pair with a RPKI validator

Usage: monocle rpki check --asn <ASN> --prefix <PREFIX>

Options:
  -a, --asn <ASN>        
  -p, --prefix <PREFIX>  
  -h, --help             Print help
  -V, --version          Print version
```

```text
➜  monocle rpki check --asn 400644 --prefix 2620:AA:A000::/48 
| asn    | prefix            | validity |
|--------|-------------------|----------|
| 400644 | 2620:aa:a000::/48 | valid    |

➜  monocle rpki check --asn 400644 --prefix 2620:AA:A000::/49 
| asn    | prefix            | validity |
|--------|-------------------|----------|
| 400644 | 2620:aa:a000::/49 | invalid  |

```

#### `monocle rpki roa`
Parse a given RPKI ROA file and display the prefix-ASN pairs with max length.

```text
➜  monocle rpki roa https://spaces.bgpkit.org/parser/bgpkit.roa

| asn    | prefix            | max_len |
|--------|-------------------|---------|
| 393949 | 192.67.222.0/24   | 24      |
| 393949 | 192.195.251.0/24  | 24      |
| 393949 | 2620:98:4000::/44 | 48      |
```

#### `monocle rpki aspa`

Parse a given RPKI ASPA file and display the allowed upstreams.

```text
➜  monocle rpki aspa https://spaces.bgpkit.org/parser/as945.asa
| asn | allowed_upstream |
|-----|------------------|
| 945 | 1299             |
|     | 6939             |
|     | 7480             |
|     | 32097            |
|     | 50058            |
|     | 61138            |
```

## Built with ❤️ by BGPKIT Team

<a href="https://bgpkit.com"><img src="https://bgpkit.com/Original%20Logo%20Cropped.png" alt="https://bgpkit.com/favicon.ico" width="200"/></a>
