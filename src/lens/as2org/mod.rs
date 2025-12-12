//! AS2Org lens
//!
//! This module provides the AS2Org lens for querying AS-to-Organization mappings.
//! It uses SQLite as the backend database.

pub mod args;
pub mod types;

pub use args::{As2orgOutputArgs, As2orgSearchArgs, As2orgUpdateArgs};
pub use types::{
    As2orgOutputFormat, As2orgSearchResult, As2orgSearchResultConcise, As2orgSearchType,
    As2orgUpdateProgress, As2orgUpdateStage,
};

use crate::database::MonocleDatabase;
use crate::lens::country::CountryLens;
use anyhow::{anyhow, Result};
use tabled::settings::Style;
use tabled::Table;

/// AS2Org lens for querying AS-to-Organization mappings
///
/// This lens provides high-level operations for:
/// - Searching for ASes by ASN, name, or country
/// - Updating/bootstrapping AS2Org data
/// - Formatting results for output
pub struct As2orgLens<'a> {
    db: &'a MonocleDatabase,
    country_lookup: CountryLens,
}

impl<'a> As2orgLens<'a> {
    /// Create a new AS2Org lens
    pub fn new(db: &'a MonocleDatabase) -> Self {
        Self {
            db,
            country_lookup: CountryLens::new(),
        }
    }

    /// Check if data is available (bootstrapped)
    pub fn is_data_available(&self) -> bool {
        !self.db.as2org().is_empty()
    }

    /// Check if data needs to be bootstrapped
    pub fn needs_bootstrap(&self) -> bool {
        self.db.needs_as2org_bootstrap()
    }

    /// Bootstrap AS2Org data from bgpkit-commons
    ///
    /// Returns the count of entries loaded.
    pub fn bootstrap(&self) -> Result<usize> {
        let (as_count, _org_count) = self.db.bootstrap_as2org()?;
        Ok(as_count)
    }

    /// Lookup organization name for a single ASN
    pub fn lookup_org_name(&self, asn: u32) -> Option<String> {
        self.db.as2org().lookup_org_name(asn)
    }

    /// Batch lookup of organization names for multiple ASNs
    pub fn lookup_org_names_batch(&self, asns: &[u32]) -> std::collections::HashMap<u32, String> {
        self.db.as2org().lookup_org_names_batch(asns)
    }

    /// Search using the provided arguments
    pub fn search(&self, args: &As2orgSearchArgs) -> Result<Vec<As2orgSearchResult>> {
        let search_type = args.search_type();
        let mut all_results = Vec::new();

        for query in &args.query {
            let results = self.search_single(query, &search_type, args.full_country)?;
            all_results.extend(results);
        }

        // Sort by ASN
        all_results.sort_by_key(|r| r.asn);

        Ok(all_results)
    }

    /// Search for a single query
    fn search_single(
        &self,
        query: &str,
        search_type: &As2orgSearchType,
        full_country: bool,
    ) -> Result<Vec<As2orgSearchResult>> {
        let repo = self.db.as2org();

        let records = match search_type {
            As2orgSearchType::AsnOnly => {
                let asn = query
                    .parse::<u32>()
                    .map_err(|_| anyhow!("Invalid ASN: {}", query))?;
                repo.search_by_asn(asn)?
            }
            As2orgSearchType::NameOnly => repo.search_by_name(query)?,
            As2orgSearchType::CountryOnly => {
                // Resolve country code from query
                let countries = self.country_lookup.lookup(query);
                if countries.is_empty() {
                    return Err(anyhow!("No country found matching: {}", query));
                } else if countries.len() > 1 {
                    let names: Vec<_> = countries.iter().map(|c| c.name.as_str()).collect();
                    return Err(anyhow!(
                        "Multiple countries match '{}': {}",
                        query,
                        names.join(", ")
                    ));
                }
                repo.search_by_country(&countries[0].code)?
            }
            As2orgSearchType::Guess => {
                // Try to parse as ASN first, then fall back to name search
                if let Ok(asn) = query.parse::<u32>() {
                    repo.search_by_asn(asn)?
                } else {
                    repo.search_by_name(query)?
                }
            }
        };

        // Convert records to results with optional country name expansion
        let results: Vec<As2orgSearchResult> = records
            .into_iter()
            .map(|r| self.record_to_result(r, full_country))
            .collect();

        // Return placeholder if empty
        if results.is_empty() {
            let placeholder = match search_type {
                As2orgSearchType::AsnOnly => {
                    As2orgSearchResult::not_found_asn(query.parse().unwrap_or(0))
                }
                As2orgSearchType::NameOnly | As2orgSearchType::Guess => {
                    As2orgSearchResult::not_found_name(query)
                }
                As2orgSearchType::CountryOnly => As2orgSearchResult::not_found_country(query),
            };
            Ok(vec![placeholder])
        } else {
            Ok(results)
        }
    }

    /// Convert a database record to a search result
    fn record_to_result(
        &self,
        record: crate::database::As2orgRecord,
        full_country: bool,
    ) -> As2orgSearchResult {
        let country = if full_country {
            self.country_lookup
                .lookup_code(&record.country)
                .map(|s| s.to_string())
                .unwrap_or(record.country.clone())
        } else {
            record.country.clone()
        };

        As2orgSearchResult {
            asn: record.asn,
            as_name: record.as_name,
            org_name: record.org_name,
            org_id: record.org_id,
            org_country: country,
            org_size: record.org_size,
        }
    }

    /// Format results for output
    ///
    /// When `truncate_names` is true, names are truncated to 20 characters for table output.
    /// JSON output never truncates names regardless of this parameter.
    pub fn format_results(
        &self,
        results: &[As2orgSearchResult],
        format: &As2orgOutputFormat,
        full_table: bool,
        truncate_names: bool,
    ) -> String {
        match format {
            As2orgOutputFormat::Json => {
                if full_table {
                    serde_json::to_string_pretty(results).unwrap_or_default()
                } else {
                    let concise: Vec<As2orgSearchResultConcise> =
                        results.iter().cloned().map(Into::into).collect();
                    serde_json::to_string_pretty(&concise).unwrap_or_default()
                }
            }
            As2orgOutputFormat::Psv => {
                let mut output = String::new();
                if full_table {
                    output.push_str("asn|asn_name|org_name|org_id|org_country|org_size\n");
                    for r in results {
                        output.push_str(&format!(
                            "{}|{}|{}|{}|{}|{}\n",
                            r.asn, r.as_name, r.org_name, r.org_id, r.org_country, r.org_size
                        ));
                    }
                } else {
                    output.push_str("asn|asn_name|org_name|org_country\n");
                    for r in results {
                        output.push_str(&format!(
                            "{}|{}|{}|{}\n",
                            r.asn, r.as_name, r.org_name, r.org_country
                        ));
                    }
                }
                output
            }
            As2orgOutputFormat::Pretty => {
                if full_table {
                    let display: Vec<As2orgSearchResult> = results
                        .iter()
                        .map(|r| r.to_truncated(truncate_names))
                        .collect();
                    Table::new(display).with(Style::rounded()).to_string()
                } else {
                    let concise: Vec<As2orgSearchResultConcise> = results
                        .iter()
                        .map(|r| r.to_concise_truncated(truncate_names))
                        .collect();
                    Table::new(concise).with(Style::rounded()).to_string()
                }
            }
            As2orgOutputFormat::Markdown => {
                if full_table {
                    let display: Vec<As2orgSearchResult> = results
                        .iter()
                        .map(|r| r.to_truncated(truncate_names))
                        .collect();
                    Table::new(display).with(Style::markdown()).to_string()
                } else {
                    let concise: Vec<As2orgSearchResultConcise> = results
                        .iter()
                        .map(|r| r.to_concise_truncated(truncate_names))
                        .collect();
                    Table::new(concise).with(Style::markdown()).to_string()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_creation() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = As2orgLens::new(&db);
        assert!(!lens.is_data_available());
        assert!(lens.needs_bootstrap());
    }

    #[test]
    fn test_format_results_json() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = As2orgLens::new(&db);

        let results = vec![As2orgSearchResult {
            asn: 65000,
            as_name: "Test AS".to_string(),
            org_name: "Test Org".to_string(),
            org_id: "TEST-ORG".to_string(),
            org_country: "US".to_string(),
            org_size: 1,
        }];

        let output = lens.format_results(&results, &As2orgOutputFormat::Json, true, false);
        assert!(output.contains("65000"));
        assert!(output.contains("Test AS"));
    }

    #[test]
    fn test_format_results_psv() {
        let db = MonocleDatabase::open_in_memory().unwrap();
        let lens = As2orgLens::new(&db);

        let results = vec![As2orgSearchResult {
            asn: 65000,
            as_name: "Test AS".to_string(),
            org_name: "Test Org".to_string(),
            org_id: "TEST-ORG".to_string(),
            org_country: "US".to_string(),
            org_size: 1,
        }];

        let output = lens.format_results(&results, &As2orgOutputFormat::Psv, false, false);
        assert!(output.contains("asn|asn_name|org_name|org_country"));
        assert!(output.contains("65000|Test AS|Test Org|US"));
    }
}
