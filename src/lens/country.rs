//! Country lookup lens
//!
//! This module provides country name and code lookup functionality
//! using data from bgpkit-commons.

use anyhow::{anyhow, Result};
use bgpkit_commons::BgpkitCommons;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tabled::Tabled;

/// Global country data cache
static COUNTRY_DATA: OnceLock<CountryData> = OnceLock::new();

/// Internal country data structure
struct CountryData {
    entries: Vec<CountryEntry>,
}

impl CountryData {
    fn load() -> Result<Self> {
        let mut commons = BgpkitCommons::new();
        commons
            .load_countries()
            .map_err(|e| anyhow!("Failed to load countries from bgpkit-commons: {}", e))?;

        let countries = commons
            .country_all()
            .map_err(|e| anyhow!("Failed to get countries: {}", e))?;

        let entries: Vec<CountryEntry> = countries
            .into_iter()
            .map(|c| CountryEntry {
                code: c.code,
                name: c.name,
            })
            .collect();

        Ok(Self { entries })
    }
}

/// A country entry with code and name
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct CountryEntry {
    pub code: String,
    pub name: String,
}

/// Country lookup lens
///
/// Provides methods for looking up countries by code or name.
/// Uses bgpkit-commons for country data with lazy loading.
pub struct CountryLens {
    // Using a reference to the global data
    _marker: std::marker::PhantomData<()>,
}

impl CountryLens {
    /// Create a new country lookup lens
    ///
    /// On first call, this will load country data from bgpkit-commons.
    /// Subsequent calls will use cached data.
    pub fn new() -> Self {
        // Initialize the global data if not already done
        let _ = COUNTRY_DATA.get_or_init(|| {
            CountryData::load().unwrap_or_else(|e| {
                tracing::warn!("Failed to load country data: {}. Using empty dataset.", e);
                CountryData {
                    entries: Vec::new(),
                }
            })
        });

        Self {
            _marker: std::marker::PhantomData,
        }
    }

    /// Get the country data, initializing if necessary
    fn data(&self) -> &CountryData {
        COUNTRY_DATA.get_or_init(|| {
            CountryData::load().unwrap_or_else(|_| CountryData {
                entries: Vec::new(),
            })
        })
    }

    /// Lookup a country name by its 2-letter code
    pub fn lookup_code(&self, code: &str) -> Option<&str> {
        let code_upper = code.to_uppercase();
        self.data()
            .entries
            .iter()
            .find(|e| e.code == code_upper)
            .map(|e| e.name.as_str())
    }

    /// Search for countries by code or name
    ///
    /// If the query matches a code exactly, returns only that country.
    /// Otherwise, returns all countries whose names contain the query.
    pub fn lookup(&self, query: &str) -> Vec<CountryEntry> {
        let mut entries = vec![];
        let query_lower = query.to_lowercase();
        let query_upper = query.to_uppercase();

        for entry in &self.data().entries {
            if entry.code == query_upper {
                // Exact code match - return only this
                return vec![entry.clone()];
            } else if entry.name.to_lowercase().contains(&query_lower) {
                entries.push(entry.clone());
            }
        }
        entries
    }

    /// Get all countries sorted by code
    pub fn all(&self) -> Vec<CountryEntry> {
        let mut entries = self.data().entries.clone();
        entries.sort_by(|a, b| a.code.cmp(&b.code));
        entries
    }
}

impl Default for CountryLens {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lookup_code() {
        let lookup = CountryLens::new();

        // Test US lookup
        let result = lookup.lookup_code("US");
        assert!(result.is_some());
        // Note: The exact name may vary slightly from bgpkit-commons
        let name = result.unwrap();
        assert!(name.contains("United States") || name.contains("America"));

        // Test lowercase
        let result_lower = lookup.lookup_code("us");
        assert!(result_lower.is_some());

        // Test non-existent code
        assert!(lookup.lookup_code("XX").is_none());
    }

    #[test]
    fn test_lookup_by_code() {
        let lookup = CountryLens::new();

        let results = lookup.lookup("US");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].code, "US");
    }

    #[test]
    fn test_lookup_by_name() {
        let lookup = CountryLens::new();

        let results = lookup.lookup("united");
        // Should find multiple countries containing "united"
        assert!(!results.is_empty());
    }

    #[test]
    fn test_all() {
        let lookup = CountryLens::new();
        let all = lookup.all();

        // Should have many countries
        assert!(!all.is_empty());

        // Should be sorted by code
        if all.len() > 1 {
            assert!(all[0].code < all[1].code);
        }
    }
}
