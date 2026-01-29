//! Shared formatting utilities for BGP elements (BgpElem)
//!
//! This module provides common functionality for formatting BGP elements
//! across parse and search commands, including field selection and
//! multiple output format support.

use bgpkit_parser::BgpElem;
use monocle::lens::utils::OutputFormat;
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
pub fn format_elems_table(elems: &[(BgpElem, Option<String>)], fields: &[&str]) -> String {
    let mut builder = Builder::default();

    // Add header row
    builder.push_record(fields.iter().copied());

    // Add data rows
    for (elem, collector) in elems {
        let row: Vec<String> = fields
            .iter()
            .map(|f| get_field_value(elem, f, collector.as_deref()))
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
pub fn get_field_value(elem: &BgpElem, field: &str, collector: Option<&str>) -> String {
    match field {
        "type" => {
            if elem.elem_type == bgpkit_parser::models::ElemType::ANNOUNCE {
                "A".to_string()
            } else {
                "W".to_string()
            }
        }
        "timestamp" => elem.timestamp.to_string(),
        "peer_ip" => elem.peer_ip.to_string(),
        "peer_asn" => elem.peer_asn.to_string(),
        "prefix" => elem.prefix.to_string(),
        "as_path" => elem
            .as_path
            .as_ref()
            .map(|p| p.to_string())
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
/// Note: For Table format, this returns an empty string - use format_elems_table() instead
/// after collecting all elements.
pub fn format_elem(
    elem: &BgpElem,
    output_format: OutputFormat,
    fields: &[&str],
    collector: Option<&str>,
) -> Option<String> {
    match output_format {
        OutputFormat::Json | OutputFormat::JsonLine => {
            let obj = build_json_object(elem, fields, collector);
            Some(serde_json::to_string(&obj).unwrap_or_else(|_| elem.to_string()))
        }
        OutputFormat::JsonPretty => {
            let obj = build_json_object(elem, fields, collector);
            Some(serde_json::to_string_pretty(&obj).unwrap_or_else(|_| elem.to_string()))
        }
        OutputFormat::Psv => {
            // Pipe-separated values (no header for backward compatibility)
            let values: Vec<String> = fields
                .iter()
                .map(|f| get_field_value(elem, f, collector))
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
                .map(|f| get_field_value(elem, f, collector))
                .collect();
            Some(format!("| {} |", values.join(" | ")))
        }
    }
}

/// Build a JSON object with only the selected fields
/// The `collector` parameter provides the collector value for the "collector" field.
pub fn build_json_object(
    elem: &BgpElem,
    fields: &[&str],
    collector: Option<&str>,
) -> serde_json::Value {
    // If all default parse fields are selected and no collector field, use the original serialization
    if fields == DEFAULT_FIELDS_PARSE && !fields.contains(&"collector") {
        return json!(elem);
    }

    let mut obj = serde_json::Map::new();
    for field in fields {
        let value = match *field {
            "type" => json!(elem.elem_type),
            "timestamp" => json!(elem.timestamp),
            "peer_ip" => json!(elem.peer_ip.to_string()),
            "peer_asn" => json!(elem.peer_asn),
            "prefix" => json!(elem.prefix.to_string()),
            "as_path" => match &elem.as_path {
                Some(p) => json!(p.to_string()),
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
