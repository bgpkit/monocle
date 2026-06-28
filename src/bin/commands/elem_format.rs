//! Shared formatting utilities for BGP elements (BgpElem)
//!
//! This module provides common functionality for formatting BGP elements
//! across parse and search commands, including field selection and
//! multiple output format support.

use bgpkit_parser::BgpElem;
use monocle::utils::{OrderByField, OrderDirection, OutputFormat, TimestampFormat};
use serde_json::json;
use tabled::builder::Builder;
use tabled::settings::Style;

/// All available output fields for BGP elements
pub const AVAILABLE_FIELDS: &[&str] = &[
    "type",
    "timestamp",
    "peer_ip",
    "peer_asn",
    "prefix",
    "path_id",
    "as_path",
    "origin_asns",
    "origin",
    "next_hop",
    "local_pref",
    "med",
    "communities",
    "atomic",
    "aggr_asn",
    "aggr_ip",
    "collector",
];

/// Default fields to output for parse command (no collector)
pub const DEFAULT_FIELDS_PARSE: &[&str] = &[
    "type",
    "timestamp",
    "peer_ip",
    "peer_asn",
    "prefix",
    "as_path",
    "origin",
    "next_hop",
    "local_pref",
    "med",
    "communities",
    "atomic",
    "aggr_asn",
    "aggr_ip",
];

/// Default fields to output for search command (includes collector)
pub const DEFAULT_FIELDS_SEARCH: &[&str] = &[
    "type",
    "timestamp",
    "peer_ip",
    "peer_asn",
    "prefix",
    "as_path",
    "origin",
    "next_hop",
    "local_pref",
    "med",
    "communities",
    "atomic",
    "aggr_asn",
    "aggr_ip",
    "collector",
];

/// Format a collection of BgpElems as a tabled table with selected fields
pub fn format_elems_table(
    elems: &[(BgpElem, Option<String>)],
    fields: &[&str],
    time_format: TimestampFormat,
) -> String {
    let mut builder = Builder::default();

    // Add header row
    builder.push_record(fields.iter().copied());

    // Add data rows
    for (elem, collector) in elems {
        let row: Vec<String> = fields
            .iter()
            .map(|f| get_field_value_with_time_format(elem, f, collector.as_deref(), time_format))
            .collect();
        builder.push_record(row);
    }

    let mut table = builder.build();
    table.with(Style::rounded());
    table.to_string()
}

/// Generate help text for the fields argument
pub fn available_fields_help() -> String {
    format!(
        "Comma-separated list of fields to output. Available fields: {}",
        AVAILABLE_FIELDS.join(", ")
    )
}

/// Parse and validate the fields argument
/// Returns the validated fields. If fields_arg is None, returns the appropriate defaults.
/// `include_collector_default` should be true for search command, false for parse command.
pub fn parse_fields(
    fields_arg: &Option<String>,
    include_collector_default: bool,
) -> Result<Vec<&'static str>, String> {
    match fields_arg {
        None => {
            if include_collector_default {
                Ok(DEFAULT_FIELDS_SEARCH.to_vec())
            } else {
                Ok(DEFAULT_FIELDS_PARSE.to_vec())
            }
        }
        Some(fields_str) => {
            let requested: Vec<&str> = fields_str.split(',').map(|s| s.trim()).collect();
            let mut validated = Vec::new();

            for field in requested {
                if field.is_empty() {
                    continue;
                }
                if let Some(&valid_field) = AVAILABLE_FIELDS.iter().find(|&&f| f == field) {
                    validated.push(valid_field);
                } else {
                    return Err(format!(
                        "Unknown field '{}'. Available fields: {}",
                        field,
                        AVAILABLE_FIELDS.join(", ")
                    ));
                }
            }

            if validated.is_empty() {
                if include_collector_default {
                    Ok(DEFAULT_FIELDS_SEARCH.to_vec())
                } else {
                    Ok(DEFAULT_FIELDS_PARSE.to_vec())
                }
            } else {
                Ok(validated)
            }
        }
    }
}

/// Get the value of a specific field from a BgpElem
/// For the "collector" field, pass the collector value via the `collector` parameter.
/// Uses default Unix timestamp format for backward compatibility.
#[allow(dead_code)]
pub fn get_field_value(elem: &BgpElem, field: &str, collector: Option<&str>) -> String {
    get_field_value_with_time_format(elem, field, collector, TimestampFormat::Unix)
}

/// Get the value of a specific field from a BgpElem with configurable timestamp format
/// For the "collector" field, pass the collector value via the `collector` parameter.
pub fn get_field_value_with_time_format(
    elem: &BgpElem,
    field: &str,
    collector: Option<&str>,
    time_format: TimestampFormat,
) -> String {
    match field {
        "type" => {
            if elem.elem_type == bgpkit_parser::models::ElemType::ANNOUNCE {
                "A".to_string()
            } else {
                "W".to_string()
            }
        }
        "timestamp" => time_format.format_timestamp(elem.timestamp),
        "peer_ip" => elem.peer_ip.to_string(),
        "peer_asn" => elem.peer_asn.to_string(),
        "prefix" => elem.prefix.to_string(),
        "path_id" => elem
            .prefix
            .path_id
            .map(|path_id| path_id.to_string())
            .unwrap_or_default(),
        "as_path" => elem
            .as_path
            .as_ref()
            .map(|p| p.to_string())
            .unwrap_or_default(),
        "origin_asns" => elem
            .origin_asns
            .as_ref()
            .map(|asns| {
                asns.iter()
                    .map(|asn| asn.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default(),
        "origin" => elem
            .origin
            .as_ref()
            .map(|o| o.to_string())
            .unwrap_or_default(),
        "next_hop" => elem
            .next_hop
            .as_ref()
            .map(|h| h.to_string())
            .unwrap_or_default(),
        "local_pref" => elem
            .local_pref
            .as_ref()
            .map(|l| l.to_string())
            .unwrap_or_default(),
        "med" => elem.med.as_ref().map(|m| m.to_string()).unwrap_or_default(),
        "communities" => elem
            .communities
            .as_ref()
            .map(|c| {
                c.iter()
                    .map(|comm| comm.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default(),
        "atomic" => elem.atomic.to_string(),
        "aggr_asn" => elem
            .aggr_asn
            .as_ref()
            .map(|a| a.to_string())
            .unwrap_or_default(),
        "aggr_ip" => elem
            .aggr_ip
            .as_ref()
            .map(|i| i.to_string())
            .unwrap_or_default(),
        "collector" => collector.unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// Format a BgpElem according to the output format and selected fields.
/// The `collector` parameter provides the collector value for the "collector" field.
/// The `time_format` parameter controls how timestamps are displayed, including JSON
/// output: `Unix` (default) emits a numeric timestamp; `Rfc3339` emits an RFC 3339 string.
/// Note: For Table format, this returns an empty string - use format_elems_table() instead
/// after collecting all elements.
pub fn format_elem(
    elem: &BgpElem,
    output_format: OutputFormat,
    fields: &[&str],
    collector: Option<&str>,
    time_format: TimestampFormat,
) -> Option<String> {
    match output_format {
        OutputFormat::Json | OutputFormat::JsonLine => {
            let obj = build_json_object(elem, fields, collector, time_format);
            Some(serde_json::to_string(&obj).unwrap_or_else(|_| elem.to_string()))
        }
        OutputFormat::JsonPretty => {
            let obj = build_json_object(elem, fields, collector, time_format);
            Some(serde_json::to_string_pretty(&obj).unwrap_or_else(|_| elem.to_string()))
        }
        OutputFormat::Psv => {
            // Pipe-separated values (no header for backward compatibility)
            let values: Vec<String> = fields
                .iter()
                .map(|f| get_field_value_with_time_format(elem, f, collector, time_format))
                .collect();
            Some(values.join("|"))
        }
        OutputFormat::Table => {
            // Table format buffers all elements and formats at the end
            // Return None to signal caller should buffer this element
            None
        }
        OutputFormat::Markdown => {
            // Markdown table row
            let values: Vec<String> = fields
                .iter()
                .map(|f| get_field_value_with_time_format(elem, f, collector, time_format))
                .collect();
            Some(format!("| {} |", values.join(" | ")))
        }
    }
}

/// Build a JSON object with only the selected fields
/// The `collector` parameter provides the collector value for the "collector" field.
/// The `time_format` parameter controls the `timestamp` field representation:
/// `Unix` (default) emits a numeric f64; `Rfc3339` emits a formatted string.
pub fn build_json_object(
    elem: &BgpElem,
    fields: &[&str],
    collector: Option<&str>,
    time_format: TimestampFormat,
) -> serde_json::Value {
    // If all default parse fields are selected and no collector field, use the
    // original element serialization to preserve the full JSON shape (which
    // includes fields like `origin_asns`, `only_to_customer`, etc. that are not
    // listed in DEFAULT_FIELDS_PARSE). When RFC3339 timestamps are requested,
    // override just the `timestamp` key on top of the native serialization.
    if fields == DEFAULT_FIELDS_PARSE && !fields.contains(&"collector") {
        let mut obj = json!(elem);
        if time_format != TimestampFormat::Unix {
            if let Some(map) = obj.as_object_mut() {
                map.insert(
                    "timestamp".to_string(),
                    json!(time_format.format_timestamp(elem.timestamp)),
                );
            }
        }
        return obj;
    }

    let mut obj = serde_json::Map::new();
    for field in fields {
        let value = match *field {
            "type" => json!(elem.elem_type),
            "timestamp" => {
                if time_format == TimestampFormat::Unix {
                    json!(elem.timestamp)
                } else {
                    json!(time_format.format_timestamp(elem.timestamp))
                }
            }
            "peer_ip" => json!(elem.peer_ip.to_string()),
            "peer_asn" => json!(elem.peer_asn),
            "prefix" => json!(elem.prefix.to_string()),
            "path_id" => match elem.prefix.path_id {
                Some(path_id) => json!(path_id),
                None => serde_json::Value::Null,
            },
            "as_path" => match &elem.as_path {
                Some(p) => json!(p.to_string()),
                None => serde_json::Value::Null,
            },
            "origin_asns" => match &elem.origin_asns {
                Some(asns) => json!(asns.iter().map(|asn| asn.to_string()).collect::<Vec<_>>()),
                None => serde_json::Value::Null,
            },
            "origin" => match &elem.origin {
                Some(o) => json!(o.to_string()),
                None => serde_json::Value::Null,
            },
            "next_hop" => match &elem.next_hop {
                Some(h) => json!(h.to_string()),
                None => serde_json::Value::Null,
            },
            "local_pref" => match elem.local_pref {
                Some(l) => json!(l),
                None => serde_json::Value::Null,
            },
            "med" => match elem.med {
                Some(m) => json!(m),
                None => serde_json::Value::Null,
            },
            "communities" => match &elem.communities {
                Some(c) => json!(c.iter().map(|comm| comm.to_string()).collect::<Vec<_>>()),
                None => serde_json::Value::Null,
            },
            "atomic" => json!(elem.atomic),
            "aggr_asn" => match &elem.aggr_asn {
                Some(a) => json!(a),
                None => serde_json::Value::Null,
            },
            "aggr_ip" => match &elem.aggr_ip {
                Some(i) => json!(i.to_string()),
                None => serde_json::Value::Null,
            },
            "collector" => match collector {
                Some(c) => json!(c),
                None => serde_json::Value::Null,
            },
            _ => serde_json::Value::Null,
        };
        obj.insert((*field).to_string(), value);
    }

    serde_json::Value::Object(obj)
}

/// Get the header line for markdown output format.
/// Returns None for JSON, PSV (backward compatibility), and Table (uses tabled) formats.
pub fn get_header(output_format: OutputFormat, fields: &[&str]) -> Option<String> {
    match output_format {
        // JSON formats don't need headers
        OutputFormat::Json | OutputFormat::JsonPretty | OutputFormat::JsonLine => None,
        // PSV: no header for backward compatibility
        OutputFormat::Psv => None,
        // Table: handled by tabled, no streaming header needed
        OutputFormat::Table => None,
        // Markdown: needs header row with separator
        OutputFormat::Markdown => {
            let header = format!("| {} |", fields.join(" | "));
            let separator = format!(
                "| {} |",
                fields.iter().map(|_| "---").collect::<Vec<_>>().join(" | ")
            );
            Some(format!("{}\n{}", header, separator))
        }
    }
}

/// Sort a collection of BgpElems by the specified field and direction.
///
/// This function sorts the elements in place, which is more efficient than
/// creating a new vector.
pub fn sort_elems(
    elems: &mut [(BgpElem, Option<String>)],
    order_by: OrderByField,
    direction: OrderDirection,
) {
    elems.sort_by(|(a, _), (b, _)| {
        let cmp = match order_by {
            OrderByField::Timestamp => a
                .timestamp
                .partial_cmp(&b.timestamp)
                .unwrap_or(std::cmp::Ordering::Equal),
            OrderByField::Prefix => a.prefix.to_string().cmp(&b.prefix.to_string()),
            OrderByField::PeerIp => a.peer_ip.to_string().cmp(&b.peer_ip.to_string()),
            OrderByField::PeerAsn => a.peer_asn.cmp(&b.peer_asn),
            OrderByField::AsPath => {
                let a_path = a
                    .as_path
                    .as_ref()
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                let b_path = b
                    .as_path
                    .as_ref()
                    .map(|p| p.to_string())
                    .unwrap_or_default();
                a_path.cmp(&b_path)
            }
            OrderByField::NextHop => {
                let a_hop = a
                    .next_hop
                    .as_ref()
                    .map(|h| h.to_string())
                    .unwrap_or_default();
                let b_hop = b
                    .next_hop
                    .as_ref()
                    .map(|h| h.to_string())
                    .unwrap_or_default();
                a_hop.cmp(&b_hop)
            }
        };
        match direction {
            OrderDirection::Asc => cmp,
            OrderDirection::Desc => cmp.reverse(),
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bgpkit_parser::models::{AsPath, AsPathSegment, ElemType, NetworkPrefix, Origin};
    use bgpkit_parser::BgpElem;
    use std::net::{IpAddr, Ipv4Addr};
    use std::str::FromStr;

    fn test_elem() -> BgpElem {
        BgpElem {
            timestamp: 1234567890.0,
            elem_type: ElemType::ANNOUNCE,
            peer_ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            peer_asn: 65000.into(),
            prefix: NetworkPrefix::from_str("10.0.0.0/8").unwrap(),
            next_hop: Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))),
            as_path: Some(AsPath {
                segments: vec![AsPathSegment::AsSequence(vec![65000.into(), 65001.into()])],
            }),
            origin_asns: Some(vec![65001.into()]),
            origin: Some(Origin::IGP),
            local_pref: Some(100),
            med: Some(0),
            communities: None,
            atomic: false,
            aggr_asn: None,
            aggr_ip: None,
            only_to_customer: None,
            unknown: None,
            deprecated: None,
        }
    }

    #[test]
    fn test_json_timestamp_unix_is_numeric() {
        let elem = test_elem();
        let fields = vec!["timestamp", "prefix"];
        let obj = build_json_object(&elem, &fields, None, TimestampFormat::Unix);
        let ts = obj.get("timestamp").unwrap();
        assert_eq!(ts.as_f64().unwrap(), 1234567890.0);
    }

    #[test]
    fn test_json_timestamp_rfc3339_is_string() {
        let elem = test_elem();
        let fields = vec!["timestamp", "prefix"];
        let obj = build_json_object(&elem, &fields, None, TimestampFormat::Rfc3339);
        let ts = obj.get("timestamp").unwrap();
        assert!(ts.is_string(), "expected string for rfc3339 timestamp");
        let s = ts.as_str().unwrap();
        assert!(s.contains("2009"));
        assert!(s.contains('T'));
    }

    #[test]
    fn test_json_line_format_unix_vs_rfc3339() {
        let elem = test_elem();
        let fields = vec!["timestamp", "prefix"];

        let unix_out = format_elem(
            &elem,
            OutputFormat::JsonLine,
            &fields,
            None,
            TimestampFormat::Unix,
        )
        .unwrap();
        let rfc_out = format_elem(
            &elem,
            OutputFormat::JsonLine,
            &fields,
            None,
            TimestampFormat::Rfc3339,
        )
        .unwrap();

        let unix_json: serde_json::Value = serde_json::from_str(&unix_out).unwrap();
        let rfc_json: serde_json::Value = serde_json::from_str(&rfc_out).unwrap();

        assert!(unix_json.get("timestamp").unwrap().is_number());
        assert!(rfc_json.get("timestamp").unwrap().is_string());
    }

    #[test]
    fn test_table_timestamp_rfc3339() {
        let elem = test_elem();
        let value =
            get_field_value_with_time_format(&elem, "timestamp", None, TimestampFormat::Rfc3339);
        assert!(value.contains('T'));
    }

    #[test]
    fn test_default_fields_unix_falls_back_to_native_serialization() {
        let elem = test_elem();
        let obj = build_json_object(&elem, DEFAULT_FIELDS_PARSE, None, TimestampFormat::Unix);
        // With default fields, build_json_object should use native elem serialization
        // which contains fields NOT in DEFAULT_FIELDS_PARSE (e.g. `origin_asns`).
        // If the fallback broke, origin_asns would be absent from the output.
        assert!(
            obj.get("origin_asns").is_some(),
            "native serialization should include origin_asns (not in DEFAULT_FIELDS_PARSE)"
        );
        // Timestamp stays numeric for Unix
        assert!(obj.get("timestamp").unwrap().is_number());
    }

    #[test]
    fn test_default_fields_rfc3339_preserves_shape_overrides_timestamp() {
        let elem = test_elem();
        let obj = build_json_object(&elem, DEFAULT_FIELDS_PARSE, None, TimestampFormat::Rfc3339);
        // The full native shape should be preserved (origin_asns present)
        assert!(
            obj.get("origin_asns").is_some(),
            "rfc3339 should still use native serialization shape, not field-filtered map"
        );
        // But the timestamp should be overridden to a string
        let ts = obj.get("timestamp").unwrap();
        assert!(ts.is_string(), "rfc3339 timestamp should be a string");
        assert!(ts.as_str().unwrap().contains('T'));
    }
}
