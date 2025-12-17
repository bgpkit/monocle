//! Query helper types (non-core protocol).
//!
//! This module intentionally contains types that are useful for *future* query/streaming
//! methods (e.g. `parse.*`, `search.*`) but are not part of the minimal stable core
//! WebSocket protocol envelope.
//!
//! Rationale:
//! - Keep `protocol.rs` focused on the stable envelope + shared error/progress vocabulary.
//! - Avoid accidental scope creep in the core protocol surface area.
//!
//! These types are purely data containers and should remain network-neutral.

use serde::{Deserialize, Serialize};

/// Pagination parameters for list/query methods.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Pagination {
    /// Maximum number of results to return (clamped to server max, if any).
    #[serde(default)]
    pub limit: Option<u32>,

    /// Offset for pagination (non-negative).
    #[serde(default)]
    pub offset: Option<u32>,
}

/// Query filters shared by future streaming operations (e.g. `parse.start`, `search.start`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct QueryFilters {
    /// Filter by origin ASN.
    #[serde(default)]
    pub origin_asn: Option<u32>,

    /// Filter by prefix.
    #[serde(default)]
    pub prefix: Option<String>,

    /// Include super-prefixes.
    #[serde(default)]
    pub include_super: Option<bool>,

    /// Include sub-prefixes.
    #[serde(default)]
    pub include_sub: Option<bool>,

    /// Filter by peer IPs.
    #[serde(default)]
    pub peer_ip: Vec<String>,

    /// Filter by peer ASN.
    #[serde(default)]
    pub peer_asn: Option<u32>,

    /// Filter by element type (announce/withdraw).
    #[serde(default)]
    pub elem_type: Option<String>,

    /// Start timestamp (RFC3339 or Unix).
    #[serde(default)]
    pub start_ts: Option<String>,

    /// End timestamp (RFC3339 or Unix).
    #[serde(default)]
    pub end_ts: Option<String>,

    /// Filter by AS path regex.
    #[serde(default)]
    pub as_path: Option<String>,
}
