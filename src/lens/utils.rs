//! Common utility functions for lens modules
//!
//! This module provides shared utility functions used across multiple lenses,
//! particularly for formatting output in tables.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

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
}
