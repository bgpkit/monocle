# Changelog

All notable changes to this project will be documented in this file.

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
