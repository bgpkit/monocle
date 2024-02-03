# Changelog

All notable changes to this project will be documented in this file.

## v0.5.3 - 2024-02-03

### Highlights

* remove openssl dependency, switching to rustls as TLS backend
* support installation via `cargo-binstall` 

## v0.5.2 - 2023-12-18

* add GitHub actions config to build `monocle` binary for macOS (Universal), and linux (arm and amd64)
* add `vendored-openssl` optional feature flag to enable GitHub actions builds for different systems.
* move `monocle` binary to `bin` directory
* install `monocle` with `brew install bgpkit/tap/monocle`
