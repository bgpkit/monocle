//! SQL-based query builder for search filtering
//!
//! This module provides utilities for building SQL queries for the search lens
//! that leverage DuckDB's native INET type for efficient prefix matching.
//!
//! # Usage
//!
//! ```rust,ignore
//! use monocle::lens::search::SearchQueryBuilder;
//!
//! let query = SearchQueryBuilder::new()
//!     .with_prefix("10.0.0.0/8")
//!     .include_sub_prefixes()
//!     .with_origin_asn(13335)
//!     .with_elem_type("A")
//!     .build();
//! ```

use crate::database::core::{build_prefix_containment_clause, order_by_prefix_length};

/// Query builder for search operations on BGP elements
///
/// This builder constructs SQL queries that efficiently filter BGP elements
/// using DuckDB's native INET operations for prefix matching.
#[derive(Debug, Clone, Default)]
pub struct SearchQueryBuilder {
    /// Target prefix for filtering
    prefix: Option<String>,
    /// Include sub-prefixes (more specific)
    include_sub: bool,
    /// Include super-prefixes (less specific)
    include_super: bool,
    /// Origin ASN filter
    origin_asn: Option<u32>,
    /// Peer ASN filter
    peer_asn: Option<u32>,
    /// Peer IP filters
    peer_ips: Vec<String>,
    /// Element type filter (A for announcement, W for withdrawal)
    elem_type: Option<String>,
    /// AS path regex filter
    as_path_regex: Option<String>,
    /// Start timestamp filter
    start_ts: Option<i64>,
    /// End timestamp filter
    end_ts: Option<i64>,
    /// Collector filter
    collector: Option<String>,
    /// Additional WHERE conditions
    additional_conditions: Vec<String>,
    /// SELECT columns (defaults to *)
    select_columns: Option<String>,
    /// ORDER BY clause
    order_by: Option<String>,
    /// LIMIT clause
    limit: Option<u64>,
    /// OFFSET clause
    offset: Option<u64>,
}

impl SearchQueryBuilder {
    /// Create a new search query builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the target prefix for filtering
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        self.prefix = Some(prefix.to_string());
        self
    }

    /// Include sub-prefixes (more specific) in results
    pub fn include_sub_prefixes(mut self) -> Self {
        self.include_sub = true;
        self
    }

    /// Include super-prefixes (less specific) in results
    pub fn include_super_prefixes(mut self) -> Self {
        self.include_super = true;
        self
    }

    /// Include both sub and super prefixes
    pub fn include_all_related_prefixes(mut self) -> Self {
        self.include_sub = true;
        self.include_super = true;
        self
    }

    /// Filter by origin ASN
    pub fn with_origin_asn(mut self, asn: u32) -> Self {
        self.origin_asn = Some(asn);
        self
    }

    /// Filter by peer ASN
    pub fn with_peer_asn(mut self, asn: u32) -> Self {
        self.peer_asn = Some(asn);
        self
    }

    /// Add a peer IP filter
    pub fn with_peer_ip(mut self, ip: &str) -> Self {
        self.peer_ips.push(ip.to_string());
        self
    }

    /// Set multiple peer IP filters
    pub fn with_peer_ips(mut self, ips: &[String]) -> Self {
        self.peer_ips.extend(ips.iter().cloned());
        self
    }

    /// Filter by element type
    pub fn with_elem_type(mut self, elem_type: &str) -> Self {
        self.elem_type = Some(elem_type.to_string());
        self
    }

    /// Filter by AS path regex
    pub fn with_as_path_regex(mut self, regex: &str) -> Self {
        self.as_path_regex = Some(regex.to_string());
        self
    }

    /// Set start timestamp filter (Unix timestamp)
    pub fn with_start_ts(mut self, ts: i64) -> Self {
        self.start_ts = Some(ts);
        self
    }

    /// Set end timestamp filter (Unix timestamp)
    pub fn with_end_ts(mut self, ts: i64) -> Self {
        self.end_ts = Some(ts);
        self
    }

    /// Set time range filter
    pub fn with_time_range(mut self, start: i64, end: i64) -> Self {
        self.start_ts = Some(start);
        self.end_ts = Some(end);
        self
    }

    /// Filter by collector
    pub fn with_collector(mut self, collector: &str) -> Self {
        self.collector = Some(collector.to_string());
        self
    }

    /// Add a custom WHERE condition
    pub fn with_condition(mut self, condition: &str) -> Self {
        self.additional_conditions.push(condition.to_string());
        self
    }

    /// Set SELECT columns
    pub fn select(mut self, columns: &str) -> Self {
        self.select_columns = Some(columns.to_string());
        self
    }

    /// Set ORDER BY clause
    pub fn order_by(mut self, order: &str) -> Self {
        self.order_by = Some(order.to_string());
        self
    }

    /// Order by timestamp (most recent first)
    pub fn order_by_timestamp_desc(mut self) -> Self {
        self.order_by = Some("timestamp DESC".to_string());
        self
    }

    /// Order by timestamp (oldest first)
    pub fn order_by_timestamp_asc(mut self) -> Self {
        self.order_by = Some("timestamp ASC".to_string());
        self
    }

    /// Order by prefix specificity (most specific first)
    pub fn order_by_prefix_specificity_desc(mut self) -> Self {
        self.order_by = Some(order_by_prefix_length("prefix", true));
        self
    }

    /// Set LIMIT
    pub fn limit(mut self, limit: u64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set OFFSET
    pub fn offset(mut self, offset: u64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Build the SQL query
    pub fn build(&self) -> String {
        self.build_for_table("elems")
    }

    /// Build the SQL query for a specific table
    pub fn build_for_table(&self, table: &str) -> String {
        let columns = self.select_columns.as_deref().unwrap_or("*");
        let mut conditions = Vec::new();

        // Prefix containment filter
        if let Some(prefix) = &self.prefix {
            let prefix_clause = build_prefix_containment_clause(
                "prefix",
                prefix,
                self.include_sub,
                self.include_super,
            );
            conditions.push(prefix_clause);
        }

        // Origin ASN filter
        if let Some(asn) = self.origin_asn {
            conditions.push(format!("origin_asn = {}", asn));
        }

        // Peer ASN filter
        if let Some(asn) = self.peer_asn {
            conditions.push(format!("peer_asn = {}", asn));
        }

        // Peer IP filters
        if !self.peer_ips.is_empty() {
            let ip_conditions: Vec<String> = self
                .peer_ips
                .iter()
                .map(|ip| format!("peer_ip = '{}'::INET", ip))
                .collect();
            conditions.push(format!("({})", ip_conditions.join(" OR ")));
        }

        // Element type filter
        if let Some(elem_type) = &self.elem_type {
            let type_str = match elem_type.to_uppercase().as_str() {
                "A" | "ANNOUNCE" | "ANNOUNCEMENT" => "A",
                "W" | "WITHDRAW" | "WITHDRAWAL" => "W",
                _ => elem_type.as_str(),
            };
            conditions.push(format!("elem_type = '{}'", type_str));
        }

        // AS path regex filter
        if let Some(regex) = &self.as_path_regex {
            // DuckDB uses regexp_matches for regex matching
            conditions.push(format!("regexp_matches(as_path, '{}')", regex));
        }

        // Timestamp filters
        if let Some(ts) = self.start_ts {
            conditions.push(format!("timestamp >= to_timestamp({})", ts));
        }
        if let Some(ts) = self.end_ts {
            conditions.push(format!("timestamp <= to_timestamp({})", ts));
        }

        // Collector filter
        if let Some(collector) = &self.collector {
            conditions.push(format!("collector = '{}'", collector));
        }

        // Additional conditions
        conditions.extend(self.additional_conditions.clone());

        // Build the query
        let mut query = format!("SELECT {} FROM {}", columns, table);

        if !conditions.is_empty() {
            query.push_str(&format!(" WHERE {}", conditions.join(" AND ")));
        }

        if let Some(order) = &self.order_by {
            query.push_str(&format!(" ORDER BY {}", order));
        }

        if let Some(limit) = self.limit {
            query.push_str(&format!(" LIMIT {}", limit));
        }

        if let Some(offset) = self.offset {
            query.push_str(&format!(" OFFSET {}", offset));
        }

        query
    }

    /// Build a count query
    pub fn build_count(&self) -> String {
        self.build_count_for_table("elems")
    }

    /// Build a count query for a specific table
    pub fn build_count_for_table(&self, table: &str) -> String {
        let mut builder = self.clone();
        builder.select_columns = Some("COUNT(*)".to_string());
        builder.order_by = None;
        builder.limit = None;
        builder.offset = None;

        builder.build_for_table(table)
    }

    /// Build a query with RPKI validation status annotation
    ///
    /// This adds an `rpki_status` column to the results using a subquery
    /// to check against the RPKI cache.
    pub fn build_with_rpki_annotation(&self) -> String {
        self.build_with_rpki_annotation_for_table("elems")
    }

    /// Build a query with RPKI validation status annotation for a specific table
    pub fn build_with_rpki_annotation_for_table(&self, table: &str) -> String {
        let columns = self.select_columns.as_deref().unwrap_or("*");
        let base_query = self.build_for_table(table);

        // Wrap in a CTE to add RPKI status
        format!(
            r#"WITH base AS ({})
            SELECT b.*,
                CASE
                    WHEN EXISTS (
                        SELECT 1 FROM rpki_roas r
                        WHERE b.prefix <<= r.prefix
                          AND b.origin_asn = r.origin_asn
                          AND CAST(split_part(b.prefix::TEXT, '/', 2) AS INTEGER) <= r.max_length
                    ) THEN 'valid'
                    WHEN EXISTS (
                        SELECT 1 FROM rpki_roas r
                        WHERE b.prefix <<= r.prefix
                    ) THEN 'invalid'
                    ELSE 'unknown'
                END AS rpki_status
            FROM base b"#,
            base_query.replace("SELECT * FROM", &format!("SELECT {} FROM", columns))
        )
    }

    /// Build a query with Pfx2as origin annotation
    ///
    /// This adds `pfx2as_origins` column showing the expected origins for the prefix.
    pub fn build_with_pfx2as_annotation(&self) -> String {
        self.build_with_pfx2as_annotation_for_table("elems")
    }

    /// Build a query with Pfx2as origin annotation for a specific table
    pub fn build_with_pfx2as_annotation_for_table(&self, table: &str) -> String {
        let base_query = self.build_for_table(table);

        format!(
            r#"WITH base AS ({})
            SELECT b.*,
                (SELECT p.origin_asns::TEXT
                 FROM pfx2as p
                 WHERE b.prefix <<= p.prefix
                 ORDER BY CAST(split_part(p.prefix::TEXT, '/', 2) AS INTEGER) DESC
                 LIMIT 1) AS pfx2as_origins
            FROM base b"#,
            base_query
        )
    }
}

/// Build a simple prefix filter clause
///
/// This is a convenience function for building prefix filter clauses
/// without the full query builder.
pub fn build_prefix_filter(prefix: &str, include_sub: bool, include_super: bool) -> String {
    build_prefix_containment_clause("prefix", prefix, include_sub, include_super)
}

/// Filter specification for search operations
///
/// This struct mirrors the ParseFilters but is optimized for SQL generation.
#[derive(Debug, Clone, Default)]
pub struct SearchFilterSpec {
    pub prefix: Option<String>,
    pub include_sub: bool,
    pub include_super: bool,
    pub origin_asn: Option<u32>,
    pub peer_asn: Option<u32>,
    pub peer_ips: Vec<String>,
    pub elem_type: Option<String>,
    pub as_path_regex: Option<String>,
    pub start_ts: Option<i64>,
    pub end_ts: Option<i64>,
    pub collector: Option<String>,
}

impl SearchFilterSpec {
    /// Convert to a SearchQueryBuilder
    pub fn to_query_builder(&self) -> SearchQueryBuilder {
        let mut builder = SearchQueryBuilder::new();

        if let Some(prefix) = &self.prefix {
            builder = builder.with_prefix(prefix);
        }
        if self.include_sub {
            builder = builder.include_sub_prefixes();
        }
        if self.include_super {
            builder = builder.include_super_prefixes();
        }
        if let Some(asn) = self.origin_asn {
            builder = builder.with_origin_asn(asn);
        }
        if let Some(asn) = self.peer_asn {
            builder = builder.with_peer_asn(asn);
        }
        if !self.peer_ips.is_empty() {
            builder = builder.with_peer_ips(&self.peer_ips);
        }
        if let Some(elem_type) = &self.elem_type {
            builder = builder.with_elem_type(elem_type);
        }
        if let Some(regex) = &self.as_path_regex {
            builder = builder.with_as_path_regex(regex);
        }
        if let Some(ts) = self.start_ts {
            builder = builder.with_start_ts(ts);
        }
        if let Some(ts) = self.end_ts {
            builder = builder.with_end_ts(ts);
        }
        if let Some(collector) = &self.collector {
            builder = builder.with_collector(collector);
        }

        builder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_query() {
        let query = SearchQueryBuilder::new().build();
        assert_eq!(query, "SELECT * FROM elems");
    }

    #[test]
    fn test_prefix_exact_match() {
        let query = SearchQueryBuilder::new().with_prefix("10.0.0.0/8").build();

        assert!(query.contains("prefix = '10.0.0.0/8'::INET"));
    }

    #[test]
    fn test_prefix_sub_match() {
        let query = SearchQueryBuilder::new()
            .with_prefix("10.0.0.0/8")
            .include_sub_prefixes()
            .build();

        assert!(query.contains("prefix <<= '10.0.0.0/8'::INET"));
    }

    #[test]
    fn test_prefix_super_match() {
        let query = SearchQueryBuilder::new()
            .with_prefix("10.0.0.0/24")
            .include_super_prefixes()
            .build();

        assert!(query.contains("prefix >>= '10.0.0.0/24'::INET"));
    }

    #[test]
    fn test_prefix_both_match() {
        let query = SearchQueryBuilder::new()
            .with_prefix("10.0.0.0/16")
            .include_all_related_prefixes()
            .build();

        assert!(query.contains("<<="));
        assert!(query.contains(">>="));
    }

    #[test]
    fn test_origin_asn_filter() {
        let query = SearchQueryBuilder::new().with_origin_asn(13335).build();

        assert!(query.contains("origin_asn = 13335"));
    }

    #[test]
    fn test_peer_asn_filter() {
        let query = SearchQueryBuilder::new().with_peer_asn(64496).build();

        assert!(query.contains("peer_asn = 64496"));
    }

    #[test]
    fn test_peer_ip_filter() {
        let query = SearchQueryBuilder::new()
            .with_peer_ip("192.168.1.1")
            .build();

        assert!(query.contains("peer_ip = '192.168.1.1'::INET"));
    }

    #[test]
    fn test_multiple_peer_ips() {
        let query = SearchQueryBuilder::new()
            .with_peer_ips(&["192.168.1.1".to_string(), "10.0.0.1".to_string()])
            .build();

        assert!(query.contains("192.168.1.1"));
        assert!(query.contains("10.0.0.1"));
        assert!(query.contains(" OR "));
    }

    #[test]
    fn test_elem_type_filter() {
        let query = SearchQueryBuilder::new().with_elem_type("A").build();

        assert!(query.contains("elem_type = 'A'"));
    }

    #[test]
    fn test_as_path_regex() {
        let query = SearchQueryBuilder::new()
            .with_as_path_regex("^64496")
            .build();

        assert!(query.contains("regexp_matches"));
        assert!(query.contains("^64496"));
    }

    #[test]
    fn test_time_range() {
        let query = SearchQueryBuilder::new()
            .with_time_range(1000000, 2000000)
            .build();

        assert!(query.contains("timestamp >= to_timestamp(1000000)"));
        assert!(query.contains("timestamp <= to_timestamp(2000000)"));
    }

    #[test]
    fn test_collector_filter() {
        let query = SearchQueryBuilder::new().with_collector("rrc00").build();

        assert!(query.contains("collector = 'rrc00'"));
    }

    #[test]
    fn test_limit_offset() {
        let query = SearchQueryBuilder::new().limit(100).offset(50).build();

        assert!(query.contains("LIMIT 100"));
        assert!(query.contains("OFFSET 50"));
    }

    #[test]
    fn test_order_by() {
        let query = SearchQueryBuilder::new().order_by_timestamp_desc().build();

        assert!(query.contains("ORDER BY timestamp DESC"));
    }

    #[test]
    fn test_select_columns() {
        let query = SearchQueryBuilder::new()
            .select("prefix, origin_asn, timestamp")
            .build();

        assert!(query.contains("SELECT prefix, origin_asn, timestamp"));
    }

    #[test]
    fn test_combined_filters() {
        let query = SearchQueryBuilder::new()
            .with_prefix("10.0.0.0/8")
            .include_sub_prefixes()
            .with_origin_asn(13335)
            .with_elem_type("A")
            .order_by_timestamp_desc()
            .limit(100)
            .build();

        assert!(query.contains("<<="));
        assert!(query.contains("origin_asn = 13335"));
        assert!(query.contains("elem_type = 'A'"));
        assert!(query.contains("ORDER BY timestamp DESC"));
        assert!(query.contains("LIMIT 100"));
    }

    #[test]
    fn test_count_query() {
        let query = SearchQueryBuilder::new()
            .with_origin_asn(13335)
            .build_count();

        assert!(query.contains("SELECT COUNT(*)"));
        assert!(query.contains("origin_asn = 13335"));
        assert!(!query.contains("LIMIT"));
        assert!(!query.contains("ORDER BY"));
    }

    #[test]
    fn test_custom_condition() {
        let query = SearchQueryBuilder::new()
            .with_condition("communities LIKE '%13335:%'")
            .build();

        assert!(query.contains("communities LIKE '%13335:%'"));
    }

    #[test]
    fn test_filter_spec_conversion() {
        let spec = SearchFilterSpec {
            prefix: Some("10.0.0.0/8".to_string()),
            include_sub: true,
            include_super: false,
            origin_asn: Some(13335),
            peer_asn: None,
            peer_ips: vec!["192.168.1.1".to_string()],
            elem_type: Some("A".to_string()),
            as_path_regex: None,
            start_ts: Some(1000000),
            end_ts: None,
            collector: Some("rrc00".to_string()),
        };

        let query = spec.to_query_builder().build();

        assert!(query.contains("<<="));
        assert!(query.contains("origin_asn = 13335"));
        assert!(query.contains("192.168.1.1"));
        assert!(query.contains("elem_type = 'A'"));
        assert!(query.contains("collector = 'rrc00'"));
    }

    #[test]
    fn test_rpki_annotation() {
        let query = SearchQueryBuilder::new()
            .with_prefix("1.0.0.0/24")
            .build_with_rpki_annotation();

        assert!(query.contains("WITH base AS"));
        assert!(query.contains("rpki_status"));
        assert!(query.contains("'valid'"));
        assert!(query.contains("'invalid'"));
        assert!(query.contains("'unknown'"));
    }

    #[test]
    fn test_pfx2as_annotation() {
        let query = SearchQueryBuilder::new()
            .with_origin_asn(13335)
            .build_with_pfx2as_annotation();

        assert!(query.contains("WITH base AS"));
        assert!(query.contains("pfx2as_origins"));
        assert!(query.contains("pfx2as p"));
    }

    #[test]
    fn test_build_prefix_filter() {
        let filter = build_prefix_filter("10.0.0.0/8", true, false);
        assert_eq!(filter, "prefix <<= '10.0.0.0/8'::INET");

        let filter = build_prefix_filter("10.0.0.0/8", false, true);
        assert_eq!(filter, "prefix >>= '10.0.0.0/8'::INET");

        let filter = build_prefix_filter("10.0.0.0/8", false, false);
        assert_eq!(filter, "prefix = '10.0.0.0/8'::INET");
    }
}
