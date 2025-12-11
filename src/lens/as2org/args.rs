//! AS2Org lens arguments
//!
//! This module defines the argument structures for AS2Org operations.
//! These arguments are designed to be reusable across CLI, REST API,
//! WebSocket, and GUI interfaces.

use serde::{Deserialize, Serialize};

use super::types::{As2orgOutputFormat, As2orgSearchType};

/// Arguments for AS2Org search operations
///
/// This struct can be used in multiple contexts:
/// - CLI: with clap derives (when `cli` feature is enabled)
/// - REST API: as query parameters (via serde)
/// - WebSocket: as JSON message payload (via serde)
/// - GUI: as form state (via serde)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct As2orgSearchArgs {
    /// Search queries: ASN (e.g., "400644") or name (e.g., "bgpkit")
    #[cfg_attr(feature = "cli", clap(required = true))]
    pub query: Vec<String>,

    /// Search AS and Org name only
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub name_only: bool,

    /// Search by ASN only
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub asn_only: bool,

    /// Search by country only
    #[cfg_attr(feature = "cli", clap(short = 'C', long))]
    #[serde(default)]
    pub country_only: bool,

    /// Show full country names instead of 2-letter code
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub full_country: bool,

    /// Display full table (with org_id, org_size)
    #[cfg_attr(feature = "cli", clap(short = 'F', long))]
    #[serde(default)]
    pub full_table: bool,
}

impl As2orgSearchArgs {
    /// Create new search arguments with a single query
    pub fn new(query: &str) -> Self {
        Self {
            query: vec![query.to_string()],
            ..Default::default()
        }
    }

    /// Create new search arguments with multiple queries
    pub fn with_queries(queries: Vec<String>) -> Self {
        Self {
            query: queries,
            ..Default::default()
        }
    }

    /// Set name-only search mode
    pub fn name_only(mut self) -> Self {
        self.name_only = true;
        self
    }

    /// Set ASN-only search mode
    pub fn asn_only(mut self) -> Self {
        self.asn_only = true;
        self
    }

    /// Set country-only search mode
    pub fn country_only(mut self) -> Self {
        self.country_only = true;
        self
    }

    /// Set full country names
    pub fn full_country(mut self) -> Self {
        self.full_country = true;
        self
    }

    /// Set full table output
    pub fn full_table(mut self) -> Self {
        self.full_table = true;
        self
    }

    /// Determine the search type based on flags
    pub fn search_type(&self) -> As2orgSearchType {
        match (self.name_only, self.asn_only, self.country_only) {
            (true, false, false) => As2orgSearchType::NameOnly,
            (false, true, false) => As2orgSearchType::AsnOnly,
            (false, false, true) => As2orgSearchType::CountryOnly,
            _ => As2orgSearchType::Guess,
        }
    }

    /// Validate the arguments
    ///
    /// Returns an error message if the arguments are invalid.
    pub fn validate(&self) -> Result<(), String> {
        // Check for conflicting flags
        let flag_count = [self.name_only, self.asn_only, self.country_only]
            .iter()
            .filter(|&&x| x)
            .count();

        if flag_count > 1 {
            return Err(
                "Cannot specify multiple search type flags (name-only, asn-only, country-only)"
                    .to_string(),
            );
        }

        // Ensure at least one query is provided
        if self.query.is_empty() {
            return Err("At least one search query is required".to_string());
        }

        Ok(())
    }
}

/// Arguments for AS2Org update operations
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct As2orgUpdateArgs {
    /// Force update even if data is fresh
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub force: bool,
}

/// Arguments for AS2Org output formatting
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct As2orgOutputArgs {
    /// Output format
    #[cfg_attr(feature = "cli", clap(skip))]
    #[serde(default)]
    pub format: As2orgOutputFormat,

    /// Output to pretty table (shortcut for format = Pretty)
    #[cfg_attr(feature = "cli", clap(short, long))]
    #[serde(default)]
    pub pretty: bool,

    /// Output as JSON (shortcut for format = Json)
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub json: bool,

    /// Export to pipe-separated values (shortcut for format = Psv)
    #[cfg_attr(feature = "cli", clap(short = 'P', long))]
    #[serde(default)]
    pub psv: bool,
}

impl As2orgOutputArgs {
    /// Determine the output format based on flags
    pub fn output_format(&self) -> As2orgOutputFormat {
        if self.json {
            As2orgOutputFormat::Json
        } else if self.psv {
            As2orgOutputFormat::Psv
        } else if self.pretty {
            As2orgOutputFormat::Pretty
        } else {
            self.format.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_args_builder() {
        let args = As2orgSearchArgs::new("cloudflare")
            .name_only()
            .full_country();

        assert_eq!(args.query, vec!["cloudflare"]);
        assert!(args.name_only);
        assert!(args.full_country);
        assert!(!args.asn_only);
    }

    #[test]
    fn test_search_type() {
        let args = As2orgSearchArgs::new("test");
        assert_eq!(args.search_type(), As2orgSearchType::Guess);

        let args = As2orgSearchArgs::new("test").name_only();
        assert_eq!(args.search_type(), As2orgSearchType::NameOnly);

        let args = As2orgSearchArgs::new("test").asn_only();
        assert_eq!(args.search_type(), As2orgSearchType::AsnOnly);

        let args = As2orgSearchArgs::new("test").country_only();
        assert_eq!(args.search_type(), As2orgSearchType::CountryOnly);
    }

    #[test]
    fn test_validate_conflicting_flags() {
        let args = As2orgSearchArgs {
            query: vec!["test".to_string()],
            name_only: true,
            asn_only: true,
            ..Default::default()
        };

        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_empty_query() {
        let args = As2orgSearchArgs::default();
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_output_format() {
        let args = As2orgOutputArgs::default();
        assert_eq!(args.output_format(), As2orgOutputFormat::Markdown);

        let args = As2orgOutputArgs {
            json: true,
            ..Default::default()
        };
        assert_eq!(args.output_format(), As2orgOutputFormat::Json);

        let args = As2orgOutputArgs {
            pretty: true,
            ..Default::default()
        };
        assert_eq!(args.output_format(), As2orgOutputFormat::Pretty);

        let args = As2orgOutputArgs {
            psv: true,
            ..Default::default()
        };
        assert_eq!(args.output_format(), As2orgOutputFormat::Psv);
    }
}
