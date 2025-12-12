//! DuckDB query helpers for prefix containment operations
//!
//! This module provides utilities for building SQL queries that leverage
//! DuckDB's native INET type operations for IP prefix matching.
//!
//! # Prefix Containment Operators
//!
//! DuckDB's inet extension supports the following containment operators:
//! - `<<=` : Prefix is contained by or equal to (sub-prefix query)
//! - `>>=` : Prefix contains or is equal to (super-prefix query)
//!
//! # Example
//!
//! ```rust,ignore
//! use monocle::database::core::duckdb_query::PrefixQueryBuilder;
//!
//! // Find all prefixes that are sub-prefixes of 10.0.0.0/8
//! let query = PrefixQueryBuilder::new("elems", "prefix")
//!     .include_sub("10.0.0.0/8")
//!     .build();
//!
//! // Find all prefixes that cover 10.1.1.0/24
//! let query = PrefixQueryBuilder::new("elems", "prefix")
//!     .include_super("10.1.1.0/24")
//!     .build();
//! ```

use anyhow::{anyhow, Result};
use ipnet::IpNet;
use std::str::FromStr;

/// Query builder for prefix containment operations
///
/// This builder helps construct SQL queries that use DuckDB's INET
/// containment operators for efficient prefix matching.
#[derive(Debug, Clone)]
pub struct PrefixQueryBuilder {
    /// The table name to query
    table: String,
    /// The column name containing the prefix (must be INET type)
    column: String,
    /// The target prefix for containment operations
    target_prefix: Option<String>,
    /// Whether to include sub-prefixes (more specific)
    include_sub: bool,
    /// Whether to include super-prefixes (less specific)
    include_super: bool,
    /// Whether to include exact match
    include_exact: bool,
    /// Additional WHERE clauses
    additional_conditions: Vec<String>,
    /// SELECT columns (defaults to *)
    select_columns: Option<String>,
    /// ORDER BY clause
    order_by: Option<String>,
    /// LIMIT clause
    limit: Option<u64>,
}

impl PrefixQueryBuilder {
    /// Create a new prefix query builder
    ///
    /// # Arguments
    /// * `table` - The table name to query
    /// * `column` - The column name containing the prefix (must be INET type)
    pub fn new(table: &str, column: &str) -> Self {
        Self {
            table: table.to_string(),
            column: column.to_string(),
            target_prefix: None,
            include_sub: false,
            include_super: false,
            include_exact: true,
            additional_conditions: Vec::new(),
            select_columns: None,
            order_by: None,
            limit: None,
        }
    }

    /// Set the target prefix for containment operations
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.target_prefix = Some(prefix.to_string());
        self
    }

    /// Include sub-prefixes (more specific) in the query
    ///
    /// Uses the `<<=` operator: prefix is contained by or equal to target
    pub fn include_sub(mut self, prefix: &str) -> Self {
        self.target_prefix = Some(prefix.to_string());
        self.include_sub = true;
        self
    }

    /// Include super-prefixes (less specific) in the query
    ///
    /// Uses the `>>=` operator: prefix contains or is equal to target
    pub fn include_super(mut self, prefix: &str) -> Self {
        self.target_prefix = Some(prefix.to_string());
        self.include_super = true;
        self
    }

    /// Include both sub and super prefixes
    pub fn include_sub_and_super(mut self, prefix: &str) -> Self {
        self.target_prefix = Some(prefix.to_string());
        self.include_sub = true;
        self.include_super = true;
        self
    }

    /// Set whether to include exact match (default: true)
    pub fn with_exact(mut self, include: bool) -> Self {
        self.include_exact = include;
        self
    }

    /// Add an additional WHERE condition
    pub fn with_condition(mut self, condition: &str) -> Self {
        self.additional_conditions.push(condition.to_string());
        self
    }

    /// Set the SELECT columns
    pub fn select(mut self, columns: &str) -> Self {
        self.select_columns = Some(columns.to_string());
        self
    }

    /// Set the ORDER BY clause
    pub fn order_by(mut self, order: &str) -> Self {
        self.order_by = Some(order.to_string());
        self
    }

    /// Set the LIMIT
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Build the SQL query string
    pub fn build(&self) -> String {
        let columns = self.select_columns.as_deref().unwrap_or("*");

        let mut conditions = Vec::new();

        // Build prefix containment conditions
        if let Some(prefix) = &self.target_prefix {
            let prefix_conditions = self.build_prefix_conditions(prefix);
            if !prefix_conditions.is_empty() {
                conditions.push(format!("({})", prefix_conditions));
            }
        }

        // Add additional conditions
        conditions.extend(self.additional_conditions.clone());

        // Build the full query
        let mut query = format!("SELECT {} FROM {}", columns, self.table);

        if !conditions.is_empty() {
            query.push_str(&format!(" WHERE {}", conditions.join(" AND ")));
        }

        if let Some(order) = &self.order_by {
            query.push_str(&format!(" ORDER BY {}", order));
        }

        if let Some(limit) = self.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        query
    }

    /// Build the prefix containment conditions
    fn build_prefix_conditions(&self, prefix: &str) -> String {
        let mut conditions = Vec::new();

        match (self.include_sub, self.include_super, self.include_exact) {
            // Sub-prefixes only (more specific, contained by target)
            (true, false, _) => {
                conditions.push(format!("{} <<= '{}'::INET", self.column, prefix));
            }
            // Super-prefixes only (less specific, containing target)
            (false, true, _) => {
                conditions.push(format!("{} >>= '{}'::INET", self.column, prefix));
            }
            // Both sub and super prefixes
            (true, true, _) => {
                conditions.push(format!(
                    "({} <<= '{}'::INET OR {} >>= '{}'::INET)",
                    self.column, prefix, self.column, prefix
                ));
            }
            // Exact match only
            (false, false, true) => {
                conditions.push(format!("{} = '{}'::INET", self.column, prefix));
            }
            // No conditions (shouldn't happen, but handle gracefully)
            (false, false, false) => {}
        }

        conditions.join(" OR ")
    }

    /// Validate that the prefix is a valid IP network
    pub fn validate_prefix(prefix: &str) -> Result<IpNet> {
        IpNet::from_str(prefix).map_err(|e| anyhow!("Invalid prefix '{}': {}", prefix, e))
    }
}

/// Build a prefix containment query clause
///
/// This is a convenience function for simple prefix containment queries.
///
/// # Arguments
/// * `column` - The column name containing the prefix (must be INET type)
/// * `prefix` - The target prefix
/// * `include_sub` - Whether to include sub-prefixes
/// * `include_super` - Whether to include super-prefixes
///
/// # Returns
/// A SQL WHERE clause fragment for prefix containment
pub fn build_prefix_containment_clause(
    column: &str,
    prefix: &str,
    include_sub: bool,
    include_super: bool,
) -> String {
    match (include_sub, include_super) {
        (true, true) => {
            format!(
                "({} <<= '{}'::INET OR {} >>= '{}'::INET)",
                column, prefix, column, prefix
            )
        }
        (true, false) => {
            format!("{} <<= '{}'::INET", column, prefix)
        }
        (false, true) => {
            format!("{} >>= '{}'::INET", column, prefix)
        }
        (false, false) => {
            format!("{} = '{}'::INET", column, prefix)
        }
    }
}

/// Order prefixes by specificity (prefix length)
///
/// Returns an ORDER BY clause that sorts by prefix length.
/// This uses string manipulation since DuckDB's inet extension
/// may not have a masklen() function.
///
/// # Arguments
/// * `column` - The column name containing the prefix
/// * `descending` - If true, more specific prefixes first
pub fn order_by_prefix_length(column: &str, descending: bool) -> String {
    let direction = if descending { "DESC" } else { "ASC" };
    // Extract prefix length from text representation: "10.0.0.0/8" -> "8"
    format!(
        "CAST(split_part({}::TEXT, '/', 2) AS INTEGER) {}",
        column, direction
    )
}

/// Query helper for RPKI validation using cached ROAs
///
/// This struct provides methods for building RPKI validation queries
/// that join BGP data with cached ROA data.
#[derive(Debug, Clone)]
pub struct RpkiValidationQuery {
    /// The BGP data table
    bgp_table: String,
    /// The prefix column in the BGP table
    prefix_column: String,
    /// The origin ASN column in the BGP table
    origin_column: String,
    /// Optional cache_id to use specific cache
    cache_id: Option<i64>,
}

impl RpkiValidationQuery {
    /// Create a new RPKI validation query builder
    pub fn new(bgp_table: &str, prefix_column: &str, origin_column: &str) -> Self {
        Self {
            bgp_table: bgp_table.to_string(),
            prefix_column: prefix_column.to_string(),
            origin_column: origin_column.to_string(),
            cache_id: None,
        }
    }

    /// Use a specific cache ID for validation
    pub fn with_cache_id(mut self, cache_id: i64) -> Self {
        self.cache_id = Some(cache_id);
        self
    }

    /// Build a query to find valid BGP entries (matching ROA exists)
    ///
    /// A BGP entry is valid if:
    /// - The prefix matches a ROA prefix (exactly or is a sub-prefix)
    /// - The origin ASN matches the ROA ASN
    /// - The prefix length is <= max_length
    pub fn build_valid_query(&self) -> String {
        let cache_condition = self.build_cache_condition();

        format!(
            r#"SELECT DISTINCT b.*
            FROM {} b
            INNER JOIN rpki_roas r ON
                b.{} <<= r.prefix AND
                b.{} = r.origin_asn AND
                CAST(split_part(b.{}::TEXT, '/', 2) AS INTEGER) <= r.max_length
                {}
            "#,
            self.bgp_table,
            self.prefix_column,
            self.origin_column,
            self.prefix_column,
            cache_condition
        )
    }

    /// Build a query to find invalid BGP entries
    ///
    /// A BGP entry is invalid if:
    /// - A covering ROA exists (prefix matches)
    /// - But either the ASN doesn't match OR the prefix length exceeds max_length
    pub fn build_invalid_query(&self) -> String {
        let cache_condition = self.build_cache_condition();

        format!(
            r#"SELECT DISTINCT b.*
            FROM {} b
            INNER JOIN rpki_roas r ON b.{} <<= r.prefix {}
            WHERE NOT EXISTS (
                SELECT 1 FROM rpki_roas r2
                WHERE b.{} <<= r2.prefix
                  AND b.{} = r2.origin_asn
                  AND CAST(split_part(b.{}::TEXT, '/', 2) AS INTEGER) <= r2.max_length
                  {}
            )
            "#,
            self.bgp_table,
            self.prefix_column,
            cache_condition,
            self.prefix_column,
            self.origin_column,
            self.prefix_column,
            cache_condition.replace("AND r.", "AND r2.")
        )
    }

    /// Build a query to find unknown BGP entries (no covering ROA)
    pub fn build_unknown_query(&self) -> String {
        let cache_condition = self.build_cache_condition();

        format!(
            r#"SELECT b.*
            FROM {} b
            WHERE NOT EXISTS (
                SELECT 1 FROM rpki_roas r
                WHERE b.{} <<= r.prefix {}
            )
            "#,
            self.bgp_table, self.prefix_column, cache_condition
        )
    }

    /// Build a query with RPKI validation status annotated
    ///
    /// Returns all BGP entries with an additional `rpki_status` column:
    /// - 'valid': Matching ROA found
    /// - 'invalid': Covering ROA exists but doesn't match
    /// - 'unknown': No covering ROA
    pub fn build_annotated_query(&self) -> String {
        let cache_condition = self.build_cache_condition();

        format!(
            r#"SELECT b.*,
                CASE
                    WHEN EXISTS (
                        SELECT 1 FROM rpki_roas r
                        WHERE b.{prefix} <<= r.prefix
                          AND b.{origin} = r.origin_asn
                          AND CAST(split_part(b.{prefix}::TEXT, '/', 2) AS INTEGER) <= r.max_length
                          {cache}
                    ) THEN 'valid'
                    WHEN EXISTS (
                        SELECT 1 FROM rpki_roas r
                        WHERE b.{prefix} <<= r.prefix {cache}
                    ) THEN 'invalid'
                    ELSE 'unknown'
                END AS rpki_status
            FROM {table} b
            "#,
            prefix = self.prefix_column,
            origin = self.origin_column,
            table = self.bgp_table,
            cache = cache_condition
        )
    }

    /// Build the cache condition clause
    fn build_cache_condition(&self) -> String {
        if let Some(id) = self.cache_id {
            format!("AND r.cache_id = {}", id)
        } else {
            String::new()
        }
    }
}

/// Query helper for Pfx2as lookups using cached data
#[derive(Debug, Clone)]
pub struct Pfx2asQuery {
    /// Optional cache_id to use specific cache
    cache_id: Option<i64>,
}

impl Pfx2asQuery {
    /// Create a new Pfx2as query builder
    pub fn new() -> Self {
        Self { cache_id: None }
    }

    /// Use a specific cache ID
    pub fn with_cache_id(mut self, cache_id: i64) -> Self {
        self.cache_id = Some(cache_id);
        self
    }

    /// Build a query to find the longest matching prefix for an IP/prefix
    pub fn build_longest_match_query(&self, prefix: &str) -> String {
        let cache_condition = self.build_cache_condition("p.");

        format!(
            r#"SELECT p.prefix::TEXT, p.origin_asns::TEXT
            FROM pfx2as p
            WHERE '{prefix}'::INET <<= p.prefix {cache}
            ORDER BY CAST(split_part(p.prefix::TEXT, '/', 2) AS INTEGER) DESC
            LIMIT 1
            "#,
            prefix = prefix,
            cache = cache_condition
        )
    }

    /// Build a query to find all prefixes covering a given prefix
    pub fn build_covering_query(&self, prefix: &str) -> String {
        let cache_condition = self.build_cache_condition("p.");

        format!(
            r#"SELECT p.prefix::TEXT, p.origin_asns::TEXT
            FROM pfx2as p
            WHERE '{prefix}'::INET <<= p.prefix {cache}
            ORDER BY CAST(split_part(p.prefix::TEXT, '/', 2) AS INTEGER) ASC
            "#,
            prefix = prefix,
            cache = cache_condition
        )
    }

    /// Build a query to find all prefixes covered by a given prefix
    pub fn build_covered_query(&self, prefix: &str) -> String {
        let cache_condition = self.build_cache_condition("p.");

        format!(
            r#"SELECT p.prefix::TEXT, p.origin_asns::TEXT
            FROM pfx2as p
            WHERE p.prefix <<= '{prefix}'::INET {cache}
            ORDER BY CAST(split_part(p.prefix::TEXT, '/', 2) AS INTEGER) ASC
            "#,
            prefix = prefix,
            cache = cache_condition
        )
    }

    /// Build a query to find prefixes by origin ASN
    pub fn build_by_origin_query(&self, asn: u32) -> String {
        let cache_condition = self.build_cache_condition("p.");

        format!(
            r#"SELECT p.prefix::TEXT, p.origin_asns::TEXT
            FROM pfx2as p
            WHERE list_contains(p.origin_asns, {asn}) {cache}
            ORDER BY p.prefix::TEXT
            "#,
            asn = asn,
            cache = cache_condition
        )
    }

    /// Build the cache condition clause
    fn build_cache_condition(&self, prefix: &str) -> String {
        if let Some(id) = self.cache_id {
            format!("AND {}cache_id = {}", prefix, id)
        } else {
            String::new()
        }
    }
}

impl Default for Pfx2asQuery {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_query_builder_sub() {
        let query = PrefixQueryBuilder::new("elems", "prefix")
            .include_sub("10.0.0.0/8")
            .build();

        assert!(query.contains("prefix <<= '10.0.0.0/8'::INET"));
    }

    #[test]
    fn test_prefix_query_builder_super() {
        let query = PrefixQueryBuilder::new("elems", "prefix")
            .include_super("10.1.1.0/24")
            .build();

        assert!(query.contains("prefix >>= '10.1.1.0/24'::INET"));
    }

    #[test]
    fn test_prefix_query_builder_both() {
        let query = PrefixQueryBuilder::new("elems", "prefix")
            .include_sub_and_super("10.0.0.0/16")
            .build();

        assert!(query.contains("<<="));
        assert!(query.contains(">>="));
    }

    #[test]
    fn test_prefix_query_builder_with_conditions() {
        let query = PrefixQueryBuilder::new("elems", "prefix")
            .include_sub("10.0.0.0/8")
            .with_condition("origin_asn = 13335")
            .with_condition("elem_type = 'A'")
            .select("prefix, origin_asn")
            .order_by("timestamp DESC")
            .limit(100)
            .build();

        assert!(query.contains("SELECT prefix, origin_asn"));
        assert!(query.contains("origin_asn = 13335"));
        assert!(query.contains("elem_type = 'A'"));
        assert!(query.contains("ORDER BY timestamp DESC"));
        assert!(query.contains("LIMIT 100"));
    }

    #[test]
    fn test_build_prefix_containment_clause() {
        let clause = build_prefix_containment_clause("prefix", "10.0.0.0/8", true, false);
        assert_eq!(clause, "prefix <<= '10.0.0.0/8'::INET");

        let clause = build_prefix_containment_clause("prefix", "10.0.0.0/8", false, true);
        assert_eq!(clause, "prefix >>= '10.0.0.0/8'::INET");

        let clause = build_prefix_containment_clause("prefix", "10.0.0.0/8", true, true);
        assert!(clause.contains("<<="));
        assert!(clause.contains(">>="));

        let clause = build_prefix_containment_clause("prefix", "10.0.0.0/8", false, false);
        assert_eq!(clause, "prefix = '10.0.0.0/8'::INET");
    }

    #[test]
    fn test_order_by_prefix_length() {
        let order = order_by_prefix_length("prefix", true);
        assert!(order.contains("DESC"));
        assert!(order.contains("split_part"));

        let order = order_by_prefix_length("prefix", false);
        assert!(order.contains("ASC"));
    }

    #[test]
    fn test_validate_prefix() {
        assert!(PrefixQueryBuilder::validate_prefix("10.0.0.0/8").is_ok());
        assert!(PrefixQueryBuilder::validate_prefix("192.168.1.0/24").is_ok());
        assert!(PrefixQueryBuilder::validate_prefix("2001:db8::/32").is_ok());
        assert!(PrefixQueryBuilder::validate_prefix("invalid").is_err());
        assert!(PrefixQueryBuilder::validate_prefix("10.0.0.0").is_err());
    }

    #[test]
    fn test_rpki_validation_query_valid() {
        let query = RpkiValidationQuery::new("elems", "prefix", "origin_asn").build_valid_query();

        assert!(query.contains("INNER JOIN rpki_roas"));
        assert!(query.contains("<<="));
        assert!(query.contains("origin_asn"));
        assert!(query.contains("max_length"));
    }

    #[test]
    fn test_rpki_validation_query_with_cache() {
        let query = RpkiValidationQuery::new("elems", "prefix", "origin_asn")
            .with_cache_id(42)
            .build_valid_query();

        assert!(query.contains("cache_id = 42"));
    }

    #[test]
    fn test_rpki_validation_query_annotated() {
        let query =
            RpkiValidationQuery::new("elems", "prefix", "origin_asn").build_annotated_query();

        assert!(query.contains("rpki_status"));
        assert!(query.contains("'valid'"));
        assert!(query.contains("'invalid'"));
        assert!(query.contains("'unknown'"));
    }

    #[test]
    fn test_pfx2as_query_longest_match() {
        let query = Pfx2asQuery::new().build_longest_match_query("10.1.1.0/24");

        assert!(query.contains("<<="));
        assert!(query.contains("ORDER BY"));
        assert!(query.contains("DESC"));
        assert!(query.contains("LIMIT 1"));
    }

    #[test]
    fn test_pfx2as_query_by_origin() {
        let query = Pfx2asQuery::new().build_by_origin_query(13335);

        assert!(query.contains("list_contains"));
        assert!(query.contains("13335"));
    }
}
