//! AS2Org lens types
//!
//! This module defines the types used by the AS2Org lens for search
//! operations and result formatting.

use serde::{Deserialize, Serialize};
use tabled::Tabled;

/// Type of search to perform
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum As2orgSearchType {
    /// Search by ASN only
    AsnOnly,
    /// Search by name only (AS name or organization name)
    NameOnly,
    /// Search by country only
    CountryOnly,
    /// Automatically determine search type based on query
    #[default]
    Guess,
}

/// Output format for results
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum As2orgOutputFormat {
    /// Markdown table
    #[default]
    Markdown,
    /// Pretty table with borders
    Pretty,
    /// JSON output
    Json,
    /// Pipe-separated values
    Psv,
}

/// Full search result with all fields
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct As2orgSearchResult {
    pub asn: u32,
    pub as_name: String,
    pub org_name: String,
    pub org_id: String,
    pub org_country: String,
    pub org_size: u32,
}

/// Concise search result (without org_id and org_size)
#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct As2orgSearchResultConcise {
    pub asn: u32,
    pub as_name: String,
    pub org_name: String,
    pub org_country: String,
}

impl From<As2orgSearchResult> for As2orgSearchResultConcise {
    fn from(result: As2orgSearchResult) -> Self {
        Self {
            asn: result.asn,
            as_name: result.as_name,
            org_name: result.org_name,
            org_country: result.org_country,
        }
    }
}

impl As2orgSearchResult {
    /// Create a "not found" placeholder result for ASN search
    pub fn not_found_asn(asn: u32) -> Self {
        Self {
            asn,
            as_name: "?".to_string(),
            org_name: "?".to_string(),
            org_id: "?".to_string(),
            org_country: "?".to_string(),
            org_size: 0,
        }
    }

    /// Create a "not found" placeholder result for name search
    pub fn not_found_name(query: &str) -> Self {
        Self {
            asn: 0,
            as_name: "?".to_string(),
            org_name: query.to_string(),
            org_id: "?".to_string(),
            org_country: "?".to_string(),
            org_size: 0,
        }
    }

    /// Create a "not found" placeholder result for country search
    pub fn not_found_country(query: &str) -> Self {
        Self {
            asn: 0,
            as_name: "?".to_string(),
            org_name: "?".to_string(),
            org_id: "?".to_string(),
            org_country: query.to_string(),
            org_size: 0,
        }
    }
}

/// Progress update for data loading operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct As2orgUpdateProgress {
    pub stage: As2orgUpdateStage,
    pub current: usize,
    pub total: Option<usize>,
    pub message: String,
}

/// Stage of a data update operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum As2orgUpdateStage {
    /// Downloading data from source
    Downloading,
    /// Parsing downloaded data
    Parsing,
    /// Inserting data into database
    Inserting,
    /// Operation completed successfully
    Complete,
    /// Operation failed with error
    Error,
}

impl As2orgUpdateProgress {
    pub fn downloading() -> Self {
        Self {
            stage: As2orgUpdateStage::Downloading,
            current: 0,
            total: None,
            message: "Downloading data...".to_string(),
        }
    }

    pub fn parsing() -> Self {
        Self {
            stage: As2orgUpdateStage::Parsing,
            current: 0,
            total: None,
            message: "Parsing data...".to_string(),
        }
    }

    pub fn inserting(current: usize, total: usize) -> Self {
        Self {
            stage: As2orgUpdateStage::Inserting,
            current,
            total: Some(total),
            message: format!("Inserting records ({}/{})", current, total),
        }
    }

    pub fn complete(as_count: usize, org_count: usize) -> Self {
        Self {
            stage: As2orgUpdateStage::Complete,
            current: as_count,
            total: Some(org_count),
            message: format!("Complete: {} ASes, {} organizations", as_count, org_count),
        }
    }

    pub fn error(message: &str) -> Self {
        Self {
            stage: As2orgUpdateStage::Error,
            current: 0,
            total: None,
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_result_not_found() {
        let result = As2orgSearchResult::not_found_asn(65000);
        assert_eq!(result.asn, 65000);
        assert_eq!(result.as_name, "?");

        let result = As2orgSearchResult::not_found_name("test");
        assert_eq!(result.org_name, "test");

        let result = As2orgSearchResult::not_found_country("US");
        assert_eq!(result.org_country, "US");
    }

    #[test]
    fn test_search_result_to_concise() {
        let result = As2orgSearchResult {
            asn: 65000,
            as_name: "Test AS".to_string(),
            org_name: "Test Org".to_string(),
            org_id: "TEST-ORG".to_string(),
            org_country: "US".to_string(),
            org_size: 10,
        };

        let concise: As2orgSearchResultConcise = result.into();
        assert_eq!(concise.asn, 65000);
        assert_eq!(concise.as_name, "Test AS");
        assert_eq!(concise.org_name, "Test Org");
        assert_eq!(concise.org_country, "US");
    }
}
