//! Prefix-to-ASN mapping lens
//!
//! This module provides functionality for mapping IP prefixes to their
//! originating Autonomous System Numbers (ASNs). It uses a trie-based
//! data structure for efficient lookups.

use anyhow::Result;
use ipnet::IpNet;
use ipnet_trie::IpnetTrie;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tabled::Tabled;

use crate::database::Pfx2asRecord;

// =============================================================================
// Types
// =============================================================================

/// A prefix-to-ASN mapping entry
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct Pfx2asEntry {
    /// Origin ASN
    pub asn: u32,
    /// Number of observations/count
    pub count: u32,
    /// IP prefix
    pub prefix: String,
}

/// Result of a prefix-to-ASN lookup
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct Pfx2asResult {
    /// The queried prefix
    pub prefix: String,
    /// List of origin ASNs
    pub asns: String,
    /// Match type (exact or longest)
    pub match_type: String,
}

/// Output format for Pfx2as lens results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Pfx2asOutputFormat {
    /// JSON format (default)
    #[default]
    Json,
    /// Table format
    Table,
    /// Simple text format (ASNs only)
    Simple,
}

/// Lookup mode for prefix queries
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Pfx2asLookupMode {
    /// Exact match only
    Exact,
    /// Longest prefix match (default)
    #[default]
    Longest,
}

// =============================================================================
// Args
// =============================================================================

/// Arguments for Pfx2as lookup operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct Pfx2asLookupArgs {
    /// IP prefix to look up
    #[cfg_attr(feature = "cli", clap(value_name = "PREFIX"))]
    pub prefix: String,

    /// Lookup mode (exact or longest prefix match)
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "longest"))]
    #[serde(default)]
    pub mode: Pfx2asLookupMode,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "json"))]
    #[serde(default)]
    pub format: Pfx2asOutputFormat,
}

impl Pfx2asLookupArgs {
    /// Create new args for a prefix lookup
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            mode: Pfx2asLookupMode::default(),
            format: Pfx2asOutputFormat::default(),
        }
    }

    /// Set lookup mode
    pub fn with_mode(mut self, mode: Pfx2asLookupMode) -> Self {
        self.mode = mode;
        self
    }

    /// Set output format
    pub fn with_format(mut self, format: Pfx2asOutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Set exact match mode
    pub fn exact(mut self) -> Self {
        self.mode = Pfx2asLookupMode::Exact;
        self
    }

    /// Set longest prefix match mode
    pub fn longest(mut self) -> Self {
        self.mode = Pfx2asLookupMode::Longest;
        self
    }
}

// =============================================================================
// Lens
// =============================================================================

const BGPKIT_PFX2AS_URL: &str = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";

/// Prefix-to-ASN mapping lens
///
/// Provides methods for mapping IP prefixes to their originating ASNs
/// using a trie-based data structure for efficient lookups.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::pfx2as::{Pfx2asLens, Pfx2asLookupArgs};
///
/// // Load the lens (downloads data from BGPKIT)
/// let lens = Pfx2asLens::new(None)?;
///
/// // Look up a prefix
/// let args = Pfx2asLookupArgs::new("1.1.1.0/24");
/// let asns = lens.lookup(&args)?;
///
/// println!("Origin ASNs: {:?}", asns);
/// ```
pub struct Pfx2asLens {
    trie: IpnetTrie<HashSet<u32>>,
}

impl Pfx2asLens {
    /// Create a new Pfx2as lens by loading data from the given path
    ///
    /// If no path is provided, downloads from the default BGPKIT URL.
    pub fn new(path_opt: Option<String>) -> Result<Self> {
        let path = path_opt.unwrap_or_else(|| BGPKIT_PFX2AS_URL.to_string());
        let entries = oneio::read_json_struct::<Vec<Pfx2asEntry>>(&path)?;

        let mut trie = IpnetTrie::<HashSet<u32>>::new();
        for entry in entries {
            if let Ok(prefix) = entry.prefix.parse::<IpNet>() {
                match trie.exact_match_mut(prefix) {
                    None => {
                        let set = HashSet::from_iter([entry.asn]);
                        trie.insert(prefix, set);
                    }
                    Some(s) => {
                        s.insert(entry.asn);
                    }
                }
            }
        }

        Ok(Self { trie })
    }

    /// Create a new Pfx2as lens from cached records
    ///
    /// This is used to build the trie from file-cached data.
    pub fn from_records(records: Vec<Pfx2asRecord>) -> Result<Self> {
        let mut trie = IpnetTrie::<HashSet<u32>>::new();

        for record in records {
            if let Ok(prefix) = record.prefix.parse::<IpNet>() {
                match trie.exact_match_mut(prefix) {
                    None => {
                        let set: HashSet<u32> = record.origin_asns.into_iter().collect();
                        trie.insert(prefix, set);
                    }
                    Some(s) => {
                        s.extend(record.origin_asns);
                    }
                }
            }
        }

        Ok(Self { trie })
    }

    /// Look up ASNs for a prefix based on the lookup args
    pub fn lookup(&self, args: &Pfx2asLookupArgs) -> Result<Vec<u32>> {
        let prefix: IpNet = args.prefix.parse()?;
        let asns = match args.mode {
            Pfx2asLookupMode::Exact => self.lookup_exact(prefix),
            Pfx2asLookupMode::Longest => self.lookup_longest(prefix),
        };
        Ok(asns)
    }

    /// Look up exact matches for the given IP prefix.
    ///
    /// This method searches for prefixes in the trie that exactly match the given `prefix`.
    /// If a match is found, it returns a vector containing ASNs associated with the matching prefix.
    /// If no match is found, an empty vector is returned.
    ///
    /// # Arguments
    ///
    /// * `prefix` - An `IpNet` object representing the IP prefix to be matched.
    ///
    /// # Returns
    ///
    /// A `Vec<u32>` containing ASNs associated with the matching prefix.
    /// If no exact matching prefix is found, the returned vector will be empty.
    pub fn lookup_exact(&self, prefix: IpNet) -> Vec<u32> {
        match self.trie.exact_match(prefix) {
            None => vec![],
            Some(s) => s.iter().cloned().collect(),
        }
    }

    /// Perform the longest prefix match (LPM) for the given IP prefix.
    ///
    /// This method finds the most specific prefix in the trie that matches
    /// the given IP prefix. It returns a list of ASNs associated with the
    /// longest matching prefix, if any exists.
    ///
    /// # Arguments
    ///
    /// * `prefix` - An `IpNet` object representing the IP prefix to be matched.
    ///
    /// # Returns
    ///
    /// A `Vec<u32>` containing ASNs associated with the longest matching prefix.
    /// If no matching prefix is found, the returned vector will be empty.
    pub fn lookup_longest(&self, prefix: IpNet) -> Vec<u32> {
        match self.trie.longest_match(&prefix) {
            None => vec![],
            Some((_p, s)) => s.iter().cloned().collect(),
        }
    }

    /// Format lookup results for display
    pub fn format_result(
        &self,
        prefix: &str,
        asns: &[u32],
        mode: &Pfx2asLookupMode,
        format: &Pfx2asOutputFormat,
    ) -> String {
        let match_type = match mode {
            Pfx2asLookupMode::Exact => "exact",
            Pfx2asLookupMode::Longest => "longest",
        };

        match format {
            Pfx2asOutputFormat::Simple => asns
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(" "),
            Pfx2asOutputFormat::Json => {
                let result = Pfx2asResult {
                    prefix: prefix.to_string(),
                    asns: asns
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    match_type: match_type.to_string(),
                };
                serde_json::to_string_pretty(&result).unwrap_or_default()
            }
            Pfx2asOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;

                let result = Pfx2asResult {
                    prefix: prefix.to_string(),
                    asns: asns
                        .iter()
                        .map(|a| a.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    match_type: match_type.to_string(),
                };
                Table::new(vec![result]).with(Style::rounded()).to_string()
            }
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require network access to download the pfx2as data
    // They are ignored by default to avoid slow CI runs

    #[test]
    #[ignore]
    fn test_pfx2as_lens() {
        let lens = Pfx2asLens::new(None).unwrap();

        // Test exact match
        let prefix: IpNet = "1.1.1.0/24".parse().unwrap();
        let asns = lens.lookup_exact(prefix);
        println!("Exact match for 1.1.1.0/24: {:?}", asns);

        // Test longest match
        let asns = lens.lookup_longest(prefix);
        println!("Longest match for 1.1.1.0/24: {:?}", asns);
    }

    #[test]
    fn test_format_result() {
        // Create a minimal lens for testing format
        let lens = Pfx2asLens {
            trie: IpnetTrie::new(),
        };

        let asns = vec![13335, 13336];
        let output = lens.format_result(
            "1.1.1.0/24",
            &asns,
            &Pfx2asLookupMode::Exact,
            &Pfx2asOutputFormat::Simple,
        );
        assert_eq!(output, "13335 13336");
    }

    #[test]
    fn test_lookup_args() {
        let args = Pfx2asLookupArgs::new("1.1.1.0/24")
            .exact()
            .with_format(Pfx2asOutputFormat::Table);

        assert_eq!(args.prefix, "1.1.1.0/24");
        assert!(matches!(args.mode, Pfx2asLookupMode::Exact));
        assert!(matches!(args.format, Pfx2asOutputFormat::Table));
    }
}
