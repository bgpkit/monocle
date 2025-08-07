# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Monocle is a command-line tool for searching, parsing, and processing BGP (Border Gateway Protocol) information from public sources. It's part of the BGPKIT ecosystem and written in Rust.

## Development Commands

### Building
```bash
cargo build
cargo build --verbose
```

### Testing
```bash
cargo test
cargo test --verbose
```

### Code Quality Checks
```bash
cargo fmt          # Format code
cargo fmt --check  # Check formatting without changes
cargo clippy       # Run linting
```

### Documentation
After updating documentation in lib.rs:
```bash
cargo readme > README.md
```

## Architecture Overview

### Module Structure
- `src/bin/monocle.rs` - Main binary entry point with CLI command handling
- `src/lib.rs` - Library root that re-exports all public modules
- `src/config.rs` - Configuration management for the tool
- `src/database.rs` - SQLite database operations for storing BGP data
- `src/time.rs` - Time conversion utilities between Unix timestamps and RFC3339
- `src/filters/` - BGP message filtering logic for parse and search operations
- `src/datasets/` - Data source integrations:
  - `as2org.rs` - AS to organization mapping using CAIDA dataset
  - `country.rs` - Country code/name lookup utilities
  - `ip.rs` - IP address information retrieval
  - `pfx2as.rs` - Prefix to AS number mapping
  - `radar.rs` - Cloudflare Radar API integration
  - `rpki/` - RPKI validation and ROA/ASPA file parsing

### Key Dependencies
- `bgpkit-parser` - MRT file parsing
- `bgpkit-broker` - BGP data source discovery
- `radar-rs` - Cloudflare Radar API client
- `rpki` - RPKI data processing
- `rusqlite` - SQLite database interface
- `clap` - Command-line argument parsing
- `tabled` - Table formatting for output

## Core Functionality

The tool provides these main subcommands:
- `parse` - Parse individual MRT files (local or remote)
- `search` - Search BGP messages across public MRT collectors
- `whois` - AS and organization lookup
- `country` - Country code/name utilities
- `time` - Unix timestamp/RFC3339 conversion
- `rpki` - RPKI validation and ROA/ASPA utilities
- `radar` - BGP stats via Cloudflare Radar API
- `ip` - IP address information lookup
- `pfx2as` - Bulk prefix-to-ASN mapping

## Testing Approach

Tests are integrated within source files using `#[test]` attributes. Test data files are stored in `tests/` directory. Run all tests with `cargo test`.

## Release Process

When preparing a release:
1. Update version in `Cargo.toml`
2. Update `CHANGELOG.md` with release notes
3. Run full test suite and quality checks
4. Tag the release with `v{version}` format