//! Common utility functions for lens modules
//!
//! This module provides shared utility functions used across multiple lenses,
//! particularly for formatting output in tables and deserializing query parameters.

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

// =============================================================================
// Serde Deserializers for Query Parameters
// =============================================================================

/// Deserialize a string or vec of strings into a Vec<String>
///
/// This allows query parameters to accept either `param=value` or `param=v1&param=v2`
pub fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrVec;

    impl<'de> Visitor<'de> for StringOrVec {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or array of strings")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![value.to_string()])
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![value])
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(value) = seq.next_element()? {
                vec.push(value);
            }
            Ok(vec)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Vec::new())
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Vec::new())
        }
    }

    deserializer.deserialize_any(StringOrVec)
}

/// Deserialize a u32 or vec of u32s into a Vec<u32>
///
/// This allows query parameters to accept either `asn=12345` or `asn=12345&asn=67890`
/// Also handles string representations of numbers from query parameters.
pub fn u32_or_vec<'de, D>(deserializer: D) -> Result<Vec<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    struct U32OrVec;

    impl<'de> Visitor<'de> for U32OrVec {
        type Value = Vec<u32>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a u32, string representing u32, or array of u32s")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(vec![value as u32])
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 {
                return Err(de::Error::custom("ASN cannot be negative"));
            }
            Ok(vec![value as u32])
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value
                .parse::<u32>()
                .map(|v| vec![v])
                .map_err(|_| de::Error::custom(format!("Invalid ASN: {}", value)))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(value) = seq.next_element::<serde_json::Value>()? {
                let num = match value {
                    serde_json::Value::Number(n) => n
                        .as_u64()
                        .map(|v| v as u32)
                        .ok_or_else(|| de::Error::custom("Invalid number"))?,
                    serde_json::Value::String(s) => s
                        .parse::<u32>()
                        .map_err(|_| de::Error::custom(format!("Invalid ASN: {}", s)))?,
                    _ => return Err(de::Error::custom("Expected number or string")),
                };
                vec.push(num);
            }
            Ok(vec)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Vec::new())
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Vec::new())
        }
    }

    deserializer.deserialize_any(U32OrVec)
}

/// Deserialize a u32 from string or number
///
/// This allows query parameters to accept `asn=12345` as a string
pub fn u32_from_str<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    struct U32FromStr;

    impl<'de> Visitor<'de> for U32FromStr {
        type Value = u32;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a u32 or string representing u32")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value as u32)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 {
                return Err(de::Error::custom("ASN cannot be negative"));
            }
            Ok(value as u32)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            value
                .parse::<u32>()
                .map_err(|_| de::Error::custom(format!("Invalid ASN: {}", value)))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(U32FromStr)
}

/// Deserialize an optional u32 from string or number
///
/// This allows query parameters to accept `asn=12345` as a string
pub fn option_u32_from_str<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionU32FromStr;

    impl<'de> Visitor<'de> for OptionU32FromStr {
        type Value = Option<u32>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a u32, string representing u32, or null")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Some(value as u32))
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value < 0 {
                return Err(de::Error::custom("ASN cannot be negative"));
            }
            Ok(Some(value as u32))
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if value.is_empty() {
                return Ok(None);
            }
            value
                .parse::<u32>()
                .map(Some)
                .map_err(|_| de::Error::custom(format!("Invalid ASN: {}", value)))
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(None)
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: Deserializer<'de>,
        {
            deserializer.deserialize_any(OptionU32FromStr)
        }
    }

    deserializer.deserialize_any(OptionU32FromStr)
}

/// Deserialize a boolean from string or bool
///
/// This allows query parameters to accept `param=true` or `param=false` as strings
pub fn bool_from_str<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    struct BoolFromStr;

    impl<'de> Visitor<'de> for BoolFromStr {
        type Value = bool;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a boolean or string representing a boolean")
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            match value.to_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Ok(true),
                "false" | "0" | "no" | "off" | "" => Ok(false),
                _ => Err(de::Error::custom(format!(
                    "Invalid boolean value: {}",
                    value
                ))),
            }
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value != 0)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value != 0)
        }
    }

    deserializer.deserialize_any(BoolFromStr)
}

// =============================================================================
// Output Format and Display Utilities
// =============================================================================

/// Default maximum length for name display in tables
pub const DEFAULT_NAME_MAX_LEN: usize = 20;

/// Unified output format for all lens commands
///
/// This enum provides a consistent set of output formats that can be used
/// across all monocle commands. Commands that don't support a particular
/// format should return an error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// Pretty table with borders (default)
    #[default]
    Table,
    /// Markdown table format
    Markdown,
    /// Compact JSON (single line per object)
    Json,
    /// Pretty-printed JSON with indentation
    JsonPretty,
    /// JSON Lines format (one JSON object per line, for streaming)
    JsonLine,
    /// Pipe-separated values with header
    Psv,
}

impl OutputFormat {
    /// Check if this is a JSON variant
    pub fn is_json(&self) -> bool {
        matches!(self, Self::Json | Self::JsonPretty | Self::JsonLine)
    }

    /// Check if this is a table variant
    pub fn is_table(&self) -> bool {
        matches!(self, Self::Table | Self::Markdown)
    }

    /// Get a list of all format names for help text
    pub fn all_names() -> &'static [&'static str] {
        &[
            "table",
            "markdown",
            "json",
            "json-pretty",
            "json-line",
            "psv",
        ]
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table => write!(f, "table"),
            Self::Markdown => write!(f, "markdown"),
            Self::Json => write!(f, "json"),
            Self::JsonPretty => write!(f, "json-pretty"),
            Self::JsonLine => write!(f, "json-line"),
            Self::Psv => write!(f, "psv"),
        }
    }
}

// =============================================================================
// Ordering Utilities for BGP Elements
// =============================================================================

/// Fields available for ordering BGP element output
///
/// This enum provides the list of fields that can be used for sorting
/// BGP elements in the output of parse and search commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
#[cfg_attr(feature = "cli", clap(rename_all = "snake_case"))]
pub enum OrderByField {
    /// Order by timestamp (default)
    #[default]
    Timestamp,
    /// Order by network prefix
    Prefix,
    /// Order by peer IP address
    PeerIp,
    /// Order by peer AS number
    PeerAsn,
    /// Order by AS path (string comparison)
    AsPath,
    /// Order by next hop IP address
    NextHop,
}

impl OrderByField {
    /// Get a list of all field names for help text
    pub fn all_names() -> &'static [&'static str] {
        &[
            "timestamp",
            "prefix",
            "peer_ip",
            "peer_asn",
            "as_path",
            "next_hop",
        ]
    }
}

impl fmt::Display for OrderByField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timestamp => write!(f, "timestamp"),
            Self::Prefix => write!(f, "prefix"),
            Self::PeerIp => write!(f, "peer_ip"),
            Self::PeerAsn => write!(f, "peer_asn"),
            Self::AsPath => write!(f, "as_path"),
            Self::NextHop => write!(f, "next_hop"),
        }
    }
}

impl FromStr for OrderByField {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "timestamp" | "ts" | "time" => Ok(Self::Timestamp),
            "prefix" | "pfx" => Ok(Self::Prefix),
            "peer_ip" | "peerip" | "peer-ip" => Ok(Self::PeerIp),
            "peer_asn" | "peerasn" | "peer-asn" => Ok(Self::PeerAsn),
            "as_path" | "aspath" | "as-path" | "path" => Ok(Self::AsPath),
            "next_hop" | "nexthop" | "next-hop" | "nh" => Ok(Self::NextHop),
            _ => Err(format!(
                "Unknown order-by field '{}'. Valid fields: {}",
                s,
                Self::all_names().join(", ")
            )),
        }
    }
}

/// Direction for ordering output
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum OrderDirection {
    /// Ascending order (smallest/oldest first)
    #[default]
    Asc,
    /// Descending order (largest/newest first)
    Desc,
}

impl fmt::Display for OrderDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Asc => write!(f, "asc"),
            Self::Desc => write!(f, "desc"),
        }
    }
}

impl FromStr for OrderDirection {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "asc" | "ascending" | "a" => Ok(Self::Asc),
            "desc" | "descending" | "d" => Ok(Self::Desc),
            _ => Err(format!(
                "Unknown order direction '{}'. Valid values: asc, desc",
                s
            )),
        }
    }
}

// =============================================================================
// Timestamp Format for BGP Elements
// =============================================================================

/// Format for timestamp output in parse and search commands
///
/// This enum controls how timestamps are displayed in non-JSON output formats
/// (table, psv, markdown). JSON output always uses Unix timestamps as numbers
/// for backward compatibility.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum TimestampFormat {
    /// Unix timestamp (integer or float) - default for backward compatibility
    #[default]
    Unix,
    /// RFC3339/ISO 8601 format (e.g., "2023-10-11T15:00:00Z")
    Rfc3339,
}

impl TimestampFormat {
    /// Get a list of all format names for help text
    pub fn all_names() -> &'static [&'static str] {
        &["unix", "rfc3339"]
    }

    /// Format a Unix timestamp (f64) according to this format
    pub fn format_timestamp(&self, timestamp: f64) -> String {
        match self {
            Self::Unix => timestamp.to_string(),
            Self::Rfc3339 => {
                let secs = timestamp as i64;
                let nsecs = ((timestamp.fract().abs()) * 1_000_000_000.0) as u32;
                chrono::DateTime::from_timestamp(secs, nsecs)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| timestamp.to_string())
            }
        }
    }
}

impl fmt::Display for TimestampFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix => write!(f, "unix"),
            Self::Rfc3339 => write!(f, "rfc3339"),
        }
    }
}

impl FromStr for TimestampFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "unix" | "timestamp" | "ts" => Ok(Self::Unix),
            "rfc3339" | "iso8601" | "iso" => Ok(Self::Rfc3339),
            _ => Err(format!(
                "Unknown timestamp format '{}'. Valid formats: {}",
                s,
                Self::all_names().join(", ")
            )),
        }
    }
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "table" | "pretty" => Ok(Self::Table),
            "markdown" | "md" => Ok(Self::Markdown),
            "json" => Ok(Self::Json),
            "json-pretty" | "jsonpretty" => Ok(Self::JsonPretty),
            "json-line" | "jsonline" | "jsonl" | "ndjson" => Ok(Self::JsonLine),
            "psv" | "pipe" => Ok(Self::Psv),
            _ => Err(format!(
                "Unknown output format '{}'. Valid formats: {}",
                s,
                Self::all_names().join(", ")
            )),
        }
    }
}

/// Truncate a string to the specified length, adding "..." if truncated
///
/// This is useful for displaying long names (organization names, AS names, etc.)
/// in table output without breaking the table layout.
///
/// # Arguments
///
/// * `name` - The string to truncate
/// * `max_len` - Maximum length of the output string (including "..." if truncated)
///
/// # Examples
///
/// ```
/// use monocle::lens::utils::truncate_name;
///
/// // Short name - no truncation
/// assert_eq!(truncate_name("Short", 20), "Short");
///
/// // Long name - truncated with ...
/// assert_eq!(truncate_name("This is a very long name", 20), "This is a very lo...");
/// ```
pub fn truncate_name(name: &str, max_len: usize) -> String {
    if name.chars().count() <= max_len {
        name.to_string()
    } else {
        let truncated: String = name.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_name_short() {
        assert_eq!(truncate_name("Short", 20), "Short");
    }

    #[test]
    fn test_truncate_name_exact_limit() {
        assert_eq!(
            truncate_name("12345678901234567890", 20),
            "12345678901234567890"
        );
    }

    #[test]
    fn test_truncate_name_over_limit() {
        assert_eq!(
            truncate_name("This is a very long organization name", 20),
            "This is a very lo..."
        );
    }

    #[test]
    fn test_truncate_name_empty() {
        assert_eq!(truncate_name("", 20), "");
    }

    #[test]
    fn test_truncate_name_unicode() {
        // Unicode characters should be counted properly (by char, not bytes)
        // "日本語テスト名前これは長い" is 12 chars, truncated to 10 should be 7 chars + "..."
        assert_eq!(
            truncate_name("日本語テスト名前これは長い", 10),
            "日本語テスト名..."
        );
    }

    #[test]
    fn test_truncate_name_small_max() {
        // Edge case: very small max_len
        assert_eq!(truncate_name("Hello", 3), "...");
        assert_eq!(truncate_name("Hi", 3), "Hi");
    }

    #[test]
    fn test_output_format_from_str() {
        assert_eq!(
            OutputFormat::from_str("table").unwrap(),
            OutputFormat::Table
        );
        assert_eq!(
            OutputFormat::from_str("pretty").unwrap(),
            OutputFormat::Table
        );
        assert_eq!(
            OutputFormat::from_str("markdown").unwrap(),
            OutputFormat::Markdown
        );
        assert_eq!(
            OutputFormat::from_str("md").unwrap(),
            OutputFormat::Markdown
        );
        assert_eq!(OutputFormat::from_str("json").unwrap(), OutputFormat::Json);
        assert_eq!(
            OutputFormat::from_str("json-pretty").unwrap(),
            OutputFormat::JsonPretty
        );
        assert_eq!(
            OutputFormat::from_str("json-line").unwrap(),
            OutputFormat::JsonLine
        );
        assert_eq!(
            OutputFormat::from_str("jsonl").unwrap(),
            OutputFormat::JsonLine
        );
        assert_eq!(OutputFormat::from_str("psv").unwrap(), OutputFormat::Psv);
        assert!(OutputFormat::from_str("invalid").is_err());
    }

    #[test]
    fn test_output_format_display() {
        assert_eq!(OutputFormat::Table.to_string(), "table");
        assert_eq!(OutputFormat::Markdown.to_string(), "markdown");
        assert_eq!(OutputFormat::Json.to_string(), "json");
        assert_eq!(OutputFormat::JsonPretty.to_string(), "json-pretty");
        assert_eq!(OutputFormat::JsonLine.to_string(), "json-line");
        assert_eq!(OutputFormat::Psv.to_string(), "psv");
    }

    #[test]
    fn test_output_format_is_json() {
        assert!(!OutputFormat::Table.is_json());
        assert!(!OutputFormat::Markdown.is_json());
        assert!(OutputFormat::Json.is_json());
        assert!(OutputFormat::JsonPretty.is_json());
        assert!(OutputFormat::JsonLine.is_json());
        assert!(!OutputFormat::Psv.is_json());
    }

    #[test]
    fn test_output_format_is_table() {
        assert!(OutputFormat::Table.is_table());
        assert!(OutputFormat::Markdown.is_table());
        assert!(!OutputFormat::Json.is_table());
        assert!(!OutputFormat::JsonPretty.is_table());
        assert!(!OutputFormat::JsonLine.is_table());
        assert!(!OutputFormat::Psv.is_table());
    }

    #[test]
    fn test_string_or_vec_single() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "string_or_vec")]
            values: Vec<String>,
        }

        let json = r#"{"values": "hello"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.values, vec!["hello"]);
    }

    #[test]
    fn test_string_or_vec_array() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "string_or_vec")]
            values: Vec<String>,
        }

        let json = r#"{"values": ["hello", "world"]}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.values, vec!["hello", "world"]);
    }

    #[test]
    fn test_u32_or_vec_single_number() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "u32_or_vec")]
            asns: Vec<u32>,
        }

        let json = r#"{"asns": 13335}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asns, vec![13335]);
    }

    #[test]
    fn test_u32_or_vec_single_string() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "u32_or_vec")]
            asns: Vec<u32>,
        }

        let json = r#"{"asns": "13335"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asns, vec![13335]);
    }

    #[test]
    fn test_u32_or_vec_array() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "u32_or_vec")]
            asns: Vec<u32>,
        }

        let json = r#"{"asns": [13335, 15169]}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asns, vec![13335, 15169]);
    }

    #[test]
    fn test_option_u32_from_str_number() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(default, deserialize_with = "option_u32_from_str")]
            asn: Option<u32>,
        }

        let json = r#"{"asn": 13335}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asn, Some(13335));
    }

    #[test]
    fn test_option_u32_from_str_string() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(default, deserialize_with = "option_u32_from_str")]
            asn: Option<u32>,
        }

        let json = r#"{"asn": "13335"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asn, Some(13335));
    }

    #[test]
    fn test_option_u32_from_str_null() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(default, deserialize_with = "option_u32_from_str")]
            asn: Option<u32>,
        }

        let json = r#"{"asn": null}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asn, None);
    }

    #[test]
    fn test_u32_from_str_number() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "u32_from_str")]
            asn: u32,
        }

        let json = r#"{"asn": 13335}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asn, 13335);
    }

    #[test]
    fn test_u32_from_str_string() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(deserialize_with = "u32_from_str")]
            asn: u32,
        }

        let json = r#"{"asn": "13335"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert_eq!(result.asn, 13335);
    }

    #[test]
    fn test_bool_from_str_true() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(default, deserialize_with = "bool_from_str")]
            flag: bool,
        }

        let json = r#"{"flag": "true"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert!(result.flag);

        let json = r#"{"flag": true}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert!(result.flag);

        let json = r#"{"flag": "1"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert!(result.flag);
    }

    #[test]
    fn test_bool_from_str_false() {
        #[derive(Deserialize)]
        struct Test {
            #[serde(default, deserialize_with = "bool_from_str")]
            flag: bool,
        }

        let json = r#"{"flag": "false"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert!(!result.flag);

        let json = r#"{"flag": false}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert!(!result.flag);

        let json = r#"{"flag": "0"}"#;
        let result: Test = serde_json::from_str(json).unwrap();
        assert!(!result.flag);
    }

    #[test]
    fn test_order_by_field_from_str() {
        assert_eq!(
            OrderByField::from_str("timestamp").unwrap(),
            OrderByField::Timestamp
        );
        assert_eq!(
            OrderByField::from_str("ts").unwrap(),
            OrderByField::Timestamp
        );
        assert_eq!(
            OrderByField::from_str("prefix").unwrap(),
            OrderByField::Prefix
        );
        assert_eq!(
            OrderByField::from_str("peer_ip").unwrap(),
            OrderByField::PeerIp
        );
        assert_eq!(
            OrderByField::from_str("peer-ip").unwrap(),
            OrderByField::PeerIp
        );
        assert_eq!(
            OrderByField::from_str("peer_asn").unwrap(),
            OrderByField::PeerAsn
        );
        assert_eq!(
            OrderByField::from_str("as_path").unwrap(),
            OrderByField::AsPath
        );
        assert_eq!(
            OrderByField::from_str("path").unwrap(),
            OrderByField::AsPath
        );
        assert_eq!(
            OrderByField::from_str("next_hop").unwrap(),
            OrderByField::NextHop
        );
        assert_eq!(OrderByField::from_str("nh").unwrap(), OrderByField::NextHop);
        assert!(OrderByField::from_str("invalid").is_err());
    }

    #[test]
    fn test_order_by_field_display() {
        assert_eq!(OrderByField::Timestamp.to_string(), "timestamp");
        assert_eq!(OrderByField::Prefix.to_string(), "prefix");
        assert_eq!(OrderByField::PeerIp.to_string(), "peer_ip");
        assert_eq!(OrderByField::PeerAsn.to_string(), "peer_asn");
        assert_eq!(OrderByField::AsPath.to_string(), "as_path");
        assert_eq!(OrderByField::NextHop.to_string(), "next_hop");
    }

    #[test]
    fn test_order_direction_from_str() {
        assert_eq!(
            OrderDirection::from_str("asc").unwrap(),
            OrderDirection::Asc
        );
        assert_eq!(
            OrderDirection::from_str("ascending").unwrap(),
            OrderDirection::Asc
        );
        assert_eq!(
            OrderDirection::from_str("desc").unwrap(),
            OrderDirection::Desc
        );
        assert_eq!(
            OrderDirection::from_str("descending").unwrap(),
            OrderDirection::Desc
        );
        assert!(OrderDirection::from_str("invalid").is_err());
    }

    #[test]
    fn test_order_direction_display() {
        assert_eq!(OrderDirection::Asc.to_string(), "asc");
        assert_eq!(OrderDirection::Desc.to_string(), "desc");
    }

    #[test]
    fn test_timestamp_format_from_str() {
        assert_eq!(
            TimestampFormat::from_str("unix").unwrap(),
            TimestampFormat::Unix
        );
        assert_eq!(
            TimestampFormat::from_str("ts").unwrap(),
            TimestampFormat::Unix
        );
        assert_eq!(
            TimestampFormat::from_str("rfc3339").unwrap(),
            TimestampFormat::Rfc3339
        );
        assert_eq!(
            TimestampFormat::from_str("iso8601").unwrap(),
            TimestampFormat::Rfc3339
        );
        assert_eq!(
            TimestampFormat::from_str("iso").unwrap(),
            TimestampFormat::Rfc3339
        );
        assert!(TimestampFormat::from_str("invalid").is_err());
    }

    #[test]
    fn test_timestamp_format_display() {
        assert_eq!(TimestampFormat::Unix.to_string(), "unix");
        assert_eq!(TimestampFormat::Rfc3339.to_string(), "rfc3339");
    }

    #[test]
    fn test_timestamp_format_unix() {
        let format = TimestampFormat::Unix;
        assert_eq!(format.format_timestamp(1697043600.0), "1697043600");
        assert_eq!(format.format_timestamp(1697043600.5), "1697043600.5");
    }

    #[test]
    fn test_timestamp_format_rfc3339() {
        let format = TimestampFormat::Rfc3339;
        // 1697043600 = 2023-10-11T17:00:00Z (UTC)
        let result = format.format_timestamp(1697043600.0);
        assert!(result.starts_with("2023-10-11T17:00:00"));
        assert!(result.ends_with("Z") || result.contains("+00:00"));
    }

    #[test]
    fn test_timestamp_format_default() {
        assert_eq!(TimestampFormat::default(), TimestampFormat::Unix);
    }
}
