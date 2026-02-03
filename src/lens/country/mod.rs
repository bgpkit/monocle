//! Country lookup lens
//!
//! This module provides country name and code lookup functionality
//! using data from bgpkit-commons.
//!
//! # Feature Requirements
//!
//! This module requires the `lib` feature.
//!
//! # Example
//!
//! ```rust,ignore
//! use monocle::lens::country::{CountryLens, CountryLookupArgs};
//!
//! let lens = CountryLens::new();
//!
//! // Look up by country code
//! let args = CountryLookupArgs::new("US");
//! let results = lens.search(&args)?;
//!
//! for country in &results {
//!     println!("{}: {}", country.code, country.name);
//! }
//! ```

use anyhow::{anyhow, Result};
use bgpkit_commons::BgpkitCommons;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

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

// =============================================================================
// Types
// =============================================================================

/// A country entry with code and name
#[derive(Debug, Clone, Serialize, Deserialize, tabled::Tabled)]
pub struct CountryEntry {
    /// ISO 3166-1 alpha-2 country code
    pub code: String,
    /// Full country name
    pub name: String,
}

/// Output format for country lens results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum CountryOutputFormat {
    /// Table format with borders (default)
    #[default]
    Table,
    /// JSON format
    Json,
    /// Simple text format (code: name)
    Simple,
    /// Markdown table
    Markdown,
}

// =============================================================================
// Args
// =============================================================================

/// Arguments for country lookup operations
///
/// This struct works in multiple contexts:
/// - CLI: with clap derives (when `cli` feature is enabled)
/// - REST API: as query parameters or JSON body (via serde)
/// - WebSocket: as JSON message payload (via serde)
/// - Library: constructed programmatically
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct CountryLookupArgs {
    /// Search query: country code (e.g., "US") or partial name (e.g., "united")
    #[cfg_attr(feature = "cli", clap(value_name = "QUERY"))]
    pub query: Option<String>,

    /// List all countries
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub all: bool,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "table"))]
    #[serde(default)]
    pub format: CountryOutputFormat,
}

impl CountryLookupArgs {
    /// Create new args with a query
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: Some(query.into()),
            all: false,
            format: CountryOutputFormat::default(),
        }
    }

    /// Create args to list all countries
    pub fn all_countries() -> Self {
        Self {
            query: None,
            all: true,
            format: CountryOutputFormat::default(),
        }
    }

    /// Set output format
    pub fn with_format(mut self, format: CountryOutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Validate the arguments
    pub fn validate(&self) -> Result<(), String> {
        if !self.all && self.query.is_none() {
            return Err("Either a query or --all flag is required".to_string());
        }
        Ok(())
    }
}

// =============================================================================
// Lens
// =============================================================================

/// Country lookup lens
///
/// Provides methods for looking up countries by code or name.
/// Uses bgpkit-commons for country data with lazy loading.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::country::{CountryLens, CountryLookupArgs, CountryOutputFormat};
///
/// let lens = CountryLens::new();
///
/// // Look up by country code
/// let args = CountryLookupArgs::new("US");
/// let results = lens.search(&args)?;
///
/// // Format for display (requires "display" feature for Table format)
/// let output = lens.format_results(&results, &CountryOutputFormat::Json);
/// println!("{}", output);
/// ```
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

    /// Search for countries based on the provided arguments
    pub fn search(&self, args: &CountryLookupArgs) -> Result<Vec<CountryEntry>> {
        if args.all {
            return Ok(self.all());
        }

        match &args.query {
            Some(query) => Ok(self.lookup(query)),
            None => Err(anyhow!("Either a query or --all flag is required")),
        }
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

    /// Format results based on output format
    ///
    /// Note: Table and Markdown formats require the `display` feature.
    /// Without it, they will fall back to Simple format.
    pub fn format_results(&self, results: &[CountryEntry], format: &CountryOutputFormat) -> String {
        if results.is_empty() {
            return match format {
                CountryOutputFormat::Json => "[]".to_string(),
                _ => "No countries found".to_string(),
            };
        }

        match format {
            CountryOutputFormat::Table => {
                use tabled::settings::Style;
                use tabled::Table;
                Table::new(results).with(Style::rounded()).to_string()
            }
            CountryOutputFormat::Markdown => {
                use tabled::settings::Style;
                use tabled::Table;
                Table::new(results).with(Style::markdown()).to_string()
            }
            CountryOutputFormat::Json => serde_json::to_string_pretty(results).unwrap_or_default(),
            CountryOutputFormat::Simple => results
                .iter()
                .map(|e| format!("{}: {}", e.code, e.name))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Format results as JSON
    ///
    /// This is a convenience method that always works regardless of features.
    pub fn format_json(&self, results: &[CountryEntry], pretty: bool) -> String {
        if pretty {
            serde_json::to_string_pretty(results).unwrap_or_default()
        } else {
            serde_json::to_string(results).unwrap_or_default()
        }
    }
}

impl Default for CountryLens {
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
    fn test_lookup_code() {
        let lens = CountryLens::new();

        // Test US lookup
        let result = lens.lookup_code("US");
        assert!(result.is_some());
        // Note: The exact name may vary slightly from bgpkit-commons
        let name = result.unwrap();
        assert!(name.contains("United States") || name.contains("America"));

        // Test lowercase
        let result_lower = lens.lookup_code("us");
        assert!(result_lower.is_some());

        // Test non-existent code
        assert!(lens.lookup_code("XX").is_none());
    }

    #[test]
    fn test_lookup_by_code() {
        let lens = CountryLens::new();

        let results = lens.lookup("US");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].code, "US");
    }

    #[test]
    fn test_lookup_by_name() {
        let lens = CountryLens::new();

        let results = lens.lookup("united");
        // Should find multiple countries containing "united"
        assert!(!results.is_empty());
    }

    #[test]
    fn test_all() {
        let lens = CountryLens::new();
        let all = lens.all();

        // Should have many countries
        assert!(!all.is_empty());

        // Should be sorted by code
        if all.len() > 1 {
            assert!(all[0].code < all[1].code);
        }
    }

    #[test]
    fn test_search_with_args() {
        let lens = CountryLens::new();

        // Test with query
        let args = CountryLookupArgs::new("US");
        let results = lens.search(&args).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].code, "US");

        // Test with all flag
        let args = CountryLookupArgs::all_countries();
        let results = lens.search(&args).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_args_validation() {
        // Empty args should fail validation
        let args = CountryLookupArgs::default();
        assert!(args.validate().is_err());

        // Args with query should pass
        let args = CountryLookupArgs::new("US");
        assert!(args.validate().is_ok());

        // Args with all flag should pass
        let args = CountryLookupArgs::all_countries();
        assert!(args.validate().is_ok());
    }

    #[test]
    fn test_format_results() {
        let lens = CountryLens::new();
        let results = vec![
            CountryEntry {
                code: "US".to_string(),
                name: "United States".to_string(),
            },
            CountryEntry {
                code: "CA".to_string(),
                name: "Canada".to_string(),
            },
        ];

        // Test JSON format
        let output = lens.format_results(&results, &CountryOutputFormat::Json);
        assert!(output.contains("US"));
        assert!(output.contains("United States"));

        // Test Simple format
        let output = lens.format_results(&results, &CountryOutputFormat::Simple);
        assert!(output.contains("US: United States"));
        assert!(output.contains("CA: Canada"));

        // Test empty results
        let output = lens.format_results(&[], &CountryOutputFormat::Simple);
        assert_eq!(output, "No countries found");

        let output = lens.format_results(&[], &CountryOutputFormat::Json);
        assert_eq!(output, "[]");
    }

    #[test]
    fn test_format_json() {
        let lens = CountryLens::new();
        let results = vec![CountryEntry {
            code: "US".to_string(),
            name: "United States".to_string(),
        }];

        let compact = lens.format_json(&results, false);
        assert!(compact.contains("US"));

        let pretty = lens.format_json(&results, true);
        assert!(pretty.contains('\n'));
    }

    #[test]
    fn test_args_builder() {
        let args = CountryLookupArgs::new("US").with_format(CountryOutputFormat::Json);

        assert_eq!(args.query, Some("US".to_string()));
        assert!(matches!(args.format, CountryOutputFormat::Json));
    }
}
