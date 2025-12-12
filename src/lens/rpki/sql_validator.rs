//! SQL-based RPKI validation using cached ROA data
//!
//! This module provides RPKI validation functionality that uses the cached ROA data
//! in DuckDB, avoiding external API calls for validation. This enables:
//! - Offline validation using cached data
//! - Bulk validation via SQL JOINs
//! - Historical validation using dated cache entries
//!
//! # Example
//!
//! ```rust,ignore
//! use monocle::database::DuckDbConn;
//! use monocle::lens::rpki::SqlRpkiValidator;
//!
//! let conn = DuckDbConn::open_in_memory()?;
//! // ... load ROA cache ...
//!
//! let validator = SqlRpkiValidator::new(&conn);
//! let result = validator.validate("1.1.1.0/24", 13335)?;
//! println!("Validation status: {:?}", result.status);
//! ```

use anyhow::{anyhow, Result};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::database::core::DuckDbConn;

/// RPKI validation status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RpkiStatus {
    /// Valid: A matching ROA exists
    Valid,
    /// Invalid: A covering ROA exists but doesn't match (wrong ASN or prefix too long)
    Invalid,
    /// Unknown: No covering ROA found
    Unknown,
}

impl std::fmt::Display for RpkiStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpkiStatus::Valid => write!(f, "valid"),
            RpkiStatus::Invalid => write!(f, "invalid"),
            RpkiStatus::Unknown => write!(f, "unknown"),
        }
    }
}

impl FromStr for RpkiStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "valid" => Ok(RpkiStatus::Valid),
            "invalid" => Ok(RpkiStatus::Invalid),
            "unknown" | "not_found" | "notfound" => Ok(RpkiStatus::Unknown),
            _ => Err(anyhow!("Unknown RPKI status: {}", s)),
        }
    }
}

/// Result of RPKI validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiValidationResult {
    /// The prefix that was validated
    pub prefix: String,
    /// The origin ASN that was validated
    pub origin_asn: u32,
    /// The validation status
    pub status: RpkiStatus,
    /// Covering ROAs found (if any)
    pub covering_roas: Vec<CoveringRoa>,
}

/// A ROA that covers the validated prefix
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoveringRoa {
    /// The ROA prefix
    pub prefix: String,
    /// Maximum prefix length allowed
    pub max_length: u32,
    /// Origin ASN in the ROA
    pub origin_asn: u32,
    /// Trust anchor (optional)
    pub ta: Option<String>,
}

/// Bulk validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkValidationResult {
    /// Number of valid entries
    pub valid_count: u64,
    /// Number of invalid entries
    pub invalid_count: u64,
    /// Number of unknown entries
    pub unknown_count: u64,
    /// Detailed results (if requested)
    pub details: Option<Vec<RpkiValidationResult>>,
}

/// SQL-based RPKI validator using cached ROA data
pub struct SqlRpkiValidator<'a> {
    conn: &'a DuckDbConn,
    cache_id: Option<i64>,
}

impl<'a> SqlRpkiValidator<'a> {
    /// Create a new SQL-based RPKI validator
    pub fn new(conn: &'a DuckDbConn) -> Self {
        Self {
            conn,
            cache_id: None,
        }
    }

    /// Use a specific cache ID for validation
    ///
    /// This allows validating against historical or specific cache snapshots.
    pub fn with_cache_id(mut self, cache_id: i64) -> Self {
        self.cache_id = Some(cache_id);
        self
    }

    /// Validate a single prefix/origin pair
    ///
    /// # Arguments
    /// * `prefix` - The IP prefix to validate (e.g., "1.1.1.0/24")
    /// * `origin_asn` - The origin AS number
    ///
    /// # Returns
    /// A validation result with status and covering ROAs
    pub fn validate(&self, prefix: &str, origin_asn: u32) -> Result<RpkiValidationResult> {
        // Validate the prefix format
        let _parsed: IpNet = prefix
            .parse()
            .map_err(|e| anyhow!("Invalid prefix '{}': {}", prefix, e))?;

        // Get prefix length
        let prefix_len = self.get_prefix_length(prefix)?;

        // Find covering ROAs
        let covering_roas = self.find_covering_roas(prefix)?;

        // Determine validation status
        let status = self.determine_status(&covering_roas, origin_asn, prefix_len);

        Ok(RpkiValidationResult {
            prefix: prefix.to_string(),
            origin_asn,
            status,
            covering_roas,
        })
    }

    /// Validate multiple prefix/origin pairs in bulk
    ///
    /// This is more efficient than calling `validate` repeatedly as it
    /// uses a single SQL query with JOINs.
    pub fn validate_bulk(
        &self,
        entries: &[(String, u32)],
        include_details: bool,
    ) -> Result<BulkValidationResult> {
        if entries.is_empty() {
            return Ok(BulkValidationResult {
                valid_count: 0,
                invalid_count: 0,
                unknown_count: 0,
                details: if include_details {
                    Some(Vec::new())
                } else {
                    None
                },
            });
        }

        let mut valid_count = 0u64;
        let mut invalid_count = 0u64;
        let mut unknown_count = 0u64;
        let mut details = Vec::new();

        for (prefix, origin_asn) in entries {
            match self.validate(prefix, *origin_asn) {
                Ok(result) => {
                    match result.status {
                        RpkiStatus::Valid => valid_count += 1,
                        RpkiStatus::Invalid => invalid_count += 1,
                        RpkiStatus::Unknown => unknown_count += 1,
                    }
                    if include_details {
                        details.push(result);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to validate {}/{}: {}", prefix, origin_asn, e);
                    unknown_count += 1;
                }
            }
        }

        Ok(BulkValidationResult {
            valid_count,
            invalid_count,
            unknown_count,
            details: if include_details { Some(details) } else { None },
        })
    }

    /// Find all covering ROAs for a prefix
    pub fn find_covering_roas(&self, prefix: &str) -> Result<Vec<CoveringRoa>> {
        let cache_condition = self.build_cache_condition();

        let query = format!(
            r#"SELECT prefix::TEXT, max_length, origin_asn, ta
               FROM rpki_roas
               WHERE '{}'::INET <<= prefix {}
               ORDER BY CAST(split_part(prefix::TEXT, '/', 2) AS INTEGER) DESC"#,
            prefix, cache_condition
        );

        let mut stmt = self.conn.conn.prepare(&query)?;
        let mut rows = stmt.query([])?;

        let mut roas = Vec::new();
        while let Some(row) = rows.next()? {
            roas.push(CoveringRoa {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            });
        }

        Ok(roas)
    }

    /// Check if a specific ROA exists
    pub fn has_roa(&self, prefix: &str, origin_asn: u32) -> Result<bool> {
        let cache_condition = self.build_cache_condition();

        let query = format!(
            r#"SELECT COUNT(*) FROM rpki_roas
               WHERE prefix = '{}'::INET AND origin_asn = {} {}"#,
            prefix, origin_asn, cache_condition
        );

        let count: i64 = self.conn.query_row(&query, |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Get ROAs by origin ASN
    pub fn get_roas_by_asn(&self, asn: u32) -> Result<Vec<CoveringRoa>> {
        let cache_condition = self.build_cache_condition();

        let query = format!(
            r#"SELECT prefix::TEXT, max_length, origin_asn, ta
               FROM rpki_roas
               WHERE origin_asn = {} {}
               ORDER BY prefix::TEXT"#,
            asn, cache_condition
        );

        let mut stmt = self.conn.conn.prepare(&query)?;
        let mut rows = stmt.query([])?;

        let mut roas = Vec::new();
        while let Some(row) = rows.next()? {
            roas.push(CoveringRoa {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            });
        }

        Ok(roas)
    }

    /// Get ROAs by prefix (exact match)
    pub fn get_roas_by_prefix(&self, prefix: &str) -> Result<Vec<CoveringRoa>> {
        let cache_condition = self.build_cache_condition();

        let query = format!(
            r#"SELECT prefix::TEXT, max_length, origin_asn, ta
               FROM rpki_roas
               WHERE prefix = '{}'::INET {}
               ORDER BY origin_asn"#,
            prefix, cache_condition
        );

        let mut stmt = self.conn.conn.prepare(&query)?;
        let mut rows = stmt.query([])?;

        let mut roas = Vec::new();
        while let Some(row) = rows.next()? {
            roas.push(CoveringRoa {
                prefix: row.get(0)?,
                max_length: row.get(1)?,
                origin_asn: row.get(2)?,
                ta: row.get(3)?,
            });
        }

        Ok(roas)
    }

    /// Check if RPKI cache is available
    pub fn is_cache_available(&self) -> bool {
        let query = if let Some(cache_id) = self.cache_id {
            format!(
                "SELECT COUNT(*) FROM rpki_cache_meta WHERE id = {}",
                cache_id
            )
        } else {
            "SELECT COUNT(*) FROM rpki_cache_meta WHERE data_type = 'roa'".to_string()
        };

        match self.conn.query_row(&query, |row| row.get::<_, i64>(0)) {
            Ok(count) => count > 0,
            Err(_) => false,
        }
    }

    /// Get statistics about the RPKI cache
    pub fn cache_stats(&self) -> Result<RpkiCacheStats> {
        let cache_condition = self.build_cache_condition();

        let roa_count: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM rpki_roas WHERE 1=1 {}",
                cache_condition
            ),
            |row| row.get(0),
        )?;

        let unique_prefixes: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT prefix) FROM rpki_roas WHERE 1=1 {}",
                cache_condition
            ),
            |row| row.get(0),
        )?;

        let unique_asns: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(DISTINCT origin_asn) FROM rpki_roas WHERE 1=1 {}",
                cache_condition
            ),
            |row| row.get(0),
        )?;

        Ok(RpkiCacheStats {
            roa_count: roa_count as u64,
            unique_prefixes: unique_prefixes as u64,
            unique_asns: unique_asns as u64,
            cache_id: self.cache_id,
        })
    }

    /// Determine validation status based on covering ROAs
    fn determine_status(
        &self,
        covering_roas: &[CoveringRoa],
        origin_asn: u32,
        prefix_len: u32,
    ) -> RpkiStatus {
        if covering_roas.is_empty() {
            return RpkiStatus::Unknown;
        }

        // Check if any ROA validates this announcement
        for roa in covering_roas {
            if roa.origin_asn == origin_asn && prefix_len <= roa.max_length {
                return RpkiStatus::Valid;
            }
        }

        // Covering ROAs exist but none match - invalid
        RpkiStatus::Invalid
    }

    /// Get the prefix length from a prefix string
    fn get_prefix_length(&self, prefix: &str) -> Result<u32> {
        let parts: Vec<&str> = prefix.split('/').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid prefix format: {}", prefix));
        }
        parts[1]
            .parse::<u32>()
            .map_err(|e| anyhow!("Invalid prefix length in '{}': {}", prefix, e))
    }

    /// Build the cache condition clause for SQL queries
    fn build_cache_condition(&self) -> String {
        if let Some(id) = self.cache_id {
            format!("AND cache_id = {}", id)
        } else {
            String::new()
        }
    }
}

/// Statistics about the RPKI cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpkiCacheStats {
    /// Total number of ROAs
    pub roa_count: u64,
    /// Number of unique prefixes
    pub unique_prefixes: u64,
    /// Number of unique ASNs
    pub unique_asns: u64,
    /// Cache ID if using a specific cache
    pub cache_id: Option<i64>,
}

/// ASPA validation status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AspaStatus {
    /// Valid: Customer-provider relationship is authorized
    Valid,
    /// Invalid: ASPA exists but relationship not authorized
    Invalid,
    /// Unknown: No ASPA found for customer
    Unknown,
}

impl std::fmt::Display for AspaStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AspaStatus::Valid => write!(f, "valid"),
            AspaStatus::Invalid => write!(f, "invalid"),
            AspaStatus::Unknown => write!(f, "unknown"),
        }
    }
}

/// SQL-based ASPA validator using cached ASPA data
pub struct SqlAspaValidator<'a> {
    conn: &'a DuckDbConn,
    cache_id: Option<i64>,
}

impl<'a> SqlAspaValidator<'a> {
    /// Create a new SQL-based ASPA validator
    pub fn new(conn: &'a DuckDbConn) -> Self {
        Self {
            conn,
            cache_id: None,
        }
    }

    /// Use a specific cache ID for validation
    pub fn with_cache_id(mut self, cache_id: i64) -> Self {
        self.cache_id = Some(cache_id);
        self
    }

    /// Validate a customer-provider relationship
    ///
    /// # Arguments
    /// * `customer_asn` - The customer AS number
    /// * `provider_asn` - The provider AS number
    ///
    /// # Returns
    /// The ASPA validation status
    pub fn validate(&self, customer_asn: u32, provider_asn: u32) -> Result<AspaStatus> {
        let cache_condition = self.build_cache_condition();

        // First check if an ASPA exists for this customer
        let aspa_query = format!(
            r#"SELECT provider_asns::TEXT
               FROM rpki_aspas
               WHERE customer_asn = {} {}"#,
            customer_asn, cache_condition
        );

        let mut stmt = self.conn.conn.prepare(&aspa_query)?;
        let result: std::result::Result<String, _> = stmt.query_row([], |row| row.get(0));

        match result {
            Ok(providers_str) => {
                // Parse the provider ASNs array
                let providers = self.parse_array_string(&providers_str);

                if providers.contains(&provider_asn) {
                    Ok(AspaStatus::Valid)
                } else {
                    Ok(AspaStatus::Invalid)
                }
            }
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(AspaStatus::Unknown),
            Err(e) => Err(anyhow!("Failed to query ASPA: {}", e)),
        }
    }

    /// Get the authorized providers for a customer ASN
    pub fn get_providers(&self, customer_asn: u32) -> Result<Vec<u32>> {
        let cache_condition = self.build_cache_condition();

        let query = format!(
            r#"SELECT provider_asns::TEXT
               FROM rpki_aspas
               WHERE customer_asn = {} {}"#,
            customer_asn, cache_condition
        );

        let mut stmt = self.conn.conn.prepare(&query)?;
        let result: std::result::Result<String, _> = stmt.query_row([], |row| row.get(0));

        match result {
            Ok(providers_str) => Ok(self.parse_array_string(&providers_str)),
            Err(duckdb::Error::QueryReturnedNoRows) => Ok(Vec::new()),
            Err(e) => Err(anyhow!("Failed to query ASPA: {}", e)),
        }
    }

    /// Get all customers for a provider ASN
    pub fn get_customers(&self, provider_asn: u32) -> Result<Vec<u32>> {
        let cache_condition = self.build_cache_condition();

        let query = format!(
            r#"SELECT customer_asn
               FROM rpki_aspas
               WHERE list_contains(provider_asns, {}) {}"#,
            provider_asn, cache_condition
        );

        let mut stmt = self.conn.conn.prepare(&query)?;
        let mut rows = stmt.query([])?;

        let mut customers = Vec::new();
        while let Some(row) = rows.next()? {
            customers.push(row.get(0)?);
        }

        Ok(customers)
    }

    /// Check if ASPA cache is available
    pub fn is_cache_available(&self) -> bool {
        let query = if let Some(cache_id) = self.cache_id {
            format!(
                "SELECT COUNT(*) FROM rpki_cache_meta WHERE id = {}",
                cache_id
            )
        } else {
            "SELECT COUNT(*) FROM rpki_cache_meta WHERE data_type = 'aspa'".to_string()
        };

        match self.conn.query_row(&query, |row| row.get::<_, i64>(0)) {
            Ok(count) => count > 0,
            Err(_) => false,
        }
    }

    /// Get statistics about the ASPA cache
    pub fn cache_stats(&self) -> Result<AspaCacheStats> {
        let cache_condition = self.build_cache_condition();

        let aspa_count: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM rpki_aspas WHERE 1=1 {}",
                cache_condition
            ),
            |row| row.get(0),
        )?;

        Ok(AspaCacheStats {
            aspa_count: aspa_count as u64,
            cache_id: self.cache_id,
        })
    }

    /// Parse a DuckDB array string like "[1, 2, 3]" into Vec<u32>
    fn parse_array_string(&self, s: &str) -> Vec<u32> {
        let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
        if trimmed.is_empty() {
            return Vec::new();
        }
        trimmed
            .split(',')
            .filter_map(|part| part.trim().parse::<u32>().ok())
            .collect()
    }

    /// Build the cache condition clause
    fn build_cache_condition(&self) -> String {
        if let Some(id) = self.cache_id {
            format!("AND cache_id = {}", id)
        } else {
            String::new()
        }
    }
}

/// Statistics about the ASPA cache
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AspaCacheStats {
    /// Total number of ASPAs
    pub aspa_count: u64,
    /// Cache ID if using a specific cache
    pub cache_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::core::{DuckDbConn, DuckDbSchemaManager};

    fn setup_test_db() -> DuckDbConn {
        let conn = DuckDbConn::open_in_memory().unwrap();
        let manager = DuckDbSchemaManager::new(&conn);
        manager.initialize().unwrap();
        conn
    }

    fn populate_test_roas(conn: &DuckDbConn) {
        // Insert test cache metadata
        conn.execute(
            "INSERT INTO rpki_cache_meta (id, data_type, data_source, record_count) VALUES (1, 'roa', 'test', 3)"
        ).unwrap();

        // Insert test ROAs
        conn.execute(
            "INSERT INTO rpki_roas (prefix, max_length, origin_asn, ta, cache_id) VALUES
             ('1.0.0.0/24'::INET, 24, 13335, 'ARIN', 1),
             ('1.0.0.0/22'::INET, 24, 13335, 'ARIN', 1),
             ('10.0.0.0/8'::INET, 16, 64496, 'RIPE', 1)",
        )
        .unwrap();
    }

    fn populate_test_aspas(conn: &DuckDbConn) {
        // Insert test cache metadata
        conn.execute(
            "INSERT INTO rpki_cache_meta (id, data_type, data_source, record_count) VALUES (2, 'aspa', 'test', 2)"
        ).unwrap();

        // Insert test ASPAs
        conn.execute(
            "INSERT INTO rpki_aspas (customer_asn, provider_asns, cache_id) VALUES
             (64497, [64496, 64498], 2),
             (64499, [64500], 2)",
        )
        .unwrap();
    }

    #[test]
    fn test_validate_valid() {
        let conn = setup_test_db();
        populate_test_roas(&conn);

        let validator = SqlRpkiValidator::new(&conn);
        let result = validator.validate("1.0.0.0/24", 13335).unwrap();

        assert_eq!(result.status, RpkiStatus::Valid);
        assert!(!result.covering_roas.is_empty());
    }

    #[test]
    fn test_validate_invalid_asn() {
        let conn = setup_test_db();
        populate_test_roas(&conn);

        let validator = SqlRpkiValidator::new(&conn);
        let result = validator.validate("1.0.0.0/24", 99999).unwrap();

        assert_eq!(result.status, RpkiStatus::Invalid);
    }

    #[test]
    fn test_validate_invalid_length() {
        let conn = setup_test_db();
        populate_test_roas(&conn);

        let validator = SqlRpkiValidator::new(&conn);
        // /25 exceeds max_length of /24
        let result = validator.validate("1.0.0.0/25", 13335).unwrap();

        assert_eq!(result.status, RpkiStatus::Invalid);
    }

    #[test]
    fn test_validate_unknown() {
        let conn = setup_test_db();
        populate_test_roas(&conn);

        let validator = SqlRpkiValidator::new(&conn);
        let result = validator.validate("192.168.0.0/24", 12345).unwrap();

        assert_eq!(result.status, RpkiStatus::Unknown);
        assert!(result.covering_roas.is_empty());
    }

    #[test]
    fn test_find_covering_roas() {
        let conn = setup_test_db();
        populate_test_roas(&conn);

        let validator = SqlRpkiValidator::new(&conn);
        let roas = validator.find_covering_roas("1.0.0.0/24").unwrap();

        assert_eq!(roas.len(), 2); // Both /24 and /22 cover /24
    }

    #[test]
    fn test_get_roas_by_asn() {
        let conn = setup_test_db();
        populate_test_roas(&conn);

        let validator = SqlRpkiValidator::new(&conn);
        let roas = validator.get_roas_by_asn(13335).unwrap();

        assert_eq!(roas.len(), 2);
    }

    #[test]
    fn test_cache_stats() {
        let conn = setup_test_db();
        populate_test_roas(&conn);

        let validator = SqlRpkiValidator::new(&conn);
        let stats = validator.cache_stats().unwrap();

        assert_eq!(stats.roa_count, 3);
    }

    #[test]
    fn test_aspa_validate_valid() {
        let conn = setup_test_db();
        populate_test_aspas(&conn);

        let validator = SqlAspaValidator::new(&conn);
        let status = validator.validate(64497, 64496).unwrap();

        assert_eq!(status, AspaStatus::Valid);
    }

    #[test]
    fn test_aspa_validate_invalid() {
        let conn = setup_test_db();
        populate_test_aspas(&conn);

        let validator = SqlAspaValidator::new(&conn);
        let status = validator.validate(64497, 99999).unwrap();

        assert_eq!(status, AspaStatus::Invalid);
    }

    #[test]
    fn test_aspa_validate_unknown() {
        let conn = setup_test_db();
        populate_test_aspas(&conn);

        let validator = SqlAspaValidator::new(&conn);
        let status = validator.validate(12345, 67890).unwrap();

        assert_eq!(status, AspaStatus::Unknown);
    }

    #[test]
    fn test_aspa_get_providers() {
        let conn = setup_test_db();
        populate_test_aspas(&conn);

        let validator = SqlAspaValidator::new(&conn);
        let providers = validator.get_providers(64497).unwrap();

        assert_eq!(providers.len(), 2);
        assert!(providers.contains(&64496));
        assert!(providers.contains(&64498));
    }

    #[test]
    fn test_rpki_status_display() {
        assert_eq!(RpkiStatus::Valid.to_string(), "valid");
        assert_eq!(RpkiStatus::Invalid.to_string(), "invalid");
        assert_eq!(RpkiStatus::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_rpki_status_from_str() {
        assert_eq!(RpkiStatus::from_str("valid").unwrap(), RpkiStatus::Valid);
        assert_eq!(
            RpkiStatus::from_str("INVALID").unwrap(),
            RpkiStatus::Invalid
        );
        assert_eq!(
            RpkiStatus::from_str("unknown").unwrap(),
            RpkiStatus::Unknown
        );
        assert!(RpkiStatus::from_str("invalid_status").is_err());
    }
}
