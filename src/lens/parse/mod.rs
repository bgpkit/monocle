//! Parse lens module
//!
//! This module provides filter types for parsing MRT files with bgpkit-parser.
//! The filter types can optionally derive Clap's Args trait when the `cli` feature is enabled.
//!
//! # Multiple Values and OR Logic
//!
//! Filter fields accept multiple comma-separated values with OR logic. When multiple
//! values are specified, elements matching ANY of the values will be included:
//!
//! ```rust,ignore
//! use monocle::lens::parse::ParseFilters;
//!
//! let filters = ParseFilters {
//!     // Match elements from Cloudflare (13335), Google (15169), or Microsoft (8075)
//!     origin_asn: vec!["13335".to_string(), "15169".to_string(), "8075".to_string()],
//!     // Match elements for either prefix
//!     prefix: vec!["1.1.1.0/24".to_string(), "8.8.8.0/24".to_string()],
//!     ..Default::default()
//! };
//! ```
//!
//! # Negative Filters (Exclusion)
//!
//! Prefix values with `!` to exclude them:
//!
//! ```rust,ignore
//! use monocle::lens::parse::ParseFilters;
//!
//! let filters = ParseFilters {
//!     // Exclude elements from AS13335
//!     origin_asn: vec!["!13335".to_string()],
//!     ..Default::default()
//! };
//!
//! // Exclude multiple ASNs (elements NOT from AS13335 AND NOT from AS15169)
//! let filters = ParseFilters {
//!     origin_asn: vec!["!13335".to_string(), "!15169".to_string()],
//!     ..Default::default()
//! };
//! ```
//!
//! **Note**: You cannot mix positive and negative values in the same filter field.
//! All values must either be positive or all prefixed with `!`.
//!
//! # Progress Tracking
//!
//! The `ParseLens` supports progress tracking through callbacks. This is useful for
//! building GUI applications or showing progress in CLI tools.
//!
//! ```rust,ignore
//! use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
//! use std::sync::Arc;
//!
//! let lens = ParseLens::new();
//! let filters = ParseFilters::default();
//!
//! let callback = Arc::new(|progress: ParseProgress| {
//!     if let ParseProgress::Update { messages_processed, .. } = progress {
//!         println!("Processed {} messages", messages_processed);
//!     }
//! });
//!
//! let elems = lens.parse_with_progress(&filters, "file.mrt", Some(callback))?;
//! ```

pub mod filter_file;
pub mod text_dump;

use crate::lens::time::TimeLens;
use anyhow::anyhow;
use anyhow::Result;
use bgpkit_parser::parser::filter::Filter;
use bgpkit_parser::BgpElem;
use bgpkit_parser::BgpkitParser;
use ipnet::IpNet;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::io::Read;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

#[cfg(feature = "cli")]
use clap::{Args, ValueEnum};

// =============================================================================
// Progress Tracking Types
// =============================================================================

/// Progress update interval for parse operations (every 10,000 messages)
pub const PARSE_PROGRESS_INTERVAL: u64 = 10_000;

/// Progress information for parse operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ParseProgress {
    /// Parsing has started
    Started {
        /// Path to the file being parsed
        file_path: String,
    },
    /// Progress update (emitted every PARSE_PROGRESS_INTERVAL messages)
    Update {
        /// Total number of messages processed so far
        messages_processed: u64,
        /// Processing rate in messages per second (if available)
        #[serde(skip_serializing_if = "Option::is_none")]
        rate: Option<f64>,
        /// Elapsed time in seconds
        elapsed_secs: f64,
    },
    /// Parsing has completed
    Completed {
        /// Total number of messages parsed
        total_messages: u64,
        /// Total duration in seconds
        duration_secs: f64,
        /// Average processing rate in messages per second
        #[serde(skip_serializing_if = "Option::is_none")]
        rate: Option<f64>,
    },
}

/// Type alias for progress callback function
///
/// The callback receives `ParseProgress` updates and can be used to
/// update UI elements, log progress, or perform other actions.
pub type ParseProgressCallback = Arc<dyn Fn(ParseProgress) + Send + Sync>;

// =============================================================================
// Types
// =============================================================================

/// Element type for BGP messages
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(ValueEnum))]
pub enum ParseElemType {
    /// BGP announcement
    A,
    /// BGP withdrawal
    W,
}

impl Display for ParseElemType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ParseElemType::A => "announcement",
            ParseElemType::W => "withdrawal",
        })
    }
}

/// MRT output subtype for `--mrt-type`.
///
/// Controls whether the MRT export produces TABLE_DUMP_V2 RIB entries
/// or BGP4MP update messages.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum MrtType {
    /// TABLE_DUMP_V2 RIB entries (peer-index-table + per-prefix RIB)
    Rib,
    /// BGP4MP individual update messages
    Updates,
}

impl MrtType {
    /// Infer the appropriate MRT type from the input format.
    ///
    /// Text dumps are always RIB snapshots → `Rib`.
    /// MRT files default to `Updates` (the existing behavior for
    /// BGP4MP/UPDATE streams).
    pub fn infer(is_text_dump: bool) -> Self {
        if is_text_dump {
            MrtType::Rib
        } else {
            MrtType::Updates
        }
    }
}

// =============================================================================
// Args
// =============================================================================

/// Filters for parsing MRT files
///
/// All filter fields support multiple comma-separated values with OR logic.
/// Values can be prefixed with `!` for negation (exclusion).
///
/// For large filter sets, filters can also be loaded from files using the
/// [`filter_file`](crate::lens::parse::filter_file) module (`--filter-file` for
/// JSON, `--prefix-file` for newline-delimited prefixes). File-based filters
/// are merged with CLI flags: union within each dimension, AND across dimensions.
///
/// # Example
///
/// ```rust
/// use monocle::lens::parse::ParseFilters;
///
/// // Match elements from multiple origin ASNs
/// let filters = ParseFilters {
///     origin_asn: vec!["13335".to_string(), "15169".to_string()],
///     ..Default::default()
/// };
///
/// // Exclude elements from specific ASNs
/// let filters = ParseFilters {
///     origin_asn: vec!["!13335".to_string()],
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(Args))]
pub struct ParseFilters {
    /// Filter by origin AS Number(s), comma-separated. Prefix with ! to exclude.
    #[cfg_attr(feature = "cli", clap(short = 'o', long, value_delimiter = ','))]
    #[serde(default)]
    pub origin_asn: Vec<String>,

    /// Filter by network prefix(es), comma-separated. Prefix with ! to exclude.
    #[cfg_attr(feature = "cli", clap(short = 'p', long, value_delimiter = ','))]
    #[serde(default)]
    pub prefix: Vec<String>,

    /// Include super-prefixes when filtering
    #[cfg_attr(feature = "cli", clap(short = 's', long))]
    #[serde(default)]
    pub include_super: bool,

    /// Include sub-prefixes when filtering
    #[cfg_attr(feature = "cli", clap(short = 'S', long))]
    #[serde(default)]
    pub include_sub: bool,

    /// Filter by peer IP address(es)
    #[cfg_attr(feature = "cli", clap(short = 'j', long))]
    #[serde(default)]
    pub peer_ip: Vec<IpAddr>,

    /// Filter by peer ASN(s), comma-separated. Prefix with ! to exclude.
    #[cfg_attr(feature = "cli", clap(short = 'J', long, value_delimiter = ','))]
    #[serde(default)]
    pub peer_asn: Vec<String>,

    /// Filter by BGP community value(s), comma-separated (`A:B` or `A:B:C`).
    /// Each part can be a number or `*` wildcard (e.g., `*:100`, `13335:*`, `57866:104:31`).
    /// Prefix with ! to exclude.
    #[cfg_attr(
        feature = "cli",
        clap(
            short = 'C',
            long = "community",
            visible_alias = "communities",
            value_delimiter = ','
        )
    )]
    #[serde(default)]
    pub communities: Vec<String>,

    /// Filter by elem type: announce (a) or withdraw (w)
    #[cfg_attr(feature = "cli", clap(short = 'm', long, value_enum))]
    pub elem_type: Option<ParseElemType>,

    /// Filter by start unix timestamp inclusive
    #[cfg_attr(feature = "cli", clap(short = 't', long, visible_alias = "ts-start"))]
    pub start_ts: Option<String>,

    /// Filter by end unix timestamp inclusive
    #[cfg_attr(feature = "cli", clap(short = 'T', long, visible_alias = "ts-end"))]
    pub end_ts: Option<String>,

    /// Duration from the start-ts or end-ts, e.g. 1h
    #[cfg_attr(feature = "cli", clap(short = 'd', long))]
    pub duration: Option<String>,

    /// Filter by AS path regex string
    #[cfg_attr(feature = "cli", clap(short = 'a', long))]
    pub as_path: Option<String>,

    /// Apply a bgpkit-parser filter expression (`key=value` or `key!=value`).
    /// May be specified multiple times. Time filter keys are rejected; use
    /// `--start-ts`, `--end-ts`, and `--duration` instead.
    #[cfg_attr(
        feature = "cli",
        clap(long = "filter", value_name = "KEY=VALUE", action = clap::ArgAction::Append)
    )]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub generic_filters: Vec<String>,

    // --- bgpkit-parser extended element filters ---
    // Each accepts the literal value, or `*` (present) / `!*` (absent) for
    // optional fields (only_to_customer, next_hop, origin, local_pref, med,
    // aggr_asn, aggr_ip, peer_bgp_id).
    /// Filter by only-to-customer ASN (RFC 9234). Use `*`/`!*` for presence.
    /// Maps to the bgpkit-parser filter key `otc` internally.
    #[cfg_attr(
        feature = "cli",
        clap(long = "only-to-customer", visible_alias = "otc")
    )]
    #[serde(default)]
    pub only_to_customer: Option<String>,

    /// Filter by next-hop IP address. Use `*`/`!*` for presence.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub next_hop: Option<String>,

    /// Filter by ORIGIN attribute: igp, egp, or incomplete. Use `*`/`!*` for presence.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub origin: Option<String>,

    /// Filter by LOCAL_PREF value. Use `*`/`!*` for presence.
    #[cfg_attr(feature = "cli", clap(long, visible_alias = "lp"))]
    #[serde(default)]
    pub local_pref: Option<String>,

    /// Filter by MED value. Use `*`/`!*` for presence.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub med: Option<String>,

    /// Filter by atomic-aggregate flag: true or false.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub atomic_aggregate: Option<bool>,

    /// Filter by aggregator ASN. Use `*`/`!*` for presence.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub aggr_asn: Option<String>,

    /// Filter by aggregator IP address. Use `*`/`!*` for presence.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub aggr_ip: Option<String>,

    /// Filter by peer BGP identifier (router ID). Use `*`/`!*` for presence.
    #[cfg_attr(feature = "cli", clap(long, visible_alias = "peer-router-id"))]
    #[serde(default)]
    pub peer_bgp_id: Option<String>,
}

type FilterSpec = (&'static str, String);

const TIME_FILTER_KEYS: [&str; 4] = ["start_ts", "end_ts", "ts_start", "ts_end"];
const MULTI_VALUE_FILTER_KEYS: [&str; 7] = [
    "origin_asns",
    "prefixes",
    "prefixes_super",
    "prefixes_sub",
    "prefixes_super_sub",
    "peer_ips",
    "peer_asns",
];

impl ParseFilters {
    /// Parse start and end time strings into Unix timestamps
    pub fn parse_start_end_strings(&self) -> Result<(i64, i64)> {
        let time_lens = TimeLens::new();
        let mut start_ts = None;
        let mut end_ts = None;
        if let Some(ts) = &self.start_ts {
            match time_lens.parse_time_string(ts.as_str()) {
                Ok(t) => start_ts = Some(t),
                Err(_) => return Err(anyhow!("start-ts is not a valid time string: {}", ts)),
            }
        }
        if let Some(ts) = &self.end_ts {
            match time_lens.parse_time_string(ts.as_str()) {
                Ok(t) => end_ts = Some(t),
                Err(_) => return Err(anyhow!("end-ts is not a valid time string: {}", ts)),
            }
        }

        match (&self.start_ts, &self.end_ts, &self.duration) {
            (Some(_), Some(_), Some(_)) => {
                return Err(anyhow!(
                    "cannot specify start_ts, end_ts, and duration all at the same time"
                ))
            }
            (Some(_), None, None) | (None, Some(_), None) => {
                // only one start_ts or end_ts specified
                return Err(anyhow!(
                    "must specify two from: start_ts, end_ts and duration"
                ));
            }
            (None, None, _) => {
                return Err(anyhow!(
                    "must specify two from: start_ts, end_ts and duration"
                ));
            }
            _ => {}
        }
        if let Some(duration) = &self.duration {
            // this case is duration + start_ts OR end_ts
            let duration = match humantime::parse_duration(duration) {
                Ok(d) => d,
                Err(_) => {
                    return Err(anyhow!(
                        "duration is not a valid time duration string: {}",
                        duration
                    ))
                }
            };

            if let Some(ts) = start_ts {
                return Ok((ts.timestamp(), (ts + duration).timestamp()));
            }
            if let Some(ts) = end_ts {
                return Ok(((ts - duration).timestamp(), ts.timestamp()));
            }
        } else {
            // this case is start_ts AND end_ts
            match (start_ts, end_ts) {
                (Some(start), Some(end)) => return Ok((start.timestamp(), end.timestamp())),
                _ => {
                    return Err(anyhow!(
                        "Both start_ts and end_ts must be provided when duration is not set"
                    ))
                }
            }
        }

        Err(anyhow!("unexpected time-string parsing result"))
    }

    /// Validate the filters
    ///
    /// Checks:
    /// - Time strings are valid
    /// - ASN values are valid 32-bit unsigned integers (with optional `!` prefix)
    /// - Prefix values are valid CIDR notation (with optional `!` prefix)
    /// - Negation is consistent within each filter (all positive or all negative)
    pub fn validate(&self) -> Result<()> {
        let time_lens = TimeLens::new();
        if let Some(ts) = &self.start_ts {
            if time_lens.parse_time_string(ts.as_str()).is_err() {
                return Err(anyhow!("start-ts is not a valid time string: {}", ts));
            }
        }
        if let Some(ts) = &self.end_ts {
            if time_lens.parse_time_string(ts.as_str()).is_err() {
                return Err(anyhow!("end-ts is not a valid time string: {}", ts));
            }
        }

        // Validate origin ASNs
        for asn in &self.origin_asn {
            Self::validate_asn(asn)?;
        }

        // Validate peer ASNs
        for asn in &self.peer_asn {
            Self::validate_asn(asn)?;
        }

        // Validate prefixes
        for prefix in &self.prefix {
            Self::validate_prefix(prefix)?;
        }

        // Validate communities
        for community in &self.communities {
            Self::validate_community(community)?;
        }

        // Check for mixed positive/negative in same filter
        Self::check_negation_consistency(&self.origin_asn, "origin-asn")?;
        Self::check_negation_consistency(&self.peer_asn, "peer-asn")?;
        Self::check_negation_consistency(&self.prefix, "prefix")?;
        Self::check_negation_consistency(&self.communities, "community")?;

        // --- v0.19 extended element filter validation ---
        self.validate_extended_filters()?;
        self.generic_filter_specs()?;

        Ok(())
    }

    /// Validate an ASN value (with optional `!` prefix for negation)
    fn validate_asn(value: &str) -> Result<()> {
        let asn_str = value.strip_prefix('!').unwrap_or(value);
        asn_str.parse::<u32>().map_err(|_| {
            anyhow!(
                "Invalid ASN '{}': must be a valid 32-bit unsigned integer",
                value
            )
        })?;
        Ok(())
    }

    /// Validate a prefix value (with optional `!` prefix for negation)
    fn validate_prefix(value: &str) -> Result<()> {
        let prefix_str = value.strip_prefix('!').unwrap_or(value);
        prefix_str.parse::<IpNet>().map_err(|_| {
            anyhow!(
                "Invalid prefix '{}': must be valid CIDR notation (e.g., 1.1.1.0/24)",
                value
            )
        })?;
        Ok(())
    }

    /// Validate a community value (with optional `!` prefix for negation).
    /// Community format is either `A:B` (standard) or `A:B:C` (large community).
    /// Standard community parts must be `*` or u16 (0-65535).
    /// Large community parts must be `*` or u32 (0-4294967295).
    fn validate_community(value: &str) -> Result<()> {
        let community_str = value.strip_prefix('!').unwrap_or(value);
        let parts: Vec<&str> = community_str.split(':').collect();
        match parts.len() {
            2 => {
                let first_valid = parts[0] == "*" || parts[0].parse::<u16>().is_ok();
                let second_valid = parts[1] == "*" || parts[1].parse::<u16>().is_ok();
                if !first_valid || !second_valid {
                    return Err(anyhow!(
                        "Invalid community '{}': A:B parts must each be 0-65535 or '*'",
                        value
                    ));
                }
            }
            3 => {
                let all_valid = parts.iter().all(|p| *p == "*" || p.parse::<u32>().is_ok());
                if !all_valid {
                    return Err(anyhow!(
                        "Invalid community '{}': A:B:C parts must each be 0-4294967295 or '*'",
                        value
                    ));
                }
            }
            _ => {
                return Err(anyhow!(
                    "Invalid community '{}': must be A:B or A:B:C (e.g., 13335:100, *:100, 57866:104:31)",
                    value
                ));
            }
        }

        Ok(())
    }

    /// Convert a validated community pattern (`A:B` or `A:B:C`) into a strict regex body.
    /// `*` is translated to `\d+` and each community is matched with exact colon positions.
    pub(crate) fn community_pattern_to_regex_body(pattern: &str) -> Result<String> {
        Self::validate_community(pattern)?;
        let value = pattern.strip_prefix('!').unwrap_or(pattern);
        let parts: Vec<&str> = value.split(':').collect();

        let regex_parts = parts
            .iter()
            .map(|part| {
                if *part == "*" {
                    "\\d+".to_string()
                } else {
                    (*part).to_string()
                }
            })
            .collect::<Vec<String>>();

        Ok(regex_parts.join(":"))
    }

    /// Build parser-compatible community filter value from monocle community inputs.
    /// Multi-value positive filters use OR logic; multi-value negative filters negate the OR set.
    fn build_community_filter_value(&self) -> Result<Option<String>> {
        if self.communities.is_empty() {
            return Ok(None);
        }

        Self::check_negation_consistency(&self.communities, "community")?;

        let is_negated = self
            .communities
            .first()
            .map(|v| v.starts_with('!'))
            .unwrap_or(false);

        let mut pattern_bodies = Vec::with_capacity(self.communities.len());
        for pattern in &self.communities {
            pattern_bodies.push(Self::community_pattern_to_regex_body(pattern)?);
        }

        let regex = format!("^(?:{})$", pattern_bodies.join("|"));
        if is_negated {
            Ok(Some(format!("!{}", regex)))
        } else {
            Ok(Some(regex))
        }
    }

    /// Check that all values in a filter are either all positive or all negative
    pub(crate) fn check_negation_consistency(values: &[String], field_name: &str) -> Result<()> {
        if values.len() > 1 {
            let negated_count = values.iter().filter(|v| v.starts_with('!')).count();
            if negated_count > 0 && negated_count < values.len() {
                return Err(anyhow!(
                    "Invalid {}: cannot mix positive and negative values (all must be prefixed with ! or none)",
                    field_name
                ));
            }
        }
        Ok(())
    }

    /// Validate bgpkit-parser v0.19 extended element filter values.
    ///
    /// Each field accepts the literal value, or the presence wildcards `*`
    /// (present) / `!*` (absent) for optional BGP attributes. Literal values
    /// are type-checked: ASN (u32), IP address, or origin keyword.
    fn validate_extended_filters(&self) -> Result<()> {
        // Helper: strip a leading `!` for presence-negated or value-negated inputs.
        fn strip_neg(v: &str) -> &str {
            v.strip_prefix('!').unwrap_or(v)
        }

        // Helper: validate an optional string field that is either a wildcard
        // (`*` / `!*`) or a concrete u32 value (with optional `!` prefix).
        fn validate_u32_field(value: &Option<String>, field: &str) -> Result<()> {
            if let Some(v) = value {
                let v = v.trim();
                if v == "*" || v == "!*" {
                    return Ok(());
                }
                let raw = strip_neg(v);
                if raw.parse::<u32>().is_err() {
                    return Err(anyhow!(
                        "Invalid {field} '{v}': must be a u32, or '*'/'!*' for presence"
                    ));
                }
            }
            Ok(())
        }

        validate_u32_field(&self.only_to_customer, "only-to-customer")?;
        validate_u32_field(&self.local_pref, "local-pref")?;
        validate_u32_field(&self.med, "med")?;
        validate_u32_field(&self.aggr_asn, "aggr-asn")?;

        // IP-address fields (next_hop, aggr_ip, peer_bgp_id)
        for (value, field) in [
            (&self.next_hop, "next-hop"),
            (&self.aggr_ip, "aggr-ip"),
            (&self.peer_bgp_id, "peer-bgp-id"),
        ] {
            if let Some(v) = value {
                let v = v.trim();
                if v == "*" || v == "!*" {
                    continue;
                }
                if IpAddr::from_str(strip_neg(v)).is_err() {
                    return Err(anyhow!(
                        "Invalid {field} '{v}': must be an IP address, or '*'/'!*' for presence"
                    ));
                }
            }
        }

        // origin: igp | egp | incomplete | * | !*
        if let Some(v) = &self.origin {
            let v = v.trim();
            if v == "*" || v == "!*" {
                return Ok(());
            }
            let raw = strip_neg(v).to_lowercase();
            if !matches!(raw.as_str(), "igp" | "egp" | "incomplete") {
                return Err(anyhow!(
                    "Invalid origin '{v}': must be igp, egp, or incomplete (or '*'/'!*' for presence)"
                ));
            }
        }

        Ok(())
    }

    fn filter_specs(&self) -> Result<Vec<FilterSpec>> {
        let mut specs = Vec::new();

        if let Some(value) = &self.as_path {
            specs.push(("as_path", value.clone()));
        }

        // Origin ASN filter - always use plural filter key for consistency.
        if !self.origin_asn.is_empty() {
            specs.push(("origin_asns", self.origin_asn.join(",")));
        }

        // Prefix filter - always use plural filter keys.
        if !self.prefix.is_empty() {
            let filter_key = match (self.include_super, self.include_sub) {
                (false, false) => "prefixes",
                (true, false) => "prefixes_super",
                (false, true) => "prefixes_sub",
                (true, true) => "prefixes_super_sub",
            };
            specs.push((filter_key, self.prefix.join(",")));
        }

        if !self.peer_ip.is_empty() {
            let value = self.peer_ip.iter().map(ToString::to_string).join(",");
            specs.push(("peer_ips", value));
        }

        // Peer ASN filter - always use plural filter key for consistency.
        if !self.peer_asn.is_empty() {
            specs.push(("peer_asns", self.peer_asn.join(",")));
        }

        // Community filter - bgpkit-parser uses singular filter key name.
        if let Some(value) = self.build_community_filter_value()? {
            specs.push(("community", value));
        }

        if let Some(value) = &self.elem_type {
            specs.push(("type", value.to_string()));
        }

        // --- bgpkit-parser extended element filters ---
        if let Some(v) = &self.only_to_customer {
            // bgpkit-parser's filter key for the only-to-customer attribute is `otc`
            specs.push(("otc", v.clone()));
        }
        if let Some(v) = &self.next_hop {
            specs.push(("next_hop", v.clone()));
        }
        if let Some(v) = &self.origin {
            specs.push(("origin", v.clone()));
        }
        if let Some(v) = &self.local_pref {
            specs.push(("local_pref", v.clone()));
        }
        if let Some(v) = &self.med {
            specs.push(("med", v.clone()));
        }
        if let Some(v) = self.atomic_aggregate {
            specs.push(("atomic", v.to_string()));
        }
        if let Some(v) = &self.aggr_asn {
            specs.push(("aggr_asn", v.clone()));
        }
        if let Some(v) = &self.aggr_ip {
            specs.push(("aggr_ip", v.clone()));
        }
        if let Some(v) = &self.peer_bgp_id {
            specs.push(("peer_bgp_id", v.clone()));
        }

        match self.parse_start_end_strings() {
            Ok((start_ts, end_ts)) => {
                // Full start/end inputs, such as those from `monocle search`.
                specs.push(("start_ts", start_ts.to_string()));
                specs.push(("end_ts", end_ts.to_string()));
            }
            Err(_) => {
                // No complete time window: retain any individually supplied boundary.
                let time_lens = TimeLens::new();
                if let Some(value) = &self.start_ts {
                    let timestamp = time_lens.parse_time_string(value)?.timestamp();
                    specs.push(("start_ts", timestamp.to_string()));
                }
                if let Some(value) = &self.end_ts {
                    let timestamp = time_lens.parse_time_string(value)?.timestamp();
                    specs.push(("end_ts", timestamp.to_string()));
                }
            }
        }

        Ok(specs)
    }

    fn generic_filter_specs(&self) -> Result<Vec<(String, String)>> {
        self.generic_filters
            .iter()
            .map(|expression| {
                let (filter_type, filter_value) = Self::parse_generic_filter_expression(expression)?;
                if TIME_FILTER_KEYS.contains(&filter_type.as_str()) {
                    return Err(anyhow!(
                        "Invalid --filter '{}': time filter '{}' is not supported; use --start-ts, --end-ts, or --duration instead",
                        expression,
                        filter_type
                    ));
                }
                Filter::new(&filter_type, &filter_value).map_err(|error| {
                    anyhow!("Invalid --filter '{}': {}", expression, error)
                })?;
                Ok((filter_type, filter_value))
            })
            .collect()
    }

    fn parse_generic_filter_expression(expression: &str) -> Result<(String, String)> {
        let (filter_type, filter_value, negated) =
            if let Some((filter_type, filter_value)) = expression.split_once("!=") {
                (filter_type.trim(), filter_value.trim(), true)
            } else if let Some((filter_type, filter_value)) = expression.split_once('=') {
                (filter_type.trim(), filter_value.trim(), false)
            } else {
                return Err(anyhow!(
                    "Invalid --filter '{}': expression must contain '=' or '!='",
                    expression
                ));
            };

        if filter_type.is_empty() {
            return Err(anyhow!(
                "Invalid --filter '{}': filter key cannot be empty",
                expression
            ));
        }
        if filter_value.is_empty() {
            return Err(anyhow!(
                "Invalid --filter '{}': filter value cannot be empty",
                expression
            ));
        }

        let filter_value = if negated && MULTI_VALUE_FILTER_KEYS.contains(&filter_type) {
            filter_value
                .split(',')
                .map(|value| format!("!{}", value.trim()))
                .collect::<Vec<_>>()
                .join(",")
        } else if negated {
            format!("!{filter_value}")
        } else {
            filter_value.to_string()
        };

        Ok((filter_type.to_string(), filter_value))
    }

    /// Convert filters into BgpElem predicates using bgpkit-parser's canonical semantics.
    pub fn to_filters(&self) -> Result<Vec<Filter>> {
        let mut filters = Vec::new();
        for (filter_type, filter_value) in self.filter_specs()? {
            filters.push(Filter::new(filter_type, &filter_value)?);
        }
        for (filter_type, filter_value) in self.generic_filter_specs()? {
            filters.push(Filter::new(&filter_type, &filter_value)?);
        }
        Ok(filters)
    }

    /// Convert filters to a BgpkitParser.
    ///
    /// This method creates a parser with all filters applied. Multi-value filters
    /// use OR logic (matches ANY of the specified values). Negated values (prefixed
    /// with `!`) exclude matching elements.
    pub fn to_parser(&self, file_path: &str) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        let mut parser = BgpkitParser::new(file_path)?.disable_warnings();
        for (filter_type, filter_value) in self.filter_specs()? {
            parser = parser.add_filter(filter_type, &filter_value)?;
        }
        for (filter_type, filter_value) in self.generic_filter_specs()? {
            parser = parser.add_filter(&filter_type, &filter_value)?;
        }
        Ok(parser)
    }
}

// =============================================================================
// Lens
// =============================================================================

/// Parse lens for MRT file parsing operations
///
/// This lens provides high-level operations for parsing MRT files
/// with various filters applied, and optional progress tracking.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
/// use std::sync::Arc;
///
/// let lens = ParseLens::new();
/// let filters = ParseFilters::default();
///
/// // Simple parsing without progress tracking
/// let parser = lens.create_parser(&filters, "path/to/file.mrt")?;
/// for elem in parser {
///     println!("{}", elem);
/// }
///
/// // Parsing with progress tracking
/// let callback = Arc::new(|progress: ParseProgress| {
///     println!("{:?}", progress);
/// });
/// let elems = lens.parse_with_progress(&filters, "file.mrt", Some(callback))?;
/// ```
pub struct ParseLens;

impl ParseLens {
    /// Create a new parse lens
    pub fn new() -> Self {
        Self
    }

    /// Create a parser from filters and file path
    ///
    /// This returns a streaming parser that yields BGP elements one at a time.
    /// For progress tracking, use `parse_with_progress` instead.
    pub fn create_parser(
        &self,
        filters: &ParseFilters,
        file_path: &str,
    ) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        filters.to_parser(file_path)
    }

    /// Validate filters
    pub fn validate_filters(&self, filters: &ParseFilters) -> Result<()> {
        filters.validate()
    }

    /// Parse a file with progress tracking
    ///
    /// This method parses an MRT file and collects all elements into a Vec,
    /// reporting progress through the callback at regular intervals
    /// (every PARSE_PROGRESS_INTERVAL messages, currently 10,000).
    ///
    /// # Arguments
    ///
    /// * `filters` - Filters to apply during parsing
    /// * `file_path` - Path to the MRT file (local or remote)
    /// * `callback` - Optional callback to receive progress updates
    ///
    /// # Returns
    ///
    /// A vector of all parsed BGP elements
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
    /// use std::sync::Arc;
    ///
    /// let lens = ParseLens::new();
    /// let filters = ParseFilters::default();
    ///
    /// let callback = Arc::new(|progress: ParseProgress| {
    ///     match progress {
    ///         ParseProgress::Update { messages_processed, rate, .. } => {
    ///             println!("Processed {} messages ({:.0} msg/s)",
    ///                 messages_processed, rate.unwrap_or(0.0));
    ///         }
    ///         ParseProgress::Completed { total_messages, duration_secs, .. } => {
    ///             println!("Done: {} messages in {:.2}s", total_messages, duration_secs);
    ///         }
    ///         _ => {}
    ///     }
    /// });
    ///
    /// let elems = lens.parse_with_progress(&filters, "file.mrt", Some(callback))?;
    /// ```
    pub fn parse_with_progress(
        &self,
        filters: &ParseFilters,
        file_path: &str,
        callback: Option<ParseProgressCallback>,
    ) -> Result<Vec<BgpElem>> {
        let parser = self.create_parser(filters, file_path)?;

        // Notify start
        if let Some(ref cb) = callback {
            cb(ParseProgress::Started {
                file_path: file_path.to_string(),
            });
        }

        let start_time = Instant::now();
        let mut messages_processed: u64 = 0;
        let mut elements = Vec::new();

        for elem in parser {
            elements.push(elem);
            messages_processed += 1;

            // Report progress every PARSE_PROGRESS_INTERVAL messages
            if messages_processed.is_multiple_of(PARSE_PROGRESS_INTERVAL) {
                if let Some(ref cb) = callback {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let rate = if elapsed > 0.0 {
                        Some(messages_processed as f64 / elapsed)
                    } else {
                        None
                    };

                    cb(ParseProgress::Update {
                        messages_processed,
                        rate,
                        elapsed_secs: elapsed,
                    });
                }
            }
        }

        // Notify completion
        if let Some(ref cb) = callback {
            let duration_secs = start_time.elapsed().as_secs_f64();
            let rate = if duration_secs > 0.0 {
                Some(messages_processed as f64 / duration_secs)
            } else {
                None
            };

            cb(ParseProgress::Completed {
                total_messages: messages_processed,
                duration_secs,
                rate,
            });
        }

        Ok(elements)
    }

    /// Parse a file with progress tracking, processing elements through a handler
    ///
    /// Unlike `parse_with_progress`, this method processes elements one at a time
    /// through the provided handler function, avoiding the need to collect all
    /// elements into memory. This is more memory-efficient for large files.
    ///
    /// # Arguments
    ///
    /// * `filters` - Filters to apply during parsing
    /// * `file_path` - Path to the MRT file (local or remote)
    /// * `progress_callback` - Optional callback to receive progress updates
    /// * `element_handler` - Function called for each parsed element
    ///
    /// # Returns
    ///
    /// The total number of elements processed
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use monocle::lens::parse::{ParseLens, ParseFilters, ParseProgress};
    /// use std::sync::Arc;
    ///
    /// let lens = ParseLens::new();
    /// let filters = ParseFilters::default();
    ///
    /// let progress_cb = Arc::new(|progress: ParseProgress| {
    ///     if let ParseProgress::Update { messages_processed, .. } = progress {
    ///         println!("Processed {} messages", messages_processed);
    ///     }
    /// });
    ///
    /// let count = lens.parse_with_handler(
    ///     &filters,
    ///     "file.mrt",
    ///     Some(progress_cb),
    ///     |elem| {
    ///         // Process each element
    ///         println!("{}", elem);
    ///     },
    /// )?;
    /// println!("Total elements: {}", count);
    /// ```
    pub fn parse_with_handler<F>(
        &self,
        filters: &ParseFilters,
        file_path: &str,
        progress_callback: Option<ParseProgressCallback>,
        mut element_handler: F,
    ) -> Result<u64>
    where
        F: FnMut(BgpElem),
    {
        let parser = self.create_parser(filters, file_path)?;

        // Notify start
        if let Some(ref cb) = progress_callback {
            cb(ParseProgress::Started {
                file_path: file_path.to_string(),
            });
        }

        let start_time = Instant::now();
        let mut messages_processed: u64 = 0;

        for elem in parser {
            element_handler(elem);
            messages_processed += 1;

            // Report progress every PARSE_PROGRESS_INTERVAL messages
            if messages_processed.is_multiple_of(PARSE_PROGRESS_INTERVAL) {
                if let Some(ref cb) = progress_callback {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let rate = if elapsed > 0.0 {
                        Some(messages_processed as f64 / elapsed)
                    } else {
                        None
                    };

                    cb(ParseProgress::Update {
                        messages_processed,
                        rate,
                        elapsed_secs: elapsed,
                    });
                }
            }
        }

        // Notify completion
        if let Some(ref cb) = progress_callback {
            let duration_secs = start_time.elapsed().as_secs_f64();
            let rate = if duration_secs > 0.0 {
                Some(messages_processed as f64 / duration_secs)
            } else {
                None
            };

            cb(ParseProgress::Completed {
                total_messages: messages_processed,
                duration_secs,
                rate,
            });
        }

        Ok(messages_processed)
    }
}

impl Default for ParseLens {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mrt_type_inference() {
        assert_eq!(MrtType::infer(true), MrtType::Rib);
        assert_eq!(MrtType::infer(false), MrtType::Updates);
    }

    #[test]
    fn test_parse_progress_serialization() {
        // Test that progress types can be serialized for GUI communication
        let progress = ParseProgress::Started {
            file_path: "test.mrt".to_string(),
        };
        let json = serde_json::to_string(&progress).expect("Failed to serialize");
        assert!(json.contains("test.mrt"));

        let progress = ParseProgress::Update {
            messages_processed: 10000,
            rate: Some(5000.0),
            elapsed_secs: 2.0,
        };
        let json = serde_json::to_string(&progress).expect("Failed to serialize");
        assert!(json.contains("10000"));
        assert!(json.contains("messages_processed"));

        let progress = ParseProgress::Completed {
            total_messages: 50000,
            duration_secs: 10.0,
            rate: Some(5000.0),
        };
        let json = serde_json::to_string(&progress).expect("Failed to serialize");
        assert!(json.contains("50000"));
        assert!(json.contains("duration_secs"));
    }

    #[test]
    fn test_parse_progress_interval() {
        // Verify the progress interval constant is set correctly
        assert_eq!(PARSE_PROGRESS_INTERVAL, 10_000);
    }

    #[test]
    fn test_validate_asn_valid() {
        assert!(ParseFilters::validate_asn("13335").is_ok());
        assert!(ParseFilters::validate_asn("!13335").is_ok());
        assert!(ParseFilters::validate_asn("0").is_ok());
        assert!(ParseFilters::validate_asn("4294967295").is_ok()); // max u32
    }

    #[test]
    fn test_validate_asn_invalid() {
        assert!(ParseFilters::validate_asn("invalid").is_err());
        assert!(ParseFilters::validate_asn("!invalid").is_err());
        assert!(ParseFilters::validate_asn("-1").is_err());
        assert!(ParseFilters::validate_asn("4294967296").is_err()); // overflow u32
    }

    #[test]
    fn test_validate_prefix_valid() {
        assert!(ParseFilters::validate_prefix("1.1.1.0/24").is_ok());
        assert!(ParseFilters::validate_prefix("!1.1.1.0/24").is_ok());
        assert!(ParseFilters::validate_prefix("2001:db8::/32").is_ok());
        assert!(ParseFilters::validate_prefix("!2001:db8::/32").is_ok());
    }

    #[test]
    fn test_validate_prefix_invalid() {
        assert!(ParseFilters::validate_prefix("invalid").is_err());
        assert!(ParseFilters::validate_prefix("1.1.1.1").is_err()); // missing prefix length
        assert!(ParseFilters::validate_prefix("1.1.1.0/33").is_err()); // invalid prefix length
    }

    #[test]
    fn test_validate_community_valid() {
        assert!(ParseFilters::validate_community("13335:100").is_ok());
        assert!(ParseFilters::validate_community("!13335:100").is_ok());
        assert!(ParseFilters::validate_community("0:0").is_ok());
        assert!(ParseFilters::validate_community("65535:65535").is_ok());
        assert!(ParseFilters::validate_community("*:100").is_ok());
        assert!(ParseFilters::validate_community("13335:*").is_ok());
        assert!(ParseFilters::validate_community("*:*").is_ok());
        assert!(ParseFilters::validate_community("!*:100").is_ok());
        assert!(ParseFilters::validate_community("57866:104:31").is_ok());
        assert!(ParseFilters::validate_community("!57866:104:31").is_ok());
        assert!(ParseFilters::validate_community("*:104:31").is_ok());
        assert!(ParseFilters::validate_community("57866:*:*").is_ok());
        assert!(ParseFilters::validate_community("4294967295:0:1").is_ok());
    }

    #[test]
    fn test_validate_community_invalid() {
        assert!(ParseFilters::validate_community("13335").is_err());
        assert!(ParseFilters::validate_community("13335:").is_err());
        assert!(ParseFilters::validate_community(":100").is_err());
        assert!(ParseFilters::validate_community("65536:1").is_err());
        assert!(ParseFilters::validate_community("1:65536").is_err());
        assert!(ParseFilters::validate_community("abc:100").is_err());
        assert!(ParseFilters::validate_community("13*:100").is_err());
        assert!(ParseFilters::validate_community("*6:100").is_err());
        assert!(ParseFilters::validate_community("1:2:3:4").is_err());
        assert!(ParseFilters::validate_community("4294967296:1:1").is_err());
        assert!(ParseFilters::validate_community("1::3").is_err());
    }

    #[test]
    fn test_community_pattern_to_regex_body() {
        assert_eq!(
            ParseFilters::community_pattern_to_regex_body("1299:*").unwrap(),
            "1299:\\d+"
        );
        assert_eq!(
            ParseFilters::community_pattern_to_regex_body("*:100").unwrap(),
            "\\d+:100"
        );
        assert_eq!(
            ParseFilters::community_pattern_to_regex_body("57866:104:31").unwrap(),
            "57866:104:31"
        );
        assert_eq!(
            ParseFilters::community_pattern_to_regex_body("*:*:*").unwrap(),
            "\\d+:\\d+:\\d+"
        );
    }

    #[test]
    fn test_build_community_filter_value() {
        let filters = ParseFilters {
            communities: vec!["1299:*".to_string(), "*:100".to_string()],
            ..Default::default()
        };
        assert_eq!(
            filters.build_community_filter_value().unwrap(),
            Some("^(?:1299:\\d+|\\d+:100)$".to_string())
        );

        let filters = ParseFilters {
            communities: vec!["!1299:*".to_string(), "!*:100".to_string()],
            ..Default::default()
        };
        assert_eq!(
            filters.build_community_filter_value().unwrap(),
            Some("!^(?:1299:\\d+|\\d+:100)$".to_string())
        );
    }

    #[test]
    fn test_negation_consistency_valid() {
        // All positive
        let values = vec!["13335".to_string(), "15169".to_string()];
        assert!(ParseFilters::check_negation_consistency(&values, "test").is_ok());

        // All negative
        let values = vec!["!13335".to_string(), "!15169".to_string()];
        assert!(ParseFilters::check_negation_consistency(&values, "test").is_ok());

        // Single value (positive or negative is fine)
        let values = vec!["13335".to_string()];
        assert!(ParseFilters::check_negation_consistency(&values, "test").is_ok());
        let values = vec!["!13335".to_string()];
        assert!(ParseFilters::check_negation_consistency(&values, "test").is_ok());

        // Empty is fine
        let values: Vec<String> = vec![];
        assert!(ParseFilters::check_negation_consistency(&values, "test").is_ok());
    }

    #[test]
    fn test_negation_consistency_invalid() {
        // Mixed positive and negative
        let values = vec!["13335".to_string(), "!15169".to_string()];
        assert!(ParseFilters::check_negation_consistency(&values, "test").is_err());

        let values = vec!["!13335".to_string(), "15169".to_string()];
        assert!(ParseFilters::check_negation_consistency(&values, "test").is_err());
    }

    #[test]
    fn test_to_filters_uses_canonical_filter_specs() {
        let filters = ParseFilters {
            as_path: Some("^13335 ".to_string()),
            origin_asn: vec!["13335".to_string(), "15169".to_string()],
            prefix: vec!["0.0.0.0/0".to_string()],
            ..Default::default()
        };
        let actual = match filters.to_filters() {
            Ok(filters) => filters,
            Err(error) => panic!("filter conversion failed: {error}"),
        };

        for (filter_type, filter_value) in [
            ("as_path", "^13335 "),
            ("origin_asns", "13335,15169"),
            ("prefixes", "0.0.0.0/0"),
        ] {
            let expected = match Filter::new(filter_type, filter_value) {
                Ok(filter) => filter,
                Err(error) => panic!("invalid expected filter: {error}"),
            };
            assert!(actual.contains(&expected));
        }
        assert_eq!(actual.len(), 3);
    }

    #[test]
    fn test_parse_filters_validate() {
        // Valid filters
        let filters = ParseFilters {
            origin_asn: vec!["13335".to_string(), "15169".to_string()],
            prefix: vec!["1.1.1.0/24".to_string()],
            peer_asn: vec!["!174".to_string()],
            communities: vec!["*:100".to_string()],
            ..Default::default()
        };
        assert!(filters.validate().is_ok());

        // Invalid ASN
        let filters = ParseFilters {
            origin_asn: vec!["invalid".to_string()],
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Invalid prefix
        let filters = ParseFilters {
            prefix: vec!["not-a-prefix".to_string()],
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Invalid community
        let filters = ParseFilters {
            communities: vec!["not-a-community".to_string()],
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Mixed negation
        let filters = ParseFilters {
            origin_asn: vec!["13335".to_string(), "!15169".to_string()],
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Mixed community negation
        let filters = ParseFilters {
            communities: vec!["13335:100".to_string(), "!15169:100".to_string()],
            ..Default::default()
        };
        assert!(filters.validate().is_err());
    }

    #[test]
    fn test_validate_extended_filters_valid() {
        // All valid v0.19 filter values
        let filters = ParseFilters {
            only_to_customer: Some("65200".to_string()),
            next_hop: Some("10.0.0.1".to_string()),
            origin: Some("igp".to_string()),
            local_pref: Some("100".to_string()),
            med: Some("50".to_string()),
            atomic_aggregate: Some(true),
            aggr_asn: Some("65001".to_string()),
            aggr_ip: Some("192.168.1.1".to_string()),
            peer_bgp_id: Some("10.0.0.1".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_ok());

        // Presence wildcards
        let filters = ParseFilters {
            only_to_customer: Some("*".to_string()),
            next_hop: Some("*".to_string()),
            origin: Some("*".to_string()),
            local_pref: Some("*".to_string()),
            med: Some("*".to_string()),
            aggr_asn: Some("*".to_string()),
            aggr_ip: Some("*".to_string()),
            peer_bgp_id: Some("*".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_ok());

        // Absence wildcards
        let filters = ParseFilters {
            only_to_customer: Some("!*".to_string()),
            next_hop: Some("!*".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_ok());

        // Negated concrete values
        let filters = ParseFilters {
            only_to_customer: Some("!65200".to_string()),
            origin: Some("!igp".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_ok());

        // IPv6 next-hop
        let filters = ParseFilters {
            next_hop: Some("2001:db8::1".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_ok());
    }

    #[test]
    fn test_validate_extended_filters_invalid() {
        // Invalid u32 for otc
        let filters = ParseFilters {
            only_to_customer: Some("not-a-number".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Invalid IP for next_hop
        let filters = ParseFilters {
            next_hop: Some("999.999.999.999".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Invalid origin keyword
        let filters = ParseFilters {
            origin: Some("bgp".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Invalid u32 for local_pref
        let filters = ParseFilters {
            local_pref: Some("-1".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_err());

        // Invalid IP for aggr_ip
        let filters = ParseFilters {
            aggr_ip: Some("not-an-ip".to_string()),
            ..Default::default()
        };
        assert!(filters.validate().is_err());
    }

    #[test]
    fn test_extended_filters_wired_to_parser() {
        // Verify that the v0.19 filter specs are correctly emitted for
        // bgpkit-parser consumption via Filter::new.
        let filters = ParseFilters {
            only_to_customer: Some("65200".to_string()),
            next_hop: Some("10.0.0.1".to_string()),
            origin: Some("igp".to_string()),
            local_pref: Some("100".to_string()),
            med: Some("50".to_string()),
            atomic_aggregate: Some(true),
            aggr_asn: Some("65001".to_string()),
            aggr_ip: Some("192.168.1.1".to_string()),
            peer_bgp_id: Some("10.0.0.1".to_string()),
            ..Default::default()
        };

        let actual = filters.to_filters().expect("filter conversion failed");

        // Each extended filter should produce a corresponding Filter value
        // that bgpkit-parser recognises.
        for (filter_type, filter_value) in [
            ("otc", "65200"),
            ("next_hop", "10.0.0.1"),
            ("origin", "igp"),
            ("local_pref", "100"),
            ("med", "50"),
            ("atomic", "true"),
            ("aggr_asn", "65001"),
            ("aggr_ip", "192.168.1.1"),
            ("peer_bgp_id", "10.0.0.1"),
        ] {
            let expected = Filter::new(filter_type, filter_value)
                .unwrap_or_else(|e| panic!("invalid expected filter {filter_type}: {e}"));
            assert!(
                actual.contains(&expected),
                "missing filter for {filter_type}={filter_value}"
            );
        }
        assert_eq!(actual.len(), 9);
    }

    #[test]
    fn test_generic_filters_use_parser_semantics() {
        let filters = ParseFilters {
            generic_filters: vec![
                "ip_version=ipv6".to_string(),
                "origin_asns!=13335,15169".to_string(),
            ],
            ..Default::default()
        };

        let actual = filters.to_filters().expect("filter conversion failed");
        assert!(actual.contains(&Filter::new("ip_version", "ipv6").expect("valid IP filter")));
        assert!(actual.contains(
            &Filter::new("origin_asns", "!13335,!15169").expect("valid negated origin filter")
        ));
    }

    #[test]
    fn test_generic_filters_reject_time_filter_keys() {
        let filters = ParseFilters {
            generic_filters: vec!["start_ts=2026-01-01T00:00:00Z".to_string()],
            ..Default::default()
        };

        assert!(filters.validate().is_err());
    }

    #[test]
    fn test_generic_filters_serialize_when_present() {
        let filters = ParseFilters {
            generic_filters: vec!["ip_version=ipv6".to_string()],
            ..Default::default()
        };

        let serialized = serde_json::to_value(&filters).expect("filters should serialize");
        assert_eq!(
            serialized["generic_filters"],
            serde_json::json!(["ip_version=ipv6"])
        );
        assert!(serde_json::to_value(ParseFilters::default())
            .expect("empty filters should serialize")
            .get("generic_filters")
            .is_none());
    }
}
