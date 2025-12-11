//! Parse lens module
//!
//! This module provides filter types for parsing MRT files with bgpkit-parser.
//! The filter types can optionally derive Clap's Args trait when the `cli` feature is enabled.

use crate::lens::time::TimeLens;
use anyhow::anyhow;
use anyhow::Result;
use bgpkit_parser::BgpkitParser;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::io::Read;
use std::net::IpAddr;

#[cfg(feature = "cli")]
use clap::{Args, ValueEnum};

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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(Args))]
pub struct ParseFilters {
    /// Filter by origin AS Number
    #[cfg_attr(feature = "cli", clap(short = 'o', long))]
    pub origin_asn: Option<u32>,

    /// Filter by network prefix
    #[cfg_attr(feature = "cli", clap(short = 'p', long))]
    pub prefix: Option<String>,

    /// Include super-prefix when filtering
    #[cfg_attr(feature = "cli", clap(short = 's', long))]
    #[serde(default)]
    pub include_super: bool,

    /// Include sub-prefix when filtering
    #[cfg_attr(feature = "cli", clap(short = 'S', long))]
    #[serde(default)]
    pub include_sub: bool,

    /// Filter by peer IP address
    #[cfg_attr(feature = "cli", clap(short = 'j', long))]
    #[serde(default)]
    pub peer_ip: Vec<IpAddr>,

    /// Filter by peer ASN
    #[cfg_attr(feature = "cli", clap(short = 'J', long))]
    pub peer_asn: Option<u32>,

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
        Ok(())
    }

    /// Convert filters to a BgpkitParser
    pub fn to_parser(&self, file_path: &str) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        let mut parser = BgpkitParser::new(file_path)?.disable_warnings();

        if let Some(v) = &self.as_path {
            parser = parser.add_filter("as_path", v.to_string().as_str())?;
        }
        if let Some(v) = &self.origin_asn {
            parser = parser.add_filter("origin_asn", v.to_string().as_str())?;
        }
        if let Some(v) = &self.prefix {
            let filter_type = match (self.include_super, self.include_sub) {
                (false, false) => "prefix",
                (true, false) => "prefix_super",
                (false, true) => "prefix_sub",
                (true, true) => "prefix_super_sub",
            };
            parser = parser.add_filter(filter_type, v.as_str())?;
        }
        if !self.peer_ip.is_empty() {
            let v = self.peer_ip.iter().map(|p| p.to_string()).join(",");
            parser = parser.add_filter("peer_ips", v.as_str())?;
        }
        if let Some(v) = &self.peer_asn {
            parser = parser.add_filter("peer_asn", v.to_string().as_str())?;
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
/// with various filters applied.
pub struct ParseLens;

impl ParseLens {
    /// Create a new parse lens
    pub fn new() -> Self {
        Self
    }

    /// Create a parser from filters and file path
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
}

impl Default for ParseLens {
    fn default() -> Self {
        Self::new()
    }
}
