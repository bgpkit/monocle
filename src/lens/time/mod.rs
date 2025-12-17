//! Time parsing and formatting lens
//!
//! This module provides time parsing and formatting functionality for BGP-related
//! timestamps. It supports Unix timestamps, RFC3339 strings, and human-readable
//! date formats.
//!
//! # Feature Requirements
//!
//! This module requires the `lens-core` feature.
//!
//! # Example
//!
//! ```rust,ignore
//! use monocle::lens::time::{TimeLens, TimeParseArgs};
//!
//! let lens = TimeLens::new();
//! let args = TimeParseArgs::new(vec!["1697043600".to_string()]);
//! let results = lens.parse(&args)?;
//!
//! for t in &results {
//!     println!("{} -> {}", t.unix, t.rfc3339);
//! }
//! ```

use anyhow::anyhow;
use chrono::{DateTime, TimeZone, Utc};
use chrono_humanize::HumanTime;
use serde::{Deserialize, Deserializer, Serialize};

/// Deserialize a string or vec of strings into a Vec<String>
/// This allows query parameters to accept either `times=value` or `times=v1&times=v2`
fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

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

// =============================================================================
// Types
// =============================================================================

/// Represents a parsed BGP time with multiple format representations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "display", derive(tabled::Tabled))]
pub struct TimeBgpTime {
    /// Unix timestamp in seconds
    pub unix: i64,
    /// RFC3339 formatted string
    pub rfc3339: String,
    /// Human-readable relative time (e.g., "2 hours ago")
    pub human: String,
}

/// Output format for time lens results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum TimeOutputFormat {
    /// Table format with borders (default)
    #[default]
    Table,
    /// RFC3339 format only
    Rfc3339,
    /// Unix timestamp only
    Unix,
    /// JSON format
    Json,
}

// =============================================================================
// Args
// =============================================================================

/// Arguments for time parsing operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct TimeParseArgs {
    /// Time strings to parse (Unix timestamp, RFC3339, or human-readable)
    /// If empty, uses current time
    #[cfg_attr(feature = "cli", clap(value_name = "TIME"))]
    #[serde(default, deserialize_with = "string_or_vec")]
    pub times: Vec<String>,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: TimeOutputFormat,
}

impl TimeParseArgs {
    /// Create new args with given time strings
    pub fn new(times: Vec<String>) -> Self {
        Self {
            times,
            format: TimeOutputFormat::default(),
        }
    }

    /// Create args for current time
    pub fn now() -> Self {
        Self::default()
    }

    /// Set the output format
    pub fn with_format(mut self, format: TimeOutputFormat) -> Self {
        self.format = format;
        self
    }
}

// =============================================================================
// Lens
// =============================================================================

/// Time parsing and formatting lens
///
/// Provides methods for parsing various time string formats into standardized
/// representations, and formatting them for display.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::time::{TimeLens, TimeParseArgs};
///
/// let lens = TimeLens::new();
///
/// // Parse a Unix timestamp
/// let args = TimeParseArgs::new(vec!["1697043600".to_string()]);
/// let results = lens.parse(&args)?;
///
/// // Access results directly
/// for t in &results {
///     println!("Unix: {}, RFC3339: {}, Human: {}", t.unix, t.rfc3339, t.human);
/// }
///
/// // Or format for display (requires "display" feature for Table format)
/// let output = lens.format_results(&results, &TimeOutputFormat::Json);
/// println!("{}", output);
/// ```
pub struct TimeLens;

impl TimeLens {
    /// Create a new time lens
    pub fn new() -> Self {
        Self
    }

    /// Parse a single time string into a `DateTime<Utc>`
    ///
    /// Accepts:
    /// - Unix timestamps (e.g., "1697043600")
    /// - RFC3339 strings (e.g., "2023-10-11T00:00:00Z")
    /// - Human-readable dates (e.g., "October 11, 2023")
    pub fn parse_time_string(&self, time_string: &str) -> anyhow::Result<DateTime<Utc>> {
        let ts = match dateparser::parse_with(
            time_string,
            &Utc,
            chrono::NaiveTime::from_hms_opt(0, 0, 0)
                .ok_or_else(|| anyhow!("Failed to create time"))?,
        ) {
            Ok(ts) => ts,
            Err(_) => {
                return Err(anyhow!(
                    "Input time must be either Unix timestamp or time string compliant with RFC3339"
                ))
            }
        };

        Ok(ts)
    }

    /// Parse time arguments and return TimeBgpTime results
    pub fn parse(&self, args: &TimeParseArgs) -> anyhow::Result<Vec<TimeBgpTime>> {
        let now_ts = Utc::now().timestamp();

        let ts_vec = if args.times.is_empty() {
            vec![now_ts]
        } else {
            args.times
                .iter()
                .map(|ts| self.parse_time_string(ts.as_str()).map(|dt| dt.timestamp()))
                .collect::<anyhow::Result<Vec<_>>>()?
        };

        let bgptime_vec = ts_vec
            .into_iter()
            .map(|ts| {
                let ht =
                    HumanTime::from(chrono::Local::now() - chrono::Duration::seconds(now_ts - ts));
                let human = ht.to_string();
                let rfc3339 = Utc
                    .from_utc_datetime(
                        &DateTime::from_timestamp(ts, 0)
                            .unwrap_or_default()
                            .naive_utc(),
                    )
                    .to_rfc3339();
                TimeBgpTime {
                    unix: ts,
                    rfc3339,
                    human,
                }
            })
            .collect();

        Ok(bgptime_vec)
    }

    /// Parse time strings and return only RFC3339 formatted strings
    pub fn parse_to_rfc3339(&self, times: &[String]) -> anyhow::Result<Vec<String>> {
        if times.is_empty() {
            Ok(vec![Utc::now().to_rfc3339()])
        } else {
            times
                .iter()
                .map(|ts| {
                    self.parse_time_string(ts)
                        .map(|dt| dt.to_rfc3339())
                        .map_err(|_| anyhow!("unable to parse timestring: {}", ts))
                })
                .collect()
        }
    }

    /// Format results based on output format
    ///
    /// Note: Table format requires the `display` feature. Without it, Table format
    /// will fall back to JSON output.
    pub fn format_results(&self, results: &[TimeBgpTime], format: &TimeOutputFormat) -> String {
        match format {
            TimeOutputFormat::Table => {
                #[cfg(feature = "display")]
                {
                    use tabled::settings::Style;
                    use tabled::Table;
                    Table::new(results).with(Style::rounded()).to_string()
                }
                #[cfg(not(feature = "display"))]
                {
                    // Fall back to JSON when display feature is not enabled
                    serde_json::to_string_pretty(results).unwrap_or_default()
                }
            }
            TimeOutputFormat::Rfc3339 => results
                .iter()
                .map(|t| t.rfc3339.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            TimeOutputFormat::Unix => results
                .iter()
                .map(|t| t.unix.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            TimeOutputFormat::Json => serde_json::to_string_pretty(results).unwrap_or_default(),
        }
    }

    /// Format results as JSON
    ///
    /// This is a convenience method that always works regardless of features.
    pub fn format_json(&self, results: &[TimeBgpTime], pretty: bool) -> String {
        if pretty {
            serde_json::to_string_pretty(results).unwrap_or_default()
        } else {
            serde_json::to_string(results).unwrap_or_default()
        }
    }
}

impl Default for TimeLens {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_to_time() {
        use chrono::TimeZone;

        let lens = TimeLens::new();

        // Test with a valid Unix timestamp
        let unix_ts = "1697043600";
        let result = lens.parse_time_string(unix_ts);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Utc.timestamp_opt(1697043600, 0).unwrap());

        // Test with a valid RFC3339 string
        let rfc3339_str = "2023-10-11T00:00:00Z";
        let result = lens.parse_time_string(rfc3339_str);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Utc.timestamp_opt(1696982400, 0).unwrap());

        // Test with an incorrect date string
        let invalid_date = "not-a-date";
        let result = lens.parse_time_string(invalid_date);
        assert!(result.is_err());

        // Test with an empty string
        let empty_string = "";
        let result = lens.parse_time_string(empty_string);
        assert!(result.is_err());

        // Test with incomplete RFC3339 string
        let incomplete_rfc3339 = "2023-10-11T";
        let result = lens.parse_time_string(incomplete_rfc3339);
        assert!(result.is_err());

        // Test with a human-readable date string allowed by `dateparser`
        let human_readable = "October 11, 2023";
        let result = lens.parse_time_string(human_readable);
        assert!(result.is_ok());
        let expected_time = Utc.with_ymd_and_hms(2023, 10, 11, 0, 0, 0).unwrap();
        assert_eq!(result.unwrap(), expected_time);
    }

    #[test]
    fn test_parse_args() {
        let lens = TimeLens::new();

        // Test with empty args (current time)
        let args = TimeParseArgs::now();
        let results = lens.parse(&args).unwrap();
        assert_eq!(results.len(), 1);

        // Test with multiple times
        let args = TimeParseArgs::new(vec![
            "1697043600".to_string(),
            "2023-10-11T00:00:00Z".to_string(),
        ]);
        let results = lens.parse(&args).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_format_results() {
        let lens = TimeLens::new();
        let bgp_time = TimeBgpTime {
            unix: 1697043600,
            rfc3339: "2023-10-11T15:00:00+00:00".to_string(),
            human: "about 1 year ago".to_string(),
        };

        // Test RFC3339 format
        let output = lens.format_results(&[bgp_time.clone()], &TimeOutputFormat::Rfc3339);
        assert_eq!(output, "2023-10-11T15:00:00+00:00");

        // Test Unix format
        let output = lens.format_results(&[bgp_time.clone()], &TimeOutputFormat::Unix);
        assert_eq!(output, "1697043600");

        // Test JSON format
        let output = lens.format_results(&[bgp_time], &TimeOutputFormat::Json);
        assert!(output.contains("1697043600"));
    }

    #[test]
    fn test_format_json() {
        let lens = TimeLens::new();
        let bgp_time = TimeBgpTime {
            unix: 1697043600,
            rfc3339: "2023-10-11T15:00:00+00:00".to_string(),
            human: "about 1 year ago".to_string(),
        };

        let compact = lens.format_json(&[bgp_time.clone()], false);
        assert!(!compact.contains('\n') || compact.matches('\n').count() == 0);

        let pretty = lens.format_json(&[bgp_time], true);
        assert!(pretty.contains('\n'));
    }
}
