# Monocle macOS UI proof of concept

This standalone crate is an initial GPUI frontend for Monocle's `search`
functionality. It calls `SearchLens` directly and streams progress and bounded
result batches into a virtualized `gpui-component` table.

The dependency revisions are pinned to the GPUI revision recorded by the
checked-out `gpui-component` lockfile:

- GPUI: `cc053a4a6fa2fd0e8793201ed9099466af1be0b1`
- gpui-component: `d598c6c0a61c23650dd933d58d046c0708085531`

## Run on macOS

```bash
cargo run --manifest-path apps/monocle-gui/Cargo.toml
```

The initial form supports:

- start and end time (the same human-readable or Unix formats as the CLI)
- update, RIB, or combined searches
- collector, prefix, and origin ASN filters
- a client-side result limit
- live progress, result rows, and cancellation

The default query covers a 15-minute window ending two hours ago and stops at
500 results. Search requires network access to the BGPKIT Broker and public MRT
archives.
