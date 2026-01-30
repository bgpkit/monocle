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

use crate::lens::time::TimeLens;
use anyhow::anyhow;
use anyhow::Result;
use bgpkit_parser::BgpElem;
use bgpkit_parser::BgpkitParser;
use ipnet::IpNet;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::io::Read;
use std::net::IpAddr;
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

// =============================================================================
// Args
// =============================================================================

/// Filters for parsing MRT files
///
/// All filter fields support multiple comma-separated values with OR logic.
/// Values can be prefixed with `!` for negation (exclusion).
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

    /// Filter by elem type: announce (a) or withdraw (w)
    #[cfg_attr(feature = "cli", clap(short = 'm', long, value_enum))]
    pub elem_type: Option<ParseElemType>,

    /// Filter by start unix timestamp inclusive
    #[cfg_attr(feature = "cli", clap(short = 't', long))]
    pub start_ts: Option<String>,

    /// Filter by end unix timestamp inclusive
    #[cfg_attr(feature = "cli", clap(short = 'T', long))]
    pub end_ts: Option<String>,

    /// Duration from the start-ts or end-ts, e.g. 1h
    #[cfg_attr(feature = "cli", clap(short = 'd', long))]
    pub duration: Option<String>,

    /// Filter by AS path regex string
    #[cfg_attr(feature = "cli", clap(short = 'a', long))]
    pub as_path: Option<String>,
}

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

        // Check for mixed positive/negative in same filter
        Self::check_negation_consistency(&self.origin_asn, "origin-asn")?;
        Self::check_negation_consistency(&self.peer_asn, "peer-asn")?;
        Self::check_negation_consistency(&self.prefix, "prefix")?;

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

    /// Check that all values in a filter are either all positive or all negative
    fn check_negation_consistency(values: &[String], field_name: &str) -> Result<()> {
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

    /// Convert filters to a BgpkitParser
    ///
    /// This method creates a parser with all filters applied. Multi-value filters
    /// use OR logic (matches ANY of the specified values). Negated values (prefixed
    /// with `!`) exclude matching elements.
    pub fn to_parser(&self, file_path: &str) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        let mut parser = BgpkitParser::new(file_path)?.disable_warnings();

        if let Some(v) = &self.as_path {
            parser = parser.add_filter("as_path", v.to_string().as_str())?;
        }

        // Origin ASN filter - always use plural filter key for consistency
        if !self.origin_asn.is_empty() {
            let value = self.origin_asn.join(",");
            parser = parser.add_filter("origin_asns", &value)?;
        }

        // Prefix filter - always use plural filter keys
        if !self.prefix.is_empty() {
            let value = self.prefix.join(",");
            let filter_key = match (self.include_super, self.include_sub) {
                (false, false) => "prefixes",
                (true, false) => "prefixes_super",
                (false, true) => "prefixes_sub",
                (true, true) => "prefixes_super_sub",
            };
            parser = parser.add_filter(filter_key, &value)?;
        }

        // Peer IPs filter
        if !self.peer_ip.is_empty() {
            let v = self.peer_ip.iter().map(|p| p.to_string()).join(",");
            parser = parser.add_filter("peer_ips", v.as_str())?;
        }

        // Peer ASN filter - always use plural filter key for consistency
        if !self.peer_asn.is_empty() {
            let value = self.peer_asn.join(",");
            parser = parser.add_filter("peer_asns", &value)?;
        }

        if let Some(v) = &self.elem_type {
            parser = parser.add_filter("type", v.to_string().as_str())?;
        }

        match self.parse_start_end_strings() {
            Ok((start_ts, end_ts)) => {
                // in case we have full start_ts and end_ts, like in `monocle search` command input,
                // we will use the parsed start_ts and end_ts.
                parser = parser.add_filter("start_ts", start_ts.to_string().as_str())?;
                parser = parser.add_filter("end_ts", end_ts.to_string().as_str())?;
            }
            Err(_) => {
                // we could also likely not have any time filters, in this case, add filters
                // as we see them, and no modification is needed.
                let time_lens = TimeLens::new();
                if let Some(v) = &self.start_ts {
                    let ts = time_lens.parse_time_string(v.as_str())?.timestamp();
                    parser = parser.add_filter("start_ts", ts.to_string().as_str())?;
                }
                if let Some(v) = &self.end_ts {
                    let ts = time_lens.parse_time_string(v.as_str())?.timestamp();
                    parser = parser.add_filter("end_ts", ts.to_string().as_str())?;
                }
            }
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
    fn test_parse_filters_validate() {
        // Valid filters
        let filters = ParseFilters {
            origin_asn: vec!["13335".to_string(), "15169".to_string()],
            prefix: vec!["1.1.1.0/24".to_string()],
            peer_asn: vec!["!174".to_string()],
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

        // Mixed negation
        let filters = ParseFilters {
            origin_asn: vec!["13335".to_string(), "!15169".to_string()],
            ..Default::default()
        };
        assert!(filters.validate().is_err());
    }
}
