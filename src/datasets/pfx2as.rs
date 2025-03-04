//! Prefix-to-ASN mapping tool

use anyhow::Result;
use ipnet::IpNet;
use ipnet_trie::IpnetTrie;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A data structure for performing prefix-to-ASN mappings.
///
/// The `Pfx2as` struct uses an internal trie to organize IP prefixes
/// and their associated Autonomous System Numbers (ASNs). It provides
/// functionality for loading prefix-to-ASN mappings from a source file and
/// methods for performing exact and longest prefix matches.
///
/// # Features
/// - Load prefix-to-ASN mappings from a JSON data source (`pfx2as-latest.json.bz2`).
/// - Perform exact match lookups with the `lookup_exact` method.
/// - Perform longest prefix match (LPM) lookups with the `lookup_longest` method.
pub struct Pfx2as {
    trie: IpnetTrie<HashSet<u32>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Pfx2asEntry {
    asn: u32,
    count: u32,
    prefix: String,
}

const BGPKIT_PFX2AS_URL: &str = "https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2";

impl Pfx2as {
    pub fn new(path_opt: Option<String>) -> Result<Pfx2as> {
        let path = path_opt.unwrap_or(BGPKIT_PFX2AS_URL.to_string());
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
        Ok(Pfx2as { trie })
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
            None => {
                vec![]
            }
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
            None => {
                vec![]
            }
            Some((_p, s)) => s.iter().cloned().collect(),
        }
    }
}
