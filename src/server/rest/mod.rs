//! REST handler modules for non-streaming endpoints.
//!
//! Organized by domain:
//! - `time` — time parsing
//! - `country` — country code/name lookup
//! - `ip` — IP information lookup
//! - `rpki` — RPKI ROA/ASPA lookup and validation
//! - `as2rel` — AS-level relationships
//! - `pfx2as` — prefix-to-ASN mapping
//! - `inspect` — unified AS/prefix inspection
//! - `database` — database status and refresh

pub mod as2rel;
pub mod country;
pub mod database;
pub mod inspect;
pub mod ip;
pub mod pfx2as;
pub mod rpki;
pub mod time;
