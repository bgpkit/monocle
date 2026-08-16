//! Cisco `sh ip bgp` text dump parsing.
//!
//! The text dump parser moved upstream into `bgpkit-parser` (v0.20.0,
//! `bgpkit_parser::parser::text_dump`). This module re-exports the upstream
//! API so existing `monocle::lens::parse::text_dump::*` call sites and the
//! `parse` command's format auto-detection keep working unchanged.

pub use bgpkit_parser::parser::text_dump::{
    detect_text_dump, infer_timestamp_from_path, parse_header, parse_text_dump,
    parse_text_dump_with_timestamp, TextDumpElemIterator, TextDumpHeader,
};
