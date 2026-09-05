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
//! pushes down `host`, `prefix`, `peer`, and `require` to reduce traffic, but
//! pushdown is only an optimization: RIS selects whole UPDATE messages while
//! elements expand per prefix, and some predicates (e.g. origin ASN with
//! AS_SET origins) have no equivalent server-side pattern. Every element-level
//! predicate also runs client-side with bgpkit-parser semantics, so watch
//! matches exactly what `monocle parse` would match.

use std::net::IpAddr;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Result};
use bgpkit_parser::parser::filter::Filterable;
use bgpkit_parser::{parse_ris_live_message_raw, BgpElem, RisSubscribe};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};

/// Public RIS Live websocket endpoint (TLS).
pub const RIS_LIVE_URL: &str = "wss://ris-live.ripe.net/v1/ws/?client=monocle";

/// Reconnect backoff after an abnormal websocket close or IO error.
pub const RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// Live-stream filters for the watch command.
///
/// A subset of [`crate::lens::parse::ParseFilters`] dimensions that have a
/// meaningful live interpretation, with the same option names and semantics.
/// Time-window filters are intentionally absent: a live stream has no archive
/// window; use `monocle parse` or `monocle search` for historical ranges.
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
    #[serde(default)]
    pub elem_type: Option<crate::lens::parse::ParseElemType>,

    /// Filter by AS path regex string
    #[cfg_attr(feature = "cli", clap(short = 'a', long))]
    #[serde(default)]
    pub as_path: Option<String>,
}

/// Which filters were pushed down to the RIS Live subscription, for display.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushdownReport {
    /// RRC host filter (`--host`)
    pub host: Option<String>,
    /// Prefix subscriptions sent (one subscription per prefix)
    pub prefixes: Vec<String>,
    /// Peer IPs sent (one subscription per peer)
    pub peers: Vec<String>,
    /// `require` value sent (`announcements`/`withdrawals`)
    pub require: Option<String>,
}

/// A single RIS Live subscription to send after connecting.
///
/// `RisSubscribe` holds single `prefix`/`peer` values, so multi-value filters
/// need one subscription per combination; each combination is sent as its own
/// `ris_subscribe` on the same socket (RIS Live semantics: subscriptions add
/// up as OR).
#[derive(Debug)]
pub struct SubscriptionPlan {
    /// One subscription per prefix (or the single no-prefix subscription).
    pub subscriptions: Vec<RisSubscribe>,
    pub report: PushdownReport,
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

    /// Build the server-side RIS Live subscription plan from these filters.
    ///
    /// `host` scopes every subscription to a single RRC. Only dimensions that
    /// reduce traffic without changing match semantics are pushed down:
    /// `host`, `prefix`, `peer`, and `require`:
    ///
    /// - Origin ASNs are NOT pushed down: the RIS `path` pattern `N$` does
    ///   not match AS_SET origins, so pushdown could discard updates that
    ///   client-side semantics accept.
    /// - Negative prefixes are NOT pushed down (no server-side negation);
    ///   they are enforced by the client-side filters.
    /// - Because `RisSubscribe` holds single `prefix`/`peer` values, the plan
    ///   carries one subscription per (prefix, peer) combination.
    /// - Both specificity flags are always set explicitly: RIS defaults
    ///   `moreSpecific` to true when omitted, which would silently widen an
    ///   exact-match `--prefix`.
    pub fn to_subscription_plan(&self, host: Option<&str>) -> Result<SubscriptionPlan> {
        let mut report = PushdownReport::default();
        if let Some(h) = host {
            report.host = Some(h.to_string());
        }

        let positive_prefixes: Vec<&String> = self
            .prefix
            .iter()
            .filter(|p| !p.trim_start().starts_with('!'))
            .collect();
        let mut prefix_nets = Vec::new();
        for value in &positive_prefixes {
            let net = IpNet::from_str(value.trim())
                .map_err(|e| anyhow!("invalid prefix '{}': {e}", value.trim()))?;
            prefix_nets.push(net);
            report.prefixes.push(value.trim().to_string());
        }

        for peer in &self.peer_ip {
            report.peers.push(peer.to_string());
        }

        if let Some(t) = &self.elem_type {
            report.require = Some(match t {
                crate::lens::parse::ParseElemType::A => "announcements".to_string(),
                crate::lens::parse::ParseElemType::W => "withdrawals".to_string(),
            });
        }

        // Cross product of prefix x peer (empty = absent dimension).
        let prefixes: Vec<Option<IpNet>> = if prefix_nets.is_empty() {
            vec![None]
        } else {
            prefix_nets.into_iter().map(Some).collect()
        };
        let peers: Vec<Option<IpAddr>> = if self.peer_ip.is_empty() {
            vec![None]
        } else {
            self.peer_ip.iter().copied().map(Some).collect()
        };

        let mut subscriptions = Vec::new();
        for pfx in &prefixes {
            for peer in &peers {
                let mut sub = RisSubscribe::new();
                if let Some(h) = host {
                    sub = sub.host(h);
                }
                if let Some(net) = pfx {
                    sub = sub
                        .prefix(*net)
                        .more_specific(self.include_sub)
                        .less_specific(self.include_super);
                }
                if let Some(ip) = peer {
                    sub = sub.peer(*ip);
                }
                if let Some(req) = &report.require {
                    sub = sub.require(req);
                }
                subscriptions.push(sub);
            }
        }

        Ok(SubscriptionPlan {
            subscriptions,
            report,
        })
    }

    /// Compile the complete client-side element filters.
    ///
    /// These carry the actual match semantics (identical to `monocle parse`)
    /// and validate all values: invalid ASNs, communities, or prefixes fail
    /// here before any connection is made. Call before opening a recording
    /// file or subscribing.
    pub fn compile_client_filters(&self) -> Result<Vec<bgpkit_parser::parser::filter::Filter>> {
        use bgpkit_parser::parser::filter::Filter;

        let mut filters = Vec::new();

        if !self.origin_asn.is_empty() {
            filters.push(Filter::new("origin_asns", &self.origin_asn.join(","))?);
        }

        if !self.prefix.is_empty() {
            let key = match (self.include_super, self.include_sub) {
                (false, false) => "prefixes",
                (true, false) => "prefixes_super",
                (false, true) => "prefixes_sub",
                (true, true) => "prefixes_super_sub",
            };
            filters.push(Filter::new(key, &self.prefix.join(","))?);
        }

        if !self.peer_ip.is_empty() {
            let value = self
                .peer_ip
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            filters.push(Filter::new("peer_ips", &value)?);
        }

        if !self.peer_asn.is_empty() {
            filters.push(Filter::new("peer_asns", &self.peer_asn.join(","))?);
        }

        // Community filtering reuses parse's canonical conversion (wildcards,
        // negation consistency, OR within the dimension) instead of
        // per-value predicates, which match_filters would combine with AND.
        if !self.communities.is_empty() {
            let spec = self.canonical_community_spec()?;
            filters.push(Filter::new("community", &spec)?);
        }

        if let Some(t) = &self.elem_type {
            filters.push(Filter::new("type", &t.to_string())?);
        }

        if let Some(pattern) = &self.as_path {
            filters.push(Filter::new("as_path", pattern)?);
        }

        Ok(filters)
    }

    /// Canonical community filter spec, identical to parse's semantics
    /// (single filter value, OR across alternatives, `*` wildcards,
    /// consistent negation).
    fn canonical_community_spec(&self) -> Result<String> {
        use crate::lens::parse::ParseFilters;
        ParseFilters::check_negation_consistency(&self.communities, "community")?;
        let is_negated = self
            .communities
            .first()
            .map(|v| v.starts_with('!'))
            .unwrap_or(false);
        let mut pattern_bodies = Vec::with_capacity(self.communities.len());
        for pattern in &self.communities {
            pattern_bodies.push(ParseFilters::community_pattern_to_regex_body(pattern)?);
        }
        let regex = format!("^(?:{})$", pattern_bodies.join("|"));
        if is_negated {
            Ok(format!("!{regex}"))
        } else {
            Ok(regex)
        }
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

/// A parsed live websocket frame.
#[derive(Debug)]
pub enum LiveFrame {
    /// A data frame carrying zero or more elements plus the RRC host.
    Data(LiveMessage),
    /// `ris_subscribe_ok`: the server accepted a subscription.
    SubscribeOk,
    /// `ris_error` with its message text.
    Error(String),
    /// Any other control frame (e.g. `ris_rrc_list`, `pong`).
    Other,
}

/// A parsed live data message.
#[derive(Debug)]
pub struct LiveMessage {
    pub host: Option<String>,
    pub elems: Vec<BgpElem>,
}

/// Classify and parse a raw RIS Live websocket text frame.
///
/// Data frames are parsed from the raw BGP message bytes (the subscription
/// requests `includeRaw`), so every attribute the `BgpElem` model carries is
/// populated — including large communities, which the JSON-projected parser
/// drops. Attributes outside the `BgpElem` model (e.g. ORIGINATOR_ID,
/// CLUSTER_LIST) are still not represented downstream; recordings are
/// `BgpElem`-faithful (exactly what filtering and output see), not
/// byte-faithful to the original UPDATE. Element parse failures are returned
/// as `Err` so the caller can report them instead of silently dropping live
/// updates.
pub fn parse_live_frame(msg_str: &str) -> Result<LiveFrame> {
    match extract_string_field(msg_str, "type").as_deref() {
        Some("ris_subscribe_ok") => return Ok(LiveFrame::SubscribeOk),
        Some("ris_error") => {
            return Ok(LiveFrame::Error(
                extract_string_field(msg_str, "message").unwrap_or_else(|| "unknown error".into()),
            ))
        }
        Some(_) if msg_str.contains("\"raw\"") => {}
        Some(_) => return Ok(LiveFrame::Other),
        None => {}
    }

    let host = extract_host(msg_str);
    match parse_ris_live_message_raw(msg_str) {
        Ok(elems) => Ok(LiveFrame::Data(LiveMessage { host, elems })),
        Err(e) => Err(anyhow!("failed to parse RIS Live message: {e}")),
    }
}

fn extract_string_field(msg_str: &str, field: &str) -> Option<String> {
    // Cheap scans avoid a full serde pass per frame and tolerate outer-shape
    // changes. Frames are small, so two scans are not a hotspot.
    let marker = format!("\"{field}\"");
    let idx = msg_str.find(&marker)?;
    let rest = &msg_str[idx + marker.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let quote = after.find('"')?;
    let value = &after[quote + 1..];
    let end = value.find('"')?;
    Some(value[..end].to_string())
}

fn extract_host(msg_str: &str) -> Option<String> {
    extract_string_field(msg_str, "host")
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
    fn test_origin_not_pushed_down() {
        let filters = WatchFilters {
            origin_asn: vec!["2906".to_string()],
            ..Default::default()
        };
        let plan = filters.to_subscription_plan(None).unwrap();
        assert_eq!(plan.subscriptions.len(), 1);
        assert!(!plan.subscriptions[0].to_json_string().contains("path"));
        // but the client-side filter carries it
        let client = filters.compile_client_filters().unwrap();
        assert!(!client.is_empty());
    }

    #[test]
    fn test_pushdown_prefix_and_host() {
        let filters = WatchFilters {
            prefix: vec!["1.1.1.0/24".to_string()],
            include_sub: true,
            ..Default::default()
        };
        let plan = filters.to_subscription_plan(Some("rrc00")).unwrap();
        assert_eq!(plan.report.host.as_deref(), Some("rrc00"));
        assert_eq!(plan.report.prefixes, vec!["1.1.1.0/24".to_string()]);
        let json = plan.subscriptions[0].to_json_string();
        assert!(json.contains("rrc00"));
        assert!(json.contains("1.1.1.0/24"));
        assert!(json.contains("\"moreSpecific\":true"));
    }

    #[test]
    fn test_prefix_specificity_set_explicitly() {
        // RIS defaults moreSpecific=true when omitted; exact-match --prefix
        // must send moreSpecific=false explicitly.
        let filters = WatchFilters {
            prefix: vec!["1.1.1.0/24".to_string()],
            ..Default::default()
        };
        let plan = filters.to_subscription_plan(None).unwrap();
        let json = plan.subscriptions[0].to_json_string();
        assert!(json.contains("\"moreSpecific\":false"));
        assert!(json.contains("\"lessSpecific\":false"));
    }

    #[test]
    fn test_multi_prefix_multi_peer_expands_subscriptions() {
        let filters = WatchFilters {
            prefix: vec!["1.1.1.0/24".to_string(), "8.8.8.0/24".to_string()],
            peer_ip: vec!["192.0.2.1".parse().unwrap(), "192.0.2.2".parse().unwrap()],
            ..Default::default()
        };
        let plan = filters.to_subscription_plan(None).unwrap();
        assert_eq!(plan.subscriptions.len(), 4);
        assert_eq!(plan.report.prefixes.len(), 2);
        assert_eq!(plan.report.peers.len(), 2);
        let jsons: Vec<String> = plan
            .subscriptions
            .iter()
            .map(|s| s.to_json_string())
            .collect();
        assert!(jsons
            .iter()
            .any(|j| j.contains("1.1.1.0/24") && j.contains("192.0.2.1")));
        assert!(jsons
            .iter()
            .any(|j| j.contains("8.8.8.0/24") && j.contains("192.0.2.2")));
    }

    #[test]
    fn test_negative_prefix_client_side_only() {
        let filters = WatchFilters {
            prefix: vec!["!1.1.1.0/24".to_string()],
            ..Default::default()
        };
        let plan = filters.to_subscription_plan(None).unwrap();
        assert!(plan.report.prefixes.is_empty());
        // enforced client-side instead
        assert!(!filters.compile_client_filters().unwrap().is_empty());
    }

    #[test]
    fn test_elem_type_pushdown_and_client_filter() {
        let filters = WatchFilters {
            elem_type: Some(crate::lens::parse::ParseElemType::A),
            ..Default::default()
        };
        let plan = filters.to_subscription_plan(None).unwrap();
        assert_eq!(plan.report.require.as_deref(), Some("announcements"));
        // elem type also re-checked client-side (mixed UPDATEs)
        assert!(!filters.compile_client_filters().unwrap().is_empty());
    }

    #[test]
    fn test_invalid_origin_rejected_at_compile() {
        let filters = WatchFilters {
            origin_asn: vec!["bogus".to_string()],
            ..Default::default()
        };
        assert!(filters.compile_client_filters().is_err());
    }

    #[test]
    fn test_invalid_prefix_rejected_at_compile() {
        let filters = WatchFilters {
            prefix: vec!["not-a-prefix".to_string()],
            ..Default::default()
        };
        assert!(filters.compile_client_filters().is_err());
    }

    #[test]
    fn test_mixed_positive_negative_origin_rejected() {
        let filters = WatchFilters {
            origin_asn: vec!["13335".to_string(), "!15169".to_string()],
            ..Default::default()
        };
        assert!(filters.compile_client_filters().is_err());
    }

    #[test]
    fn test_community_spec_or_semantics() {
        // multiple communities must yield ONE filter with OR semantics,
        // matching parse behavior, not one filter per value (AND).
        let filters = WatchFilters {
            communities: vec!["13335:100".to_string(), "15169:*".to_string()],
            ..Default::default()
        };
        let compiled = filters.compile_client_filters().unwrap();
        assert_eq!(compiled.len(), 1);
        let spec = filters.canonical_community_spec().unwrap();
        assert!(spec.contains("|"));
    }

    #[test]
    fn test_mixed_community_negation_rejected() {
        let filters = WatchFilters {
            communities: vec!["13335:100".to_string(), "!15169:1".to_string()],
            ..Default::default()
        };
        assert!(filters.compile_client_filters().is_err());
    }

    #[test]
    fn test_extract_host() {
        let frame = r#"{"type":"ris_message","data":{"timestamp":1700000000,"peer":"1.1.1.1","host":"rrc10","path":[1,2,3],"announcements":[]}}"#;
        assert_eq!(extract_host(frame).as_deref(), Some("rrc10"));
        assert!(extract_host("garbage").is_none());
    }

    #[test]
    fn test_control_frames_classified() {
        let ok = parse_live_frame(r#"{"type":"ris_subscribe_ok","data":{}}"#).unwrap();
        assert!(matches!(ok, LiveFrame::SubscribeOk));

        let err = parse_live_frame(r#"{"type":"ris_error","data":{"message":"bad subscription"}}"#)
            .unwrap();
        match err {
            LiveFrame::Error(msg) => assert_eq!(msg, "bad subscription"),
            other => panic!("expected error frame, got {other:?}"),
        }

        let other = parse_live_frame(r#"{"type":"pong","data":null}"#).unwrap();
        assert!(matches!(other, LiveFrame::Other));
    }
}
