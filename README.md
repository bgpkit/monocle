# Monocle

See through all BGP data with a monocle.

![](https://spaces.bgpkit.org/assets/monocle/monocle-200px.jpg)

*Still in early prototype phase. You are warned.*

## Install

```bash
cargo install monocle
```

## Usage

Subcommands:
- `parse`: parse individual MRT files
- `search`: search for matching messages from all available public MRT files
- `time`: utility to convert time between unix timestamp and RFC3339 string

Top-level help menu:
```text
monocle 0.0.1
Mingwei Zhang <mingwei@bgpkit.com>
A commandline application to search, parse, and process BGP information stored in MRT files.

USAGE:
    monocle <SUBCOMMAND>

OPTIONS:
    -h, --help       Print help information
    -V, --version    Print version information

SUBCOMMANDS:
    help       Print this message or the help of the given subcommand(s)
    parse      Parse individual MRT files given a file path, local or remote
    scouter    Investigative toolbox
    search     Search BGP messages from all available public MRT files
    time       Time conversion utilities
```

### `monocle parse`

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

```text
➜  monocle git:(main) ✗ monocle search --help
monocle-search 0.0.1
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

```text
➜  ~ monocle time --help              
monocle-time 0.0.3
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
+------------+---------------------------+
|    unix    |          rfc3339          |
+------------+---------------------------+
| 1657850362 | 2022-07-15T01:59:22+00:00 |
+------------+---------------------------+

➜  monocle time 0                               
+------+---------------------------+
| unix |          rfc3339          |
+------+---------------------------+
|  0   | 1970-01-01T00:00:00+00:00 |
+------+---------------------------+

➜  monocle time 2022-01-01T00:00:00Z
+------------+---------------------------+
|    unix    |          rfc3339          |
+------------+---------------------------+
| 1640995200 | 2022-01-01T00:00:00+00:00 |
+------------+---------------------------+

➜  monocle time 2022-01-01T00:00:00 
Input time must be either Unix timestamp or time string compliant with RFC3339
```

## Built with ❤️ by BGPKIT Team

BGPKIT is a small-team focuses on building the best open-source tooling for BGP data processing in Rust. We have over 10 years of
experience in working with BGP data and we believe that our work can enable more companies to start keeping tracks of BGP data
on their own turf. Learn more about what we do at https://bgpkit.com.

<a href="https://bgpkit.com"><img src="https://bgpkit.com/Original%20Logo%20Cropped.png" alt="https://bgpkit.com/favicon.ico" width="200"/></a>
