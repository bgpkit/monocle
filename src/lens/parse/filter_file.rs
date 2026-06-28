//! Filter file support for `parse` and `search` commands.
//!
//! This module provides two file-based filter input mechanisms that complement
//! the existing CLI filter flags (`-p`, `-o`, `-J`, etc.):
//!
//! - **JSON filter file** (`--filter-file`): a structured JSON file with any
//!   combination of filter fields (prefixes, origin ASNs, peer ASNs, etc.).
//! - **Prefix list file** (`--prefix-file`): a plain newline-delimited list of
//!   prefixes, one per line, for the common RIB-extract → filter-updates workflow.
//!
//! Both file-based filters are merged with any CLI filter flags. Within a single
//! filter dimension (e.g. prefixes), CLI and file values are **unioned** (OR
//! logic, matching how multi-value CLI filters already work). Across dimensions,
//! filters combine with **AND** logic, same as existing CLI behavior.
//!
//! # When to Use File-Based Filters
//!
//! CLI flags work well for small filter sets. File-based filters solve three
//! problems at scale:
//!
//! 1. **Arg-length limits** — filtering by 10,000+ prefixes exceeds `ARG_MAX`
//!    on many systems. A file sidesteps this entirely.
//! 2. **Reusable filter sets** — the same prefix list can be applied to many
//!    files without re-typing or shell pipeline tricks.
//! 3. **Multi-dimensional filters** — a JSON file can express prefix + ASN +
//!    community + elem-type filters in one place, instead of long flag chains.
//!
//! # JSON Filter File Format
//!
//! The JSON file mirrors the CLI filter flags. All fields are optional; include
//! only the dimensions you need.
//!
//! ```json
//! {
//!   "prefixes": ["192.0.2.0/24", "2001:db8::/32"],
//!   "origin_asns": ["64496"],
//!   "peer_asns": ["174", "6939"],
//!   "peer_ips": ["192.0.2.1"],
//!   "communities": ["64496:100", "*:200"],
//!   "as_path_regex": "174 64496$",
//!   "elem_type": "w",
//!   "include_super": false,
//!   "include_sub": true,
//!   "start_ts": "2025-01-01T00:00:00Z",
//!   "end_ts": "2025-01-01T01:00:00Z"
//! }
//! ```
//!
//! String-typed fields (`prefixes`, `origin_asns`, `peer_asns`, `communities`)
//! use the same syntax as CLI flags, including `!` prefix for negation. See the
//! [`ParseFilters`] docs for details on negation semantics.
//!
//! ## Example: Withdrawal investigation for a specific AS
//!
//! ```json
//! {
//!   "origin_asns": ["64496"],
//!   "elem_type": "w",
//!   "start_ts": "2025-03-01T00:00:00Z",
//!   "duration": "2h"
//! }
//! ```
//!
//! This matches all BGP withdrawals originated by AS64496 within a 2-hour window.
//!
//! ## Example: Country-level investigation with community filter
//!
//! ```json
//! {
//!   "prefixes": ["203.0.113.0/24", "198.51.100.0/24"],
//!   "communities": ["64496:100", "64496:200"],
//!   "include_sub": true
//! }
//! ```
//!
//! This matches announcements for the listed prefixes (and any more-specific
//! sub-prefixes) carrying either of the specified communities.
//!
//! ## Combining `--filter-file` with CLI flags
//!
//! CLI flags and file filters are merged. Within each dimension, values are
//! unioned (OR logic). For example, `-p 10.0.0.0/8 --filter-file filters.json`
//! where `filters.json` contains `"prefixes": ["192.0.2.0/24"]` results in a
//! prefix filter matching **both** `10.0.0.0/8` and `192.0.2.0/24`.
//!
//! For scalar fields (`as_path_regex`, `elem_type`, `start_ts`, `end_ts`,
//! `duration`), CLI flags take precedence over file values — if you pass
//! `--as-path` on the command line, the file's `as_path_regex` is ignored.
//! Boolean fields (`include_super`, `include_sub`) are OR-ed: the file can
//! enable them but cannot disable a CLI-enabled flag.
//!
//! ```rust,no_run
//! use monocle::lens::parse::filter_file::FilterFile;
//! use monocle::lens::parse::ParseFilters;
//! use std::path::Path;
//!
//! // Load a JSON filter file
//! let file = FilterFile::load(Path::new("filters.json"))?;
//!
//! // Merge into filters that may already have CLI values
//! let mut filters = ParseFilters {
//!     prefix: vec!["10.0.0.0/8".to_string()],
//!     ..Default::default()
//! };
//! file.merge_into(&mut filters).unwrap();
//!
//! // Now filters.prefix == ["10.0.0.0/8", <prefixes from file>...]
//! assert!(filters.prefix.len() >= 1);
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # Prefix List File Format
//!
//! For the most common case — filtering by a large list of prefixes — a plain
//! newline-delimited file is the most ergonomic option. This fits the standard
//! RIB-extract → filter-updates workflow:
//!
//! ```bash
//! # Step 1: Extract prefix list from a RIB dump
//! monocle parse -o 64496 rib.gz | cut -d'|' -f5 | sort -u > prefixes.txt
//!
//! # Step 2: Filter subsequent updates using the prefix file
//! monocle parse --prefix-file prefixes.txt updates.gz
//! ```
//!
//! The file format is one prefix per line in CIDR notation. Blank lines and
//! lines starting with `#` are ignored, so you can add comments:
//!
//! ```text
//! # Prefixes originated by AS64496 (extracted from RIB)
//! 192.0.2.0/24
//! 198.51.100.0/24
//!
//! # IPv6 prefixes
//! 2001:db8::/32
//! ```
//!
//! ```rust,no_run
//! use monocle::lens::parse::filter_file::{load_prefix_file, merge_prefix_file};
//! use monocle::lens::parse::ParseFilters;
//! use std::path::Path;
//!
//! let prefixes = load_prefix_file(Path::new("prefixes.txt"))?;
//!
//! let mut filters = ParseFilters::default();
//! merge_prefix_file(prefixes, &mut filters);
//!
//! assert!(!filters.prefix.is_empty());
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! # Combining Both File Types
//!
//! You can use `--filter-file` and `--prefix-file` together. The prefixes from
//! both sources are unioned into a single prefix filter dimension:
//!
//! ```bash
//! monocle parse \
//!   --filter-file as_filters.json \
//!   --prefix-file extra_prefixes.txt \
//!   -c rrc00 \
//!   updates.gz
//! ```
//!
//! Here `as_filters.json` might specify origin ASNs and communities, while
//! `extra_prefixes.txt` adds more prefixes on top of any `prefixes` in the JSON
//! file. The `-c rrc00` CLI flag adds a collector filter (search command only).

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::Path;

use super::{ParseElemType, ParseFilters};

/// Structured JSON filter file.
///
/// All fields are optional. Fields present in the file are merged into
/// [`ParseFilters`] alongside any CLI-provided filter values.
///
/// # Examples
///
/// ## Full filter file
///
/// ```json
/// {
///   "prefixes": ["192.0.2.0/24"],
///   "origin_asns": ["64496"],
///   "peer_asns": ["174"],
///   "communities": ["64496:100"],
///   "as_path_regex": "174 64496$",
///   "elem_type": "w",
///   "include_sub": true,
///   "start_ts": "2025-01-01T00:00:00Z",
///   "end_ts": "2025-01-01T01:00:00Z"
/// }
/// ```
///
/// ## Prefix-only filter file
///
/// ```json
/// { "prefixes": ["10.0.0.0/8", "192.0.2.0/24", "2001:db8::/32"] }
/// ```
///
/// ## Negation (exclusion)
///
/// Exclude all elements from AS64496 and AS64497:
///
/// ```json
/// { "origin_asns": ["!64496", "!64497"] }
/// ```
///
/// All values within a dimension must be either all positive or all negated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterFile {
    /// Prefix filters (CIDR notation, `!` prefix for negation).
    ///
    /// Unioned with any `-p`/`--prefix` CLI values.
    #[serde(default)]
    pub prefixes: Vec<String>,

    /// Origin ASN filters (numeric string, `!` prefix for negation).
    ///
    /// Unioned with any `-o`/`--origin-asn` CLI values.
    #[serde(default)]
    pub origin_asns: Vec<String>,

    /// Peer ASN filters (numeric string, `!` prefix for negation).
    ///
    /// Unioned with any `-J`/`--peer-asn` CLI values.
    #[serde(default)]
    pub peer_asns: Vec<String>,

    /// Peer IP filters (IP address strings, e.g. `"192.0.2.1"`).
    ///
    /// Unioned with any `-j`/`--peer-ip` CLI values.
    #[serde(default)]
    pub peer_ips: Vec<String>,

    /// Community filters (`A:B` or `A:B:C`, `!` prefix for negation).
    ///
    /// Unioned with any `-C`/`--community` CLI values.
    #[serde(default)]
    pub communities: Vec<String>,

    /// AS path regex string.
    ///
    /// Only applied if `--as-path` is not set on the CLI (CLI takes precedence).
    #[serde(default)]
    pub as_path_regex: Option<String>,

    /// Element type filter: `"a"` (announce) or `"w"` (withdraw).
    ///
    /// Only applied if `--elem-type` is not set on the CLI (CLI takes precedence).
    #[serde(default)]
    pub elem_type: Option<String>,

    /// Include super-prefixes (less specific) when filtering.
    ///
    /// OR-ed with the `-s`/`--include-super` CLI flag — file can enable but not
    /// disable.
    #[serde(default)]
    pub include_super: bool,

    /// Include sub-prefixes (more specific) when filtering.
    ///
    /// OR-ed with the `-S`/`--include-sub` CLI flag — file can enable but not
    /// disable.
    #[serde(default)]
    pub include_sub: bool,

    /// Start timestamp string (same formats accepted by `--start-ts`).
    ///
    /// Only applied if `--start-ts` is not set on the CLI (CLI takes precedence).
    #[serde(default)]
    pub start_ts: Option<String>,

    /// End timestamp string (same formats accepted by `--end-ts`).
    ///
    /// Only applied if `--end-ts` is not set on the CLI (CLI takes precedence).
    #[serde(default)]
    pub end_ts: Option<String>,

    /// Duration string (e.g. `"1h"`, `"30m"`).
    ///
    /// Only applied if `--duration` is not set on the CLI (CLI takes precedence).
    #[serde(default)]
    pub duration: Option<String>,
}

impl FilterFile {
    /// Load and parse a JSON filter file from `path`.
    ///
    /// Supports local paths and remote URLs (via `oneio`).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or contains invalid JSON.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use monocle::lens::parse::filter_file::FilterFile;
    /// use std::path::Path;
    ///
    /// let file = FilterFile::load(Path::new("filters.json"))?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn load(path: &Path) -> Result<Self> {
        let content = oneio::read_to_string(
            path.to_str()
                .ok_or_else(|| anyhow!("filter file path is not valid UTF-8"))?,
        )
        .map_err(|e| anyhow!("Failed to read filter file '{}': {}", path.display(), e))?;

        let file: FilterFile = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse filter file '{}': {}", path.display(), e))?;

        Ok(file)
    }

    /// Merge this filter file's values into `filters`.
    ///
    /// # Merge Semantics
    ///
    /// | Field type | Merge rule |
    /// |------------|------------|
    /// | Vec fields (prefixes, ASNs, communities, peer_ips) | **Union** — file values appended to CLI values |
    /// | Scalar fields (as_path, elem_type, start_ts, end_ts, duration) | **CLI precedence** — file value used only if CLI didn't set it |
    /// | Boolean fields (include_super, include_sub) | **OR** — file can enable, cannot disable |
    ///
    /// # Example
    ///
    /// ```
    /// use monocle::lens::parse::filter_file::FilterFile;
    /// use monocle::lens::parse::ParseFilters;
    ///
    /// let file = FilterFile {
    ///     prefixes: vec!["192.0.2.0/24".to_string()],
    ///     origin_asns: vec!["64496".to_string()],
    ///     include_sub: true,
    ///     ..Default::default()
    /// };
    ///
    /// let mut filters = ParseFilters {
    ///     prefix: vec!["10.0.0.0/8".to_string()], // CLI value
    ///     ..Default::default()
    /// };
    /// file.merge_into(&mut filters).unwrap();
    ///
    /// // Prefixes unioned: both CLI and file values
    /// assert_eq!(filters.prefix, vec!["10.0.0.0/8", "192.0.2.0/24"]);
    /// // Origin ASN from file (CLI didn't set it)
    /// assert_eq!(filters.origin_asn, vec!["64496"]);
    /// // Boolean OR'd
    /// assert!(filters.include_sub);
    /// ```
    pub fn merge_into(self, filters: &mut ParseFilters) -> Result<()> {
        // Vec fields: union CLI + file values, trimming whitespace and dropping empties
        filters.prefix.extend(
            self.prefixes
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
        filters.origin_asn.extend(
            self.origin_asns
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
        filters.peer_asn.extend(
            self.peer_asns
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );
        filters.communities.extend(
            self.communities
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        );

        // peer_ip is Vec<IpAddr> — parse string values, error on invalid
        for ip_str in self.peer_ips {
            let trimmed = ip_str.trim();
            let ip: IpAddr = trimmed.parse().map_err(|_| {
                anyhow!(
                    "Invalid peer IP '{}' in filter file: must be a valid IP address",
                    ip_str
                )
            })?;
            filters.peer_ip.push(ip);
        }

        // Scalar/Option fields: only set from file if CLI didn't set them
        if filters.as_path.is_none() {
            filters.as_path = self.as_path_regex;
        }
        if filters.elem_type.is_none() {
            // Map string "a"/"w" to ParseElemType, error on unrecognized
            if let Some(et) = self.elem_type.as_deref() {
                let et = et.trim();
                filters.elem_type = match et.to_lowercase().as_str() {
                    "a" | "announce" | "announcement" => Some(ParseElemType::A),
                    "w" | "withdraw" | "withdrawal" => Some(ParseElemType::W),
                    _ => {
                        return Err(anyhow!(
                            "Invalid elem_type '{}' in filter file: must be 'a' (announce) or 'w' (withdraw)",
                            et
                        ));
                    }
                };
            }
        }
        // Boolean flags: OR with file values (file can enable, CLI can't disable)
        if self.include_super {
            filters.include_super = true;
        }
        if self.include_sub {
            filters.include_sub = true;
        }
        // Time fields: CLI takes precedence
        if filters.start_ts.is_none() {
            filters.start_ts = self.start_ts;
        }
        if filters.end_ts.is_none() {
            filters.end_ts = self.end_ts;
        }
        if filters.duration.is_none() {
            filters.duration = self.duration;
        }

        Ok(())
    }
}

/// Load a newline-delimited prefix list file.
///
/// Each line should contain a single prefix in CIDR notation. Blank lines and
/// lines starting with `#` are ignored. Whitespace around each prefix is trimmed.
///
/// # Errors
///
/// Returns an error if the file cannot be read or contains no prefixes (after
/// skipping blank lines and comments).
///
/// # Example
///
/// ```rust,no_run
/// use monocle::lens::parse::filter_file::load_prefix_file;
/// use std::path::Path;
///
/// let prefixes = load_prefix_file(Path::new("prefixes.txt"))?;
/// println!("Loaded {} prefixes", prefixes.len());
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn load_prefix_file(path: &Path) -> Result<Vec<String>> {
    let content = oneio::read_to_string(
        path.to_str()
            .ok_or_else(|| anyhow!("prefix file path is not valid UTF-8"))?,
    )
    .map_err(|e| anyhow!("Failed to read prefix file '{}': {}", path.display(), e))?;

    let prefixes: Vec<String> = content
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(String::from)
        .collect();

    if prefixes.is_empty() {
        return Err(anyhow!(
            "Prefix file '{}' contains no prefixes (after skipping blank lines and comments)",
            path.display()
        ));
    }

    Ok(prefixes)
}

/// Merge a prefix list (from `--prefix-file`) into `filters.prefix`.
///
/// The prefixes are appended to any existing CLI prefix values (union / OR logic).
///
/// # Example
///
/// ```
/// use monocle::lens::parse::filter_file::merge_prefix_file;
/// use monocle::lens::parse::ParseFilters;
///
/// let mut filters = ParseFilters {
///     prefix: vec!["10.0.0.0/8".to_string()],
///     ..Default::default()
/// };
/// merge_prefix_file(
///     vec!["192.0.2.0/24".to_string(), "2001:db8::/32".to_string()],
///     &mut filters,
/// );
///
/// assert_eq!(filters.prefix, vec!["10.0.0.0/8", "192.0.2.0/24", "2001:db8::/32"]);
/// ```
pub fn merge_prefix_file(prefixes: Vec<String>, filters: &mut ParseFilters) {
    filters.prefix.extend(prefixes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // =========================================================================
    // Deserialization tests
    // =========================================================================

    #[test]
    fn test_filter_file_deserialize_full() {
        let json = r#"{
            "prefixes": ["192.0.2.0/24", "2001:db8::/32"],
            "origin_asns": ["64496", "!64497"],
            "peer_asns": ["174"],
            "peer_ips": ["192.0.2.1"],
            "communities": ["64496:100"],
            "as_path_regex": "174 64496$",
            "elem_type": "w",
            "include_super": true,
            "include_sub": false,
            "start_ts": "2025-01-01T00:00:00Z",
            "end_ts": "2025-01-01T01:00:00Z"
        }"#;
        let file: FilterFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.prefixes.len(), 2);
        assert_eq!(file.origin_asns.len(), 2);
        assert_eq!(file.peer_asns, vec!["174"]);
        assert!(file.include_super);
        assert!(!file.include_sub);
        assert_eq!(file.elem_type.as_deref(), Some("w"));
    }

    #[test]
    fn test_filter_file_deserialize_empty() {
        let json = "{}";
        let file: FilterFile = serde_json::from_str(json).unwrap();
        assert!(file.prefixes.is_empty());
        assert!(file.origin_asns.is_empty());
        assert!(file.as_path_regex.is_none());
    }

    #[test]
    fn test_filter_file_deserialize_partial() {
        let json = r#"{"prefixes": ["10.0.0.0/8"]}"#;
        let file: FilterFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.prefixes, vec!["10.0.0.0/8"]);
        assert!(file.origin_asns.is_empty());
    }

    #[test]
    fn test_filter_file_deserialize_with_negation() {
        let json = r#"{"origin_asns": ["!64496", "!64497"]}"#;
        let file: FilterFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.origin_asns, vec!["!64496", "!64497"]);
    }

    #[test]
    fn test_filter_file_deserialize_invalid_json() {
        let json = r#"{"prefixes": broken}"#;
        assert!(serde_json::from_str::<FilterFile>(json).is_err());
    }

    #[test]
    fn test_filter_file_deny_unknown_fields() {
        // Typo: "origin_asn" instead of "origin_asns" should fail
        let json = r#"{"origin_asn": ["64496"]}"#;
        let result = serde_json::from_str::<FilterFile>(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown field"));
    }

    #[test]
    fn test_filter_file_serialize_roundtrip() {
        let file = FilterFile {
            prefixes: vec!["10.0.0.0/8".to_string()],
            origin_asns: vec!["64496".to_string()],
            include_sub: true,
            start_ts: Some("2025-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: FilterFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prefixes, file.prefixes);
        assert_eq!(back.origin_asns, file.origin_asns);
        assert!(back.include_sub);
        assert_eq!(back.start_ts, file.start_ts);
    }

    // =========================================================================
    // Merge logic tests
    // =========================================================================

    #[test]
    fn test_merge_into_empty_filters() {
        let file = FilterFile {
            prefixes: vec!["192.0.2.0/24".to_string()],
            origin_asns: vec!["64496".to_string()],
            include_sub: true,
            ..Default::default()
        };
        let mut filters = ParseFilters::default();
        file.merge_into(&mut filters).unwrap();

        assert_eq!(filters.prefix, vec!["192.0.2.0/24"]);
        assert_eq!(filters.origin_asn, vec!["64496"]);
        assert!(filters.include_sub);
        assert!(!filters.include_super);
    }

    #[test]
    fn test_merge_into_union_with_cli() {
        let mut filters = ParseFilters {
            prefix: vec!["10.0.0.0/8".to_string()],
            origin_asn: vec!["13335".to_string()],
            ..Default::default()
        };
        let file = FilterFile {
            prefixes: vec!["192.0.2.0/24".to_string()],
            origin_asns: vec!["64496".to_string()],
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();

        assert_eq!(filters.prefix, vec!["10.0.0.0/8", "192.0.2.0/24"]);
        assert_eq!(filters.origin_asn, vec!["13335", "64496"]);
    }

    #[test]
    fn test_merge_into_cli_scalar_takes_precedence() {
        let mut filters = ParseFilters {
            as_path: Some("CLI_PATH".to_string()),
            start_ts: Some("2025-06-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        let file = FilterFile {
            as_path_regex: Some("FILE_PATH".to_string()),
            start_ts: Some("2025-01-01T00:00:00Z".to_string()),
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();

        assert_eq!(filters.as_path.as_deref(), Some("CLI_PATH"));
        assert_eq!(filters.start_ts.as_deref(), Some("2025-06-01T00:00:00Z"));
    }

    #[test]
    fn test_merge_into_file_scalar_when_cli_empty() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            as_path_regex: Some("FILE_PATH".to_string()),
            elem_type: Some("w".to_string()),
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();

        assert_eq!(filters.as_path.as_deref(), Some("FILE_PATH"));
        assert!(matches!(filters.elem_type, Some(ParseElemType::W)));
    }

    #[test]
    fn test_merge_into_elem_type_variants() {
        for (input, is_a) in [
            ("a", true),
            ("announce", true),
            ("ANNOUNCEMENT", true),
            ("w", false),
            ("withdraw", false),
            ("WITHDRAWAL", false),
        ] {
            let mut filters = ParseFilters::default();
            let file = FilterFile {
                elem_type: Some(input.to_string()),
                ..Default::default()
            };
            file.merge_into(&mut filters).unwrap();
            match (&filters.elem_type, is_a) {
                (Some(ParseElemType::A), true) => {}
                (Some(ParseElemType::W), false) => {}
                _ => panic!("elem_type '{}' mapped incorrectly", input),
            }
        }
    }

    #[test]
    fn test_merge_into_elem_type_invalid_errors() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            elem_type: Some("invalid".to_string()),
            ..Default::default()
        };
        let result = file.merge_into(&mut filters);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid elem_type"));
    }

    #[test]
    fn test_merge_into_elem_type_cli_takes_precedence() {
        let mut filters = ParseFilters {
            elem_type: Some(ParseElemType::A),
            ..Default::default()
        };
        let file = FilterFile {
            elem_type: Some("w".to_string()),
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();
        // CLI value (A) preserved, file value (w) ignored
        assert!(matches!(filters.elem_type, Some(ParseElemType::A)));
    }

    #[test]
    fn test_merge_into_boolean_flags_or() {
        let mut filters = ParseFilters {
            include_super: true,
            ..Default::default()
        };
        let file = FilterFile {
            include_sub: true,
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();

        assert!(filters.include_super);
        assert!(filters.include_sub);
    }

    #[test]
    fn test_merge_into_boolean_file_cannot_disable_cli() {
        let mut filters = ParseFilters {
            include_sub: true,
            ..Default::default()
        };
        let file = FilterFile {
            include_sub: false, // file says false, but CLI already true
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();
        assert!(filters.include_sub);
    }

    #[test]
    fn test_merge_into_peer_ips_parsed() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            peer_ips: vec!["192.0.2.1".to_string(), "10.0.0.1".to_string()],
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();

        assert_eq!(filters.peer_ip.len(), 2);
    }

    #[test]
    fn test_merge_into_peer_ips_invalid_errors() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            peer_ips: vec!["not_an_ip".to_string(), "192.0.2.1".to_string()],
            ..Default::default()
        };
        let result = file.merge_into(&mut filters);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid peer IP"));
    }

    #[test]
    fn test_merge_into_time_fields_cli_precedence() {
        let mut filters = ParseFilters {
            duration: Some("1h".to_string()),
            ..Default::default()
        };
        let file = FilterFile {
            duration: Some("2h".to_string()),
            end_ts: Some("2025-01-01T01:00:00Z".to_string()),
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();

        // CLI duration preserved, file end_ts applied (CLI didn't set it)
        assert_eq!(filters.duration.as_deref(), Some("1h"));
        assert_eq!(filters.end_ts.as_deref(), Some("2025-01-01T01:00:00Z"));
    }

    #[test]
    fn test_merge_into_communities_union() {
        let mut filters = ParseFilters {
            communities: vec!["13335:100".to_string()],
            ..Default::default()
        };
        let file = FilterFile {
            communities: vec!["64496:200".to_string()],
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();
        assert_eq!(filters.communities, vec!["13335:100", "64496:200"]);
    }

    #[test]
    fn test_merge_into_vec_fields_trimmed() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            prefixes: vec!["  192.0.2.0/24  ".to_string()],
            origin_asns: vec!["  64496  ".to_string()],
            peer_asns: vec!["  174 ".to_string()],
            communities: vec!["  64496:100  ".to_string()],
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();
        assert_eq!(filters.prefix, vec!["192.0.2.0/24"]);
        assert_eq!(filters.origin_asn, vec!["64496"]);
        assert_eq!(filters.peer_asn, vec!["174"]);
        assert_eq!(filters.communities, vec!["64496:100"]);
    }

    #[test]
    fn test_merge_into_vec_fields_empty_strings_dropped() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            prefixes: vec!["".to_string(), "  ".to_string(), "192.0.2.0/24".to_string()],
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();
        assert_eq!(filters.prefix, vec!["192.0.2.0/24"]);
    }

    #[test]
    fn test_merge_into_elem_type_trimmed() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            elem_type: Some("  W  ".to_string()),
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();
        assert!(matches!(filters.elem_type, Some(ParseElemType::W)));
    }

    #[test]
    fn test_merge_into_peer_ip_trimmed() {
        let mut filters = ParseFilters::default();
        let file = FilterFile {
            peer_ips: vec!["  192.0.2.1  ".to_string()],
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();
        assert_eq!(filters.peer_ip.len(), 1);
    }

    #[test]
    fn test_merge_prefix_file() {
        let mut filters = ParseFilters {
            prefix: vec!["10.0.0.0/8".to_string()],
            ..Default::default()
        };
        merge_prefix_file(
            vec!["192.0.2.0/24".to_string(), "2001:db8::/32".to_string()],
            &mut filters,
        );
        assert_eq!(
            filters.prefix,
            vec!["10.0.0.0/8", "192.0.2.0/24", "2001:db8::/32"]
        );
    }

    // =========================================================================
    // Prefix file content parsing tests
    // =========================================================================

    #[test]
    fn test_prefix_file_content_parsing() {
        let content =
            "# comment\n\n10.0.0.0/8\n  192.0.2.0/24  \n# another comment\n2001:db8::/32\n";
        let prefixes: Vec<String> = content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        assert_eq!(
            prefixes,
            vec!["10.0.0.0/8", "192.0.2.0/24", "2001:db8::/32"]
        );
    }

    #[test]
    fn test_prefix_file_content_empty_after_comments() {
        let content = "# only comments\n\n# nothing else\n";
        let prefixes: Vec<String> = content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(String::from)
            .collect();
        assert!(prefixes.is_empty());
    }

    // =========================================================================
    // Integration tests: real file I/O via FilterFile::load / load_prefix_file
    // =========================================================================

    /// Helper: write `content` to a temp file and return its path.
    fn write_temp_file(name: &str, content: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("monocle_test_{}_{}", std::process::id(), name));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_filter_file_load_from_disk() {
        let json = r#"{
            "prefixes": ["192.0.2.0/24", "198.51.100.0/24"],
            "origin_asns": ["64496"],
            "include_sub": true
        }"#;
        let path = write_temp_file("filter_good.json", json);

        let file = FilterFile::load(&path).unwrap();
        assert_eq!(file.prefixes.len(), 2);
        assert_eq!(file.origin_asns, vec!["64496"]);
        assert!(file.include_sub);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_filter_file_load_empty_json() {
        let path = write_temp_file("filter_empty.json", "{}");
        let file = FilterFile::load(&path).unwrap();
        assert!(file.prefixes.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_filter_file_load_invalid_json() {
        let path = write_temp_file("filter_bad.json", "{not valid json}");
        let result = FilterFile::load(&path);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Failed to parse filter file"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_filter_file_load_nonexistent() {
        let path = std::env::temp_dir().join("monocle_nonexistent_filter_file.json");
        let _ = std::fs::remove_file(&path); // ensure it doesn't exist
        let result = FilterFile::load(&path);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to read filter file"));
    }

    #[test]
    fn test_load_prefix_file_from_disk() {
        let content = "# Prefixes for AS64496\n192.0.2.0/24\n\n198.51.100.0/24\n2001:db8::/32\n";
        let path = write_temp_file("prefixes_good.txt", content);

        let prefixes = load_prefix_file(&path).unwrap();
        assert_eq!(
            prefixes,
            vec!["192.0.2.0/24", "198.51.100.0/24", "2001:db8::/32"]
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prefix_file_only_comments() {
        let content = "# just a comment\n# another one\n";
        let path = write_temp_file("prefixes_empty.txt", content);

        let result = load_prefix_file(&path);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("contains no prefixes"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prefix_file_empty_file() {
        let path = write_temp_file("prefixes_blank.txt", "");
        let result = load_prefix_file(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_prefix_file_nonexistent() {
        let path = std::env::temp_dir().join("monocle_nonexistent_prefix_file.txt");
        let _ = std::fs::remove_file(&path);
        let result = load_prefix_file(&path);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to read prefix file"));
    }

    // =========================================================================
    // End-to-end: load file → merge → validate
    // =========================================================================

    #[test]
    fn test_filter_file_load_merge_validate() {
        let json = r#"{
            "prefixes": ["192.0.2.0/24", "2001:db8::/32"],
            "origin_asns": ["64496", "64497"],
            "peer_asns": ["174"],
            "communities": ["64496:100"],
            "start_ts": "2025-01-01T00:00:00Z",
            "end_ts": "2025-01-01T01:00:00Z"
        }"#;
        let path = write_temp_file("filter_e2e.json", json);

        let file = FilterFile::load(&path).unwrap();
        let mut filters = ParseFilters::default();
        file.merge_into(&mut filters).unwrap();

        // The merged filters should pass validation
        assert!(filters.validate().is_ok());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_prefix_file_load_merge_validate() {
        let content = "10.0.0.0/8\n192.0.2.0/24\n2001:db8::/32\n";
        let path = write_temp_file("prefix_e2e.txt", content);

        let prefixes = load_prefix_file(&path).unwrap();
        let mut filters = ParseFilters::default();
        merge_prefix_file(prefixes, &mut filters);

        assert!(filters.validate().is_ok());
        assert_eq!(filters.prefix.len(), 3);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_filter_file_with_invalid_prefix_fails_validation() {
        let json = r#"{ "prefixes": ["not-a-prefix"] }"#;
        let path = write_temp_file("filter_bad_prefix.json", json);

        let file = FilterFile::load(&path).unwrap();
        let mut filters = ParseFilters::default();
        file.merge_into(&mut filters).unwrap();

        // Validation should catch the invalid prefix
        assert!(filters.validate().is_err());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_filter_file_negation_validation() {
        // All-negated should pass
        let json = r#"{ "origin_asns": ["!64496", "!64497"] }"#;
        let path = write_temp_file("filter_neg.json", json);
        let file = FilterFile::load(&path).unwrap();
        let mut filters = ParseFilters::default();
        file.merge_into(&mut filters).unwrap();
        assert!(filters.validate().is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_combined_filter_file_and_prefix_file() {
        // Simulate: --filter-file as_filters.json --prefix-file extra_prefixes.txt
        let json = r#"{ "origin_asns": ["64496"], "communities": ["64496:100"] }"#;
        let json_path = write_temp_file("combined_filter.json", json);

        let prefix_content = "192.0.2.0/24\n198.51.100.0/24\n";
        let prefix_path = write_temp_file("combined_prefixes.txt", prefix_content);

        let filter_file = FilterFile::load(&json_path).unwrap();
        let extra_prefixes = load_prefix_file(&prefix_path).unwrap();

        let mut filters = ParseFilters::default();
        filter_file.merge_into(&mut filters).unwrap();
        merge_prefix_file(extra_prefixes, &mut filters);

        // Both dimensions populated
        assert_eq!(filters.origin_asn, vec!["64496"]);
        assert_eq!(filters.communities, vec!["64496:100"]);
        assert_eq!(filters.prefix, vec!["192.0.2.0/24", "198.51.100.0/24"]);

        // Full validation passes
        assert!(filters.validate().is_ok());

        let _ = std::fs::remove_file(&json_path);
        let _ = std::fs::remove_file(&prefix_path);
    }

    #[test]
    fn test_cli_plus_filter_file_union_then_validate() {
        // Simulate: -p 10.0.0.0/8 --filter-file file_with_more_prefixes.json
        let json = r#"{ "prefixes": ["192.0.2.0/24", "2001:db8::/32"] }"#;
        let path = write_temp_file("union.json", json);

        let file = FilterFile::load(&path).unwrap();

        // CLI already has a prefix
        let mut filters = ParseFilters {
            prefix: vec!["10.0.0.0/8".to_string()],
            ..Default::default()
        };
        file.merge_into(&mut filters).unwrap();

        // Union of CLI + file prefixes
        assert_eq!(
            filters.prefix,
            vec!["10.0.0.0/8", "192.0.2.0/24", "2001:db8::/32"]
        );
        assert!(filters.validate().is_ok());

        let _ = std::fs::remove_file(&path);
    }
}
