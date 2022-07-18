# Monocle

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
- `time`: utility to convert time between unix timestamp and RFC3339 string
- `whois`: search AS and organization information by ASN or name

Top-level help menu:
```text
➜  ~ monocle                      
monocle 0.0.4
Mingwei Zhang <mingwei@bgpkit.com>
A commandline application to search, parse, and process BGP information in public sources.

USAGE:
    monocle [OPTIONS] <SUBCOMMAND>

OPTIONS:
    -c, --config <CONFIG>    configuration file path, by default $HOME/.monocle.toml is used
        --debug              Print debug information
    -h, --help               Print help information
    -V, --version            Print version information

SUBCOMMANDS:
    help      Print this message or the help of the given subcommand(s)
    parse     Parse individual MRT files given a file path, local or remote
    search    Search BGP messages from all available public MRT files
    time      Time conversion utilities
    whois     ASN and organization lookup utility
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

Convert between UNIX timestamp and RFC3339 time strings.

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

### `monocle whois`

Search AS/organization-level information with ASN or organization name.

Data source:
- The CAIDA AS Organizations Dataset, http://www.caida.org/data/as-organizations
- Please also cite the data source above if you use this tool for your public work.

```text
➜  ~ monocle whois --help
monocle-whois 0.0.4
ASN and organization lookup utility

USAGE:
    monocle whois [OPTIONS] <QUERY>

ARGS:
    <QUERY>    Search query, an ASN (e.g. "400644") or a name (e.g. "bgpkit")

OPTIONS:
    -a, --asn-only     Search by ASN only
    -h, --help         Print help information
    -n, --name-only    Search AS and Org name only
    -u, --update       Refresh local as2org database
    -V, --version      Print version information
```

Example queries:
```text
➜  ~ monocle whois 400644
+--------+------------+------------+--------------+-------------+----------+
|  asn   |  as_name   |  org_name  |    org_id    | org_country | org_size |
+--------+------------+------------+--------------+-------------+----------+
| 400644 | BGPKIT-LLC | BGPKIT LLC | BL-1057-ARIN |     US      |    1     |
+--------+------------+------------+--------------+-------------+----------+

➜  ~ monocle whois bgpkit
+--------+------------+------------+--------------+-------------+----------+
|  asn   |  as_name   |  org_name  |    org_id    | org_country | org_size |
+--------+------------+------------+--------------+-------------+----------+
| 400644 | BGPKIT-LLC | BGPKIT LLC | BL-1057-ARIN |     US      |    1     |
+--------+------------+------------+--------------+-------------+----------+
```

## Built with ❤️ by BGPKIT Team

BGPKIT is a small-team focuses on building the best open-source tooling for BGP data processing in Rust. We have over 10 years of
experience in working with BGP data and we believe that our work can enable more companies to start keeping tracks of BGP data
on their own turf. Learn more about what we do at https://bgpkit.com.

<a href="https://bgpkit.com"><img src="https://bgpkit.com/Original%20Logo%20Cropped.png" alt="https://bgpkit.com/favicon.ico" width="200"/></a>
