//! Cisco `sh ip bgp` text dump parser.
//!
//! Parses the fixed-width column format produced by Cisco IOS routers.
//! Extracts field boundaries from the table header so numeric attributes do not
//! become indistinguishable from numeric AS-path segments.
//!
//! # Format
//!
//! ```text
//! BGP table version is N, local router ID is X.X.X.X, vrf id 0
//! Default local pref 100, local AS NNNN
//! ...
//!     Network          Next Hop            Metric LocPrf Weight Path
//!  *> 1.0.0.0/24       103.77.108.118           0             0 13335 i
//!  *=                  103.77.108.11            0             0 13335 i
//! ```
//!
//! Route-views `sh ip bgp` snapshots (e.g. `oix-full-snapshot-*.bz2`) use the
//! same fixed-width layout but omit the `BGP table version` / `local AS`
//! preamble. When the preamble is absent, `peer_ip` and `peer_asn` default to
//! the unspecified sentinel values `0.0.0.0` and AS0.

use bgpkit_parser::models::*;
use ipnet::IpNet;
use std::io::{BufRead, Read};
use std::net::IpAddr;

/// Byte offsets for the Cisco `Next Hop`, `Metric`, `LocPrf`, `Weight`, and
/// `Path` columns, respectively.
type ColumnPositions = (usize, usize, usize, usize, usize);

/// Header metadata extracted from the preamble.
#[derive(Debug, Clone)]
pub struct TextDumpHeader {
    pub table_version: Option<u64>,
    pub router_id: Option<IpAddr>,
    pub local_as: Option<u32>,
    column_positions: Option<ColumnPositions>,
}

// ── Detection ──────────────────────────────────────────────────────

/// Detect whether a reader contains a Cisco `sh ip bgp` text dump.
pub fn detect_text_dump<R: Read>(mut reader: R) -> std::io::Result<(bool, Vec<u8>)> {
    let mut buf = vec![0u8; 256];
    let n = reader.read(&mut buf)?;
    buf.truncate(n);

    let is_text = n > 0
        && buf[0].is_ascii_graphic()
        && std::str::from_utf8(&buf[..n.min(128)]).is_ok_and(|s| {
            s.contains("BGP table") || s.contains("Next Hop") || s.contains("Status codes:")
        });

    Ok((is_text, buf))
}

// ── Header parsing ─────────────────────────────────────────────────

/// Extract column offsets from the Cisco table header using whitespace split.
///
/// Splits the header line into whitespace-delimited tokens, merges known
/// multi-word field names (e.g. "Next" + "Hop" → "Next Hop"), and records
/// the byte position of each column in the original header line.
fn parse_column_header(line: &str) -> Option<ColumnPositions> {
    // Tokenize by whitespace: record (byte_position, token) for each word.
    let bytes = line.as_bytes();
    let mut tokens: Vec<(usize, &str)> = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // Skip leading whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        // Consume non-whitespace characters.
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let token = std::str::from_utf8(&bytes[start..i]).ok()?;
        tokens.push((start, token));
    }

    if tokens.is_empty() {
        return None;
    }

    // Merge known multi-word field names.
    // "Next" immediately followed by "Hop" → "Next Hop" starting at "Next".
    let mut columns: Vec<(usize, String)> = Vec::new();
    let mut skip = false;
    for idx in 0..tokens.len() {
        if skip {
            skip = false;
            continue;
        }
        let (pos, token) = tokens[idx];
        if token == "Next" && idx + 1 < tokens.len() && tokens[idx + 1].1 == "Hop" {
            columns.push((pos, "Next Hop".to_string()));
            skip = true;
        } else {
            columns.push((pos, token.to_string()));
        }
    }

    // Extract positions for the five columns we depend on.
    let target_names = ["Next Hop", "Metric", "LocPrf", "Weight", "Path"];
    let mut positions: [Option<usize>; 5] = [None; 5];

    for (pos, name) in &columns {
        for (i, target) in target_names.iter().enumerate() {
            if name == *target {
                positions[i] = Some(*pos);
            }
        }
    }

    let next_hop = positions[0]?;
    let metric = positions[1]?;
    let local_pref = positions[2]?;
    let weight = positions[3]?;
    let path = positions[4]?;

    if next_hop < metric && metric < local_pref && local_pref < weight && weight < path {
        Some((next_hop, metric, local_pref, weight, path))
    } else {
        None
    }
}

/// Parse the preamble to extract router metadata and fixed-width column offsets.
pub fn parse_header<R: BufRead>(reader: &mut R) -> std::io::Result<TextDumpHeader> {
    let mut header = TextDumpHeader {
        table_version: None,
        router_id: None,
        local_as: None,
        column_positions: None,
    };
    let mut buf = String::new();

    for _ in 0..64 {
        buf.clear();
        if reader.read_line(&mut buf)? == 0 {
            break;
        }
        let line = buf.trim_end();

        if line.starts_with("BGP table version") {
            if let Some(rest) = line.strip_prefix("BGP table version is ") {
                if let Some((ver_str, rest)) = rest.split_once(',') {
                    header.table_version = ver_str.trim().parse().ok();
                    if let Some(rid_part) = rest.split("local router ID is ").nth(1) {
                        if let Some((rid_str, _)) = rid_part.split_once(',') {
                            header.router_id = rid_str.trim().parse().ok();
                        }
                    }
                }
            }
        }

        if line.starts_with("Default local pref") {
            if let Some(rest) = line.split("local AS ").nth(1) {
                header.local_as = rest.trim().parse().ok();
            }
        }

        if let Some(column_positions) = parse_column_header(line) {
            header.column_positions = Some(column_positions);
            break;
        }
    }
    Ok(header)
}

// ── Fixed-width route parsing ──────────────────────────────────────

/// A parsed route entry from a single line.
#[derive(Debug, Clone)]
struct RouteEntry {
    prefix: String,
    next_hop: String,
    metric: Option<u32>,
    local_pref: Option<u32>,
    as_path: Vec<String>,
    origin: Option<Origin>,
}

fn parse_u32_column(line: &str, start: usize, end: usize) -> Option<u32> {
    line.get(start..end)?.trim().parse().ok()
}

fn shift_columns_left(columns: ColumnPositions) -> Option<ColumnPositions> {
    Some((
        columns.0.checked_sub(1)?,
        columns.1.checked_sub(1)?,
        columns.2.checked_sub(1)?,
        columns.3.checked_sub(1)?,
        columns.4.checked_sub(1)?,
    ))
}

/// Detect a "wrapped" prefix-only line (no route data, just a prefix after
/// the status flags). The next-hop column position is used to distinguish
/// prefix-only lines from full route lines: if the line segment at the
/// next-hop position is empty, the line carries only a prefix.
fn parse_wrapped_prefix_line(line: &str, next_hop_start: usize) -> Option<String> {
    // If the line extends into the next-hop column, check whether the
    // characters there form route data (digits/IP) or are just whitespace.
    if line.len() > next_hop_start {
        let rest = line[next_hop_start..].trim();
        if !rest.is_empty() {
            return None; // has route data → regular line, not a wrapped prefix
        }
    }

    let prefix = if line.len() > next_hop_start {
        line.get(3..next_hop_start)?
    } else {
        line.get(3..)?
    }
    .trim();
    prefix.parse::<IpNet>().ok().map(|_| prefix.to_string())
}

/// Parse a single Cisco route line using positions extracted from its column header.
fn parse_route_line(line: &str, columns: ColumnPositions) -> Option<RouteEntry> {
    let line = line.trim_end();
    if line.trim().is_empty() || line.trim_start().starts_with("Displayed") {
        return None;
    }

    let (next_hop_start, metric_start, local_pref_start, weight_start, path_start) = columns;
    let next_hop = line.get(next_hop_start..metric_start)?.trim();
    if next_hop.parse::<IpAddr>().is_err() {
        return None;
    }

    let prefix = line
        .get(3..next_hop_start)
        .unwrap_or_default()
        .trim()
        .to_string();
    let metric = parse_u32_column(line, metric_start, local_pref_start);
    let local_pref = parse_u32_column(line, local_pref_start, weight_start);
    // Cisco weight is a router-local attribute with no `BgpElem` representation.
    let _weight = parse_u32_column(line, weight_start, path_start);

    let path_tokens: Vec<&str> = line
        .get(path_start..)
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let origin = match path_tokens.last().copied() {
        Some("i") => Some(Origin::IGP),
        Some("e") => Some(Origin::EGP),
        Some("?") => Some(Origin::INCOMPLETE),
        _ => None,
    };
    let path_end = path_tokens.len() - usize::from(origin.is_some());
    let as_path = path_tokens[..path_end]
        .iter()
        .map(|token| (*token).to_string())
        .collect();

    Some(RouteEntry {
        prefix,
        next_hop: next_hop.to_string(),
        metric,
        local_pref,
        as_path,
        origin,
    })
}

// ── BgpElem construction ───────────────────────────────────────────

/// Convert path tokens into an AsPath.
///
/// AS-set delimiters (`{` / `}`) are silently dropped, flattening AS-sets
/// into plain AS-sequences. This is intentional: the parser aims to recover
/// the AS-level propagation path, and set membership is not preserved.
fn as_path_from_tokens(tokens: &[String]) -> AsPath {
    let mut asns: Vec<Asn> = Vec::new();
    for token in tokens {
        if token == "{" || token == "}" {
            continue;
        }
        if let Ok(asn) = token.parse::<u32>() {
            asns.push(Asn::from(asn));
        }
    }
    if asns.is_empty() {
        return AsPath {
            segments: vec![AsPathSegment::AsSequence(Default::default())].into(),
        };
    }
    AsPath {
        segments: vec![AsPathSegment::AsSequence(asns.into())].into(),
    }
}

fn entry_to_elem(
    entry: &RouteEntry,
    prefix_str: &str,
    peer_ip: IpAddr,
    peer_asn: u32,
    timestamp: f64,
) -> Option<BgpElem> {
    let prefix: IpNet = match prefix_str.parse() {
        Ok(p) => p,
        Err(_) => return None,
    };

    let network_prefix = NetworkPrefix::new(prefix, None);
    let next_hop: Option<IpAddr> = entry.next_hop.parse().ok();
    let as_path = Some(as_path_from_tokens(&entry.as_path));
    let origin = entry.origin;
    let local_pref = entry.local_pref;
    let med = entry.metric;

    let origin_asns: Option<Vec<Asn>> = as_path.as_ref().and_then(|ap| {
        ap.segments
            .last()
            .and_then(|seg| match seg {
                AsPathSegment::AsSequence(asns) => asns.last().copied(),
                _ => None,
            })
            .map(|asn| vec![asn])
    });

    Some(BgpElem {
        timestamp,
        elem_type: ElemType::ANNOUNCE,
        peer_ip,
        peer_asn: Asn::from(peer_asn),
        prefix: network_prefix,
        next_hop,
        as_path,
        origin_asns,
        origin,
        local_pref,
        med,
        communities: None,
        atomic: false,
        aggr_asn: None,
        aggr_ip: None,
        only_to_customer: None,
        unknown: None,
        deprecated: None,
        peer_bgp_id: None,
    })
}

// ── Timestamp inference ────────────────────────────────────────────

/// Try to extract a Unix timestamp from a file path or URL.
///
/// Recognises route-views-style timestamps like
/// `oix-full-snapshot-2026-07-01-0000.bz2` (`YYYY-MM-DD-HHMM`, using the
/// embedded time of day) and PCH-style date components like
/// `...2026.07.01.gz` (`YYYY.MM.DD`, noon UTC). Returns `None` when no
/// recognisable date is found.
pub fn infer_timestamp_from_path(path: &str) -> Option<f64> {
    let bytes = path.as_bytes();

    // Scan for YYYY-MM-DD-HHMM pattern (route-views snapshot filenames).
    for (idx, window) in bytes.windows(15).enumerate() {
        if window[0].is_ascii_digit()
            && window[1].is_ascii_digit()
            && window[2].is_ascii_digit()
            && window[3].is_ascii_digit()
            && window[4] == b'-'
            && window[5].is_ascii_digit()
            && window[6].is_ascii_digit()
            && window[7] == b'-'
            && window[8].is_ascii_digit()
            && window[9].is_ascii_digit()
            && window[10] == b'-'
            && window[11].is_ascii_digit()
            && window[12].is_ascii_digit()
            && window[13].is_ascii_digit()
            && window[14].is_ascii_digit()
        {
            let y: Option<i32> = std::str::from_utf8(&window[0..4])
                .ok()
                .and_then(|s| s.parse().ok());
            let m: Option<u32> = std::str::from_utf8(&window[5..7])
                .ok()
                .and_then(|s| s.parse().ok());
            let d: Option<u32> = std::str::from_utf8(&window[8..10])
                .ok()
                .and_then(|s| s.parse().ok());
            let hh: Option<u32> = std::str::from_utf8(&window[11..13])
                .ok()
                .and_then(|s| s.parse().ok());
            let mm: Option<u32> = std::str::from_utf8(&window[13..15])
                .ok()
                .and_then(|s| s.parse().ok());
            // Require a separator before the date to avoid matching digit
            // runs inside longer numbers.
            if idx > 0 && bytes[idx - 1].is_ascii_digit() {
                continue;
            }
            if let (Some(y), Some(m), Some(d), Some(hh), Some(mm)) = (y, m, d, hh, mm) {
                if let Some(dt) = chrono::NaiveDate::from_ymd_opt(y, m, d)
                    .and_then(|date| date.and_hms_opt(hh, mm, 0))
                {
                    return Some(dt.and_utc().timestamp() as f64);
                }
            }
        }
    }

    // Scan for YYYY.MM.DD pattern (PCH file URLs).
    for window in bytes.windows(10) {
        if window.len() == 10
            && window[0].is_ascii_digit()
            && window[1].is_ascii_digit()
            && window[2].is_ascii_digit()
            && window[3].is_ascii_digit()
            && window[4] == b'.'
            && window[5].is_ascii_digit()
            && window[6].is_ascii_digit()
            && window[7] == b'.'
            && window[8].is_ascii_digit()
            && window[9].is_ascii_digit()
        {
            let y: i32 = std::str::from_utf8(&window[0..4]).ok()?.parse().ok()?;
            let m: u32 = std::str::from_utf8(&window[5..7]).ok()?.parse().ok()?;
            let d: u32 = std::str::from_utf8(&window[8..10]).ok()?.parse().ok()?;
            // Use noon UTC to avoid DST boundary issues.
            let dt = chrono::NaiveDate::from_ymd_opt(y, m, d)?.and_hms_opt(12, 0, 0)?;
            return Some(dt.and_utc().timestamp() as f64);
        }
    }
    None
}

// ── Top-level parse ────────────────────────────────────────────────

pub fn parse_text_dump<R: BufRead>(reader: R) -> std::io::Result<Vec<BgpElem>> {
    parse_text_dump_with_timestamp(reader, 0.0)
}

pub fn parse_text_dump_with_timestamp<R: BufRead>(
    mut reader: R,
    timestamp: f64,
) -> std::io::Result<Vec<BgpElem>> {
    let header = parse_header(&mut reader)?;
    let column_positions = match header.column_positions {
        Some(positions) => positions,
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing Cisco BGP table column header",
            ));
        }
    };
    // Route-views style snapshots omit the `BGP table version` / `local AS`
    // preamble. Fall back to the unspecified sentinels rather than rejecting
    // the dump: `0.0.0.0` and AS0 carry no peer identity.
    let peer_ip = header
        .router_id
        .unwrap_or_else(|| IpAddr::from([0, 0, 0, 0]));
    let peer_asn = header.local_as.unwrap_or(0);

    let wrapped_column_positions = shift_columns_left(column_positions);

    let mut buf = String::new();
    let mut current_prefix = String::new();
    let mut entries: Vec<(String, RouteEntry)> = Vec::new();

    while reader.read_line(&mut buf)? > 0 {
        let line = buf.trim_end().to_string();
        buf.clear();

        if line.is_empty() {
            continue;
        }

        let entry = parse_route_line(&line, column_positions).or_else(|| {
            wrapped_column_positions.and_then(|positions| parse_route_line(&line, positions))
        });
        if let Some(entry) = entry {
            if !entry.prefix.is_empty() {
                current_prefix = entry.prefix.clone();
            }
            entries.push((current_prefix.clone(), entry));
            continue;
        }

        if let Some(prefix) = parse_wrapped_prefix_line(&line, column_positions.0) {
            current_prefix = prefix;
            continue;
        }
    }

    let mut elems: Vec<BgpElem> = Vec::with_capacity(entries.len());
    for (prefix, entry) in &entries {
        if prefix.is_empty() {
            continue;
        }
        if let Some(elem) = entry_to_elem(entry, prefix, peer_ip, peer_asn, timestamp) {
            elems.push(elem);
        }
    }

    Ok(elems)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_COLUMNS: ColumnPositions = (21, 41, 48, 55, 62);
    const TABLE_HEADER: &str = "    Network          Next Hop            Metric LocPrf Weight Path";

    fn fixed_width_route(
        prefix: &str,
        next_hop: &str,
        metric: &str,
        local_pref: &str,
        weight: &str,
        path: &str,
    ) -> String {
        format!(
            " *> {:<17}{:<20}{:>7}{:>7}{:>7}{}",
            prefix, next_hop, metric, local_pref, weight, path
        )
    }

    fn parsed_route(line: &str) -> RouteEntry {
        match parse_route_line(line, TEST_COLUMNS) {
            Some(entry) => entry,
            None => panic!("expected a valid fixed-width route line"),
        }
    }

    #[test]
    fn test_detect_text_dump_positive() {
        let data = b"BGP table version is 123, local router ID is 1.2.3.4, vrf id 0\n";
        let (is_text, buf) = match detect_text_dump(&data[..]) {
            Ok(result) => result,
            Err(error) => panic!("text dump detection failed: {error}"),
        };
        assert!(is_text);
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_detect_text_dump_negative() {
        let data = vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x0d, 0x00, 0x01];
        let (is_text, _buf) = match detect_text_dump(&data[..]) {
            Ok(result) => result,
            Err(error) => panic!("text dump detection failed: {error}"),
        };
        assert!(!is_text);
    }

    #[test]
    fn test_parse_header() {
        let preamble = "\
BGP table version is 1350657, local router ID is 45.112.180.132, vrf id 0
Default local pref 100, local AS 3856
Status codes:  s suppressed, d damped, h history, * valid, > best, = multipath,
               i internal, r RIB-failure, S Stale, R Removed
Nexthop codes: @NNN nexthop's vrf id, < announce-nh-self
Origin codes:  i - IGP, e - EGP, ? - incomplete
RPKI validation codes: V valid, I invalid, N Not found

    Network          Next Hop            Metric LocPrf Weight Path
";
        let header = match parse_header(&mut preamble.as_bytes()) {
            Ok(header) => header,
            Err(error) => panic!("header parsing failed: {error}"),
        };
        let expected_router_id = match "45.112.180.132".parse() {
            Ok(router_id) => router_id,
            Err(error) => panic!("invalid expected router ID: {error}"),
        };
        assert_eq!(header.table_version, Some(1350657));
        assert_eq!(header.router_id, Some(expected_router_id));
        assert_eq!(header.local_as, Some(3856));
        assert_eq!(header.column_positions, Some(TEST_COLUMNS));
    }

    #[test]
    fn test_parse_route_line_basic() {
        let line = fixed_width_route("1.0.0.0/24", "103.77.108.11", "0", "", "0", "13335 i");
        let entry = parsed_route(&line);
        assert_eq!(entry.prefix, "1.0.0.0/24");
        assert_eq!(entry.next_hop, "103.77.108.11");
        assert_eq!(entry.metric, Some(0));
        assert_eq!(entry.local_pref, None);
        assert_eq!(entry.origin, Some(Origin::IGP));
        assert_eq!(entry.as_path, vec!["13335"]);
    }

    #[test]
    fn test_parse_route_line_continuation() {
        let line = fixed_width_route("", "103.77.108.118", "0", "", "0", "13335 i");
        let entry = parsed_route(&line);
        assert!(entry.prefix.is_empty());
        assert_eq!(entry.next_hop, "103.77.108.118");
        assert_eq!(entry.as_path, vec!["13335"]);
    }

    #[test]
    fn test_parse_route_line_uses_fixed_width_attributes() {
        let line = fixed_width_route("0.0.0.0/0", "103.77.108.116", "0", "", "0", "134942 4755 i");
        let entry = parsed_route(&line);
        assert_eq!(entry.metric, Some(0));
        assert_eq!(entry.local_pref, None);
        assert_eq!(entry.as_path, vec!["134942", "4755"]);
    }

    #[test]
    fn test_parse_route_line_multi_asn() {
        let line = fixed_width_route(
            "1.0.0.0/24",
            "103.77.108.116",
            "10",
            "200",
            "0",
            "134942 4755 i",
        );
        let entry = parsed_route(&line);
        assert_eq!(entry.metric, Some(10));
        assert_eq!(entry.local_pref, Some(200));
        assert_eq!(entry.as_path, vec!["134942", "4755"]);
        assert_eq!(entry.origin, Some(Origin::IGP));
    }

    #[test]
    fn test_parse_route_line_origin_codes() {
        let prefix = "1.0.0.0/24";
        let next_hop = "10.0.0.1";
        assert_eq!(
            parsed_route(&fixed_width_route(prefix, next_hop, "0", "", "0", "100 i")).origin,
            Some(Origin::IGP)
        );
        assert_eq!(
            parsed_route(&fixed_width_route(prefix, next_hop, "0", "", "0", "100 e")).origin,
            Some(Origin::EGP)
        );
        assert_eq!(
            parsed_route(&fixed_width_route(prefix, next_hop, "0", "", "0", "100 ?")).origin,
            Some(Origin::INCOMPLETE)
        );
    }

    #[test]
    fn test_parse_route_line_no_numeric_attrs() {
        let line = fixed_width_route("1.0.0.0/24", "10.0.0.1", "", "", "", "100 i");
        let entry = parsed_route(&line);
        assert_eq!(entry.next_hop, "10.0.0.1");
        assert_eq!(entry.as_path, vec!["100"]);
        assert!(entry.metric.is_none());
        assert!(entry.local_pref.is_none());
    }

    #[test]
    fn test_parse_text_dump_preserves_continuation_prefix() {
        let first = fixed_width_route("0.0.0.0/0", "103.77.108.116", "0", "", "0", "134942 4755 i");
        let continuation = fixed_width_route("", "103.77.108.118", "0", "", "0", "13335 i");
        let dump = format!(
            "BGP table version is 1350657, local router ID is 45.112.180.132, vrf id 0\nDefault local pref 100, local AS 3856\n\n{TABLE_HEADER}\n{first}\n{continuation}\nDisplayed 1 routes and 2 total paths\n"
        );
        let elems = match parse_text_dump(dump.as_bytes()) {
            Ok(elems) => elems,
            Err(error) => panic!("text dump parsing failed: {error}"),
        };
        assert_eq!(elems.len(), 2);
        assert_eq!(elems[0].prefix, elems[1].prefix);
        assert_eq!(elems[0].med, Some(0));
        assert_eq!(elems[0].local_pref, None);
        assert_eq!(elems[0].origin_asns, Some(vec![Asn::from(4755u32)]));
        assert_eq!(elems[1].origin_asns, Some(vec![Asn::from(13335u32)]));
    }

    #[test]
    fn test_parse_text_dump_supports_wrapped_prefix() {
        let dump = format!(
            "BGP table version is 1350657, local router ID is 45.112.180.132, vrf id 0\nDefault local pref 100, local AS 3856\n\n{TABLE_HEADER}\n *  103.85.157.176/29\n                    103.77.108.116           0             0 134942 58715 152125 i\nDisplayed 1 routes and 1 total paths\n"
        );
        let elems = match parse_text_dump(dump.as_bytes()) {
            Ok(elems) => elems,
            Err(error) => panic!("text dump parsing failed: {error}"),
        };
        assert_eq!(elems.len(), 1);
        assert_eq!(elems[0].origin_asns, Some(vec![Asn::from(152125u32)]));
    }

    #[test]
    fn test_entry_to_elem() {
        let entry = RouteEntry {
            prefix: "1.0.0.0/24".into(),
            next_hop: "103.77.108.11".into(),
            metric: Some(0),
            local_pref: Some(0),
            as_path: vec!["13335".into()],
            origin: Some(Origin::IGP),
        };
        let peer_ip = match "45.112.180.132".parse() {
            Ok(peer_ip) => peer_ip,
            Err(error) => panic!("invalid expected peer IP: {error}"),
        };
        let elem = match entry_to_elem(&entry, "1.0.0.0/24", peer_ip, 3856, 0.0) {
            Some(elem) => elem,
            None => panic!("BgpElem conversion failed"),
        };
        assert_eq!(elem.peer_ip.to_string(), "45.112.180.132");
        assert_eq!(u32::from(elem.peer_asn), 3856);
        assert_eq!(elem.origin, Some(Origin::IGP));
        assert_eq!(elem.origin_asns, Some(vec![Asn::from(13335u32)]));
    }

    const ROUTE_VIEWS_HEADER: &str =
        "   Network            Next Hop            Metric LocPrf Weight Path";

    #[test]
    fn test_detect_text_dump_route_views() {
        let data = b"Status codes: s suppressed, d damped, h history, * valid, > best, i - internal,\n              r RIB-failure, S Stale\nOrigin codes: i - IGP, e - EGP, ? - incomplete\n";
        let (is_text, _buf) = match detect_text_dump(&data[..]) {
            Ok(result) => result,
            Err(error) => panic!("text dump detection failed: {error}"),
        };
        assert!(is_text);
    }

    #[test]
    fn test_parse_route_views_dump() {
        let dump = format!(
            "Status codes: s suppressed, d damped, h history, * valid, > best, i - internal,\n              r RIB-failure, S Stale\nOrigin codes: i - IGP, e - EGP, ? - incomplete\n\n{ROUTE_VIEWS_HEADER}\n*  0.0.0.0/0          147.28.0.3               0      0      0 3130 174 i\n*  1.0.0.0/24         12.0.1.63                0      0      0 7018 13335 i\n*  1.0.0.0/24         129.250.1.71          2001      0      0 2914 13335 i\n"
        );
        let elems = match parse_text_dump(dump.as_bytes()) {
            Ok(elems) => elems,
            Err(error) => panic!("route-views dump parsing failed: {error}"),
        };
        assert_eq!(elems.len(), 3);
        // No router ID / local AS preamble: unspecified sentinel values.
        assert_eq!(elems[0].peer_ip.to_string(), "0.0.0.0");
        assert_eq!(u32::from(elems[0].peer_asn), 0);

        assert_eq!(elems[0].prefix.prefix.to_string(), "0.0.0.0/0");
        assert_eq!(elems[0].med, Some(0));
        assert_eq!(elems[0].origin_asns, Some(vec![Asn::from(174u32)]));

        assert_eq!(elems[2].prefix.prefix.to_string(), "1.0.0.0/24");
        assert_eq!(elems[2].med, Some(2001));
        assert_eq!(elems[2].origin, Some(Origin::IGP));
        assert_eq!(elems[2].origin_asns, Some(vec![Asn::from(13335u32)]));
    }

    #[test]
    fn test_infer_timestamp_route_views_path() {
        let ts = infer_timestamp_from_path(
            "https://archive.routeviews.org/oix-route-views/2026.07/oix-full-snapshot-2026-07-01-0000.bz2",
        );
        // 2026-07-01 00:00:00 UTC
        assert_eq!(ts, Some(1782864000.0));
    }

    #[test]
    fn test_infer_timestamp_pch_path() {
        let ts = infer_timestamp_from_path(
            "https://www.pch.net/resources/data/routing-tables/2026/2026.07/rib-ipv4.2026.07.01.gz",
        );
        // 2026-07-01 12:00:00 UTC (noon convention for date-only paths)
        assert_eq!(ts, Some(1782907200.0));
    }

    #[test]
    fn test_infer_timestamp_no_date() {
        assert_eq!(infer_timestamp_from_path("/tmp/some-file.gz"), None);
    }
}
