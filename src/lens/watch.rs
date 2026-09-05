//! Watch lens: live BGP message streams (RIPE RIS Live).
//!
//! Normalizes a live RIS Live websocket feed into a stream of `BgpElem`s, so
//! the same filter grammar, output formats, and MRT recording used by
//! `monocle parse` apply to live data. See `src/bin/commands/watch.rs` for the
//! CLI wiring.
//!
//! # Filter pushdown
//!
//! RIS Live supports server-side subscription filters (`RisSubscribe`). Watch
//! translates user filters into the subscription wherever possible so the
//! server sends only relevant messages; filters the API cannot express are
//! still applied client-side. See `WatchFilters::to_ris_subscribe`.

use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use bgpkit_parser::parse_ris_live_message;
use bgpkit_parser::parser::filter::Filterable;
use bgpkit_parser::BgpElem;
use bgpkit_parser::RisSubscribe;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};

/// Public RIS Live websocket endpoint.
pub const RIS_LIVE_URL: &str = "ws://ris-live.ripe.net/v1/ws/?client=monocle";

/// Reconnect backoff after an abnormal websocket close or IO error.
pub const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// Live-stream filters for the watch command.
///
/// A subset of [`crate::lens::parse::ParseFilters`] dimensions that have a
/// meaningful live interpretation. Time-window filters are intentionally
/// absent: a live stream has no archive window; use `monocle parse` or
/// `monocle search` for historical ranges.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct WatchFilters {
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
    #[cfg_attr(feature = "cli", clap(short = 'C', long, value_delimiter = ','))]
    #[serde(default)]
    pub communities: Vec<String>,

    /// Filter by elem type: announce (a) or withdraw (w)
    #[cfg_attr(feature = "cli", clap(short = 'm', long, value_enum))]
    #[serde(default)]
    pub elem_type: Option<crate::lens::parse::ParseElemType>,

    /// Filter by AS path regex string (applied client-side; RIS Live path
    /// patterns are not regular expressions)
    #[cfg_attr(feature = "cli", clap(short = 'a', long))]
    #[serde(default)]
    pub as_path: Option<String>,
}

/// Which filters were pushed down to the RIS Live subscription, for display.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushdownReport {
    /// RRC host filter (`--collector`/`--host`)
    pub host: Option<String>,
    /// Origin ASN patterns sent as `path` (e.g. `2906$`)
    pub origin_path_patterns: Vec<String>,
    /// Prefix subscriptions sent (one subscription per prefix)
    pub prefixes: Vec<String>,
    /// Peer IPs sent as `peer`
    pub peers: Vec<String>,
    /// `require` value sent (`announcements`/`withdrawals`)
    pub require: Option<String>,
}

impl WatchFilters {
    /// Return true when no filter dimension is set, i.e. the subscription
    /// would request the full feed.
    pub fn is_empty(&self) -> bool {
        self.origin_asn.is_empty()
            && self.prefix.is_empty()
            && self.peer_ip.is_empty()
            && self.peer_asn.is_empty()
            && self.communities.is_empty()
            && self.elem_type.is_none()
            && self.as_path.is_none()
    }

    /// Build the server-side RIS Live subscription from these filters.
    ///
    /// `host` scopes the subscription to a single RRC (server-side). Returns
    /// the subscription plus a report of which filter dimensions were pushed
    /// down. Dimensions the RIS Live API cannot express are omitted here and
    /// must be applied client-side via [`WatchFilters::compile_client_filters`].
    pub fn to_ris_subscribe(&self, host: Option<&str>) -> Result<(RisSubscribe, PushdownReport)> {
        let mut sub = RisSubscribe::new();
        let mut report = PushdownReport::default();

        if let Some(h) = host {
            sub = sub.host(h);
            report.host = Some(h.to_string());
        }

        // origin_asn -> path pattern "N$" (server-side)
        for value in &self.origin_asn {
            let (asn, negated) = strip_negation(value);
            let pattern = if negated {
                format!("!{asn}$")
            } else {
                format!("{asn}$")
            };
            sub = sub.path(&pattern);
            report.origin_path_patterns.push(pattern);
        }

        // prefix -> per-prefix subscriptions (server-side, with more/less-specific)
        for value in &self.prefix {
            let (raw, negated) = strip_negation(value);
            let net = IpNet::from_str(&raw).map_err(|e| anyhow!("invalid prefix '{raw}': {e}"))?;
            if negated {
                bail!(
                    "negative prefix filters are not supported for live subscriptions: '{value}'"
                );
            }
            let mut p = sub.prefix(net);
            if self.include_sub {
                p = p.more_specific(true);
            }
            if self.include_super {
                p = p.less_specific(true);
            }
            sub = p;
            report.prefixes.push(raw);
        }

        // peer_ip -> peer (server-side)
        for peer in &self.peer_ip {
            sub = sub.peer(*peer);
            report.peers.push(peer.to_string());
        }

        // elem_type -> require (server-side)
        if let Some(t) = &self.elem_type {
            let require = match t {
                crate::lens::parse::ParseElemType::A => "announcements",
                crate::lens::parse::ParseElemType::W => "withdrawals",
            };
            sub = sub.require(require);
            report.require = Some(require.to_string());
        }

        Ok((sub, report))
    }

    /// Compile the filters that must run client-side into parser `Filter`s.
    ///
    /// Pushed-down dimensions are excluded: the server already applied them.
    /// Applied client-side: peer_asn, communities, as_path regex, and negative
    /// origin filters (RIS Live `!` path patterns are unreliable across
    /// deployments, so we keep exact semantics locally).
    pub fn compile_client_filters(&self) -> Result<Vec<bgpkit_parser::parser::filter::Filter>> {
        use bgpkit_parser::parser::filter::Filter;

        let mut filters = Vec::new();

        for value in &self.peer_asn {
            let v = strip_negation(value);
            filters.push(Filter::new("peer_asns", &v.0)?);
        }

        for value in &self.communities {
            let (raw, negated) = strip_negation(value);
            let spec = if negated { format!("!{raw}") } else { raw };
            filters.push(Filter::new("community", &spec)?);
        }

        if let Some(pattern) = &self.as_path {
            filters.push(Filter::new("as_path", pattern)?);
        }

        // Negative origin filters: server pushdown used "!N$" path patterns for
        // them, but we additionally apply an exact client-side filter to keep
        // semantics identical to parse/search.
        for value in &self.origin_asn {
            if strip_negation(value).1 {
                filters.push(Filter::new("origin_asns", &strip_negation(value).0)?);
            }
        }

        Ok(filters)
    }

    /// Apply client-side filters to a parsed element.
    pub fn matches(
        &self,
        elem: &BgpElem,
        filters: &[bgpkit_parser::parser::filter::Filter],
    ) -> bool {
        elem.match_filters(filters)
    }
}

fn strip_negation(value: &str) -> (String, bool) {
    let v = value.trim();
    if let Some(stripped) = v.strip_prefix('!') {
        (stripped.trim().to_string(), true)
    } else {
        (v.to_string(), false)
    }
}

/// A parsed live message: zero or more elements plus the RRC host it came from.
#[derive(Debug)]
pub struct LiveMessage {
    pub host: Option<String>,
    pub elems: Vec<BgpElem>,
}

/// Parse a raw RIS Live websocket text frame into elements.
///
/// Non-data frames (errors, `ris_subscribe_ok`, state messages) return an
/// empty element list rather than an error, matching the feed's best-effort
/// nature.
pub fn parse_live_frame(msg_str: &str) -> Result<LiveMessage> {
    // Extract the originating host without a full serde pass of every frame
    // shape; parse_ris_live_message handles the heavy lifting.
    let host = extract_host(msg_str);
    let elems = parse_ris_live_message(msg_str).unwrap_or_default();
    Ok(LiveMessage { host, elems })
}

fn extract_host(msg_str: &str) -> Option<String> {
    // RIS Live data frames carry "data": { "host": "rrcXX", ... }. A cheap
    // substring scan avoids rejecting frames whose outer shape changes.
    let marker = "\"host\"";
    let idx = msg_str.find(marker)?;
    let rest = &msg_str[idx + marker.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let quote = after.find('"')?;
    let value = &after[quote + 1..];
    let end = value.find('"')?;
    Some(value[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgpkit_parser::RisLiveClientMessage;

    #[test]
    fn test_empty_filters_report_full_feed() {
        let filters = WatchFilters::default();
        assert!(filters.is_empty());
    }

    #[test]
    fn test_pushdown_origin_asn() {
        let filters = WatchFilters {
            origin_asn: vec!["2906".to_string()],
            ..Default::default()
        };
        let (sub, report) = filters.to_ris_subscribe(None).unwrap();
        assert!(report.origin_path_patterns.contains(&"2906$".to_string()));
        assert!(!sub.to_json_string().is_empty());
    }

    #[test]
    fn test_pushdown_prefix_and_host() {
        let filters = WatchFilters {
            prefix: vec!["1.1.1.0/24".to_string()],
            include_sub: true,
            ..Default::default()
        };
        let (sub, report) = filters.to_ris_subscribe(Some("rrc00")).unwrap();
        assert_eq!(report.host.as_deref(), Some("rrc00"));
        assert_eq!(report.prefixes, vec!["1.1.1.0/24".to_string()]);
        let json = sub.to_json_string();
        assert!(json.contains("rrc00"));
        assert!(json.contains("1.1.1.0/24"));
        assert!(json.contains("\"moreSpecific\":true"));
    }

    #[test]
    fn test_negative_prefix_rejected() {
        let filters = WatchFilters {
            prefix: vec!["!1.1.1.0/24".to_string()],
            ..Default::default()
        };
        assert!(filters.to_ris_subscribe(None).is_err());
    }

    #[test]
    fn test_elem_type_pushdown() {
        let filters = WatchFilters {
            elem_type: Some(crate::lens::parse::ParseElemType::A),
            ..Default::default()
        };
        let (_, report) = filters.to_ris_subscribe(None).unwrap();
        assert_eq!(report.require.as_deref(), Some("announcements"));
    }

    #[test]
    fn test_negative_origin_keeps_client_filter() {
        let filters = WatchFilters {
            origin_asn: vec!["!13335".to_string()],
            ..Default::default()
        };
        let filters_compiled = filters.compile_client_filters().unwrap();
        assert!(!filters_compiled.is_empty());
    }

    #[test]
    fn test_extract_host() {
        let frame = r#"{"type":"ris_message","data":{"timestamp":1700000000,"peer":"1.1.1.1","host":"rrc10","path":[1,2,3],"announcements":[]}}"#;
        assert_eq!(extract_host(frame).as_deref(), Some("rrc10"));
        assert!(extract_host("garbage").is_none());
    }

    #[test]
    fn test_parse_live_frame_non_data() {
        // Error frames parse to empty element lists, not errors.
        let frame = r#"{"type":"ris_error","data":{"message":"bad subscription"}}"#;
        let msg = parse_live_frame(frame).unwrap();
        assert!(msg.elems.is_empty());
    }
}
