//! SQL-based query builder for search filtering
//!
//! This module provides utilities for building SQL queries for the search lens.
//! The queries are designed for SQLite and use simple string matching for prefixes.
//!
//! # Usage
//!
//! ```rust,ignore
//! use monocle::lens::search::SearchQueryBuilder;
//!
//! let query = SearchQueryBuilder::new()
//!     .with_prefix("10.0.0.0/8")
//!     .with_origin_asn(13335)
//!     .with_elem_type("A")
//!     .build();
//! ```

/// Query builder for search operations on BGP elements
///
/// This builder constructs SQL queries for filtering BGP elements
/// stored in SQLite databases.
#[derive(Debug, Clone, Default)]
pub struct SearchQueryBuilder {
    /// Target prefix for filtering (exact match)
    prefix: Option<String>,
    /// Include sub-prefixes (more specific) - uses LIKE prefix%
    include_sub: bool,
    /// Include super-prefixes (less specific) - not fully supported in SQLite
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
    /// SELECT columns
    select_columns: Option<String>,
    /// ORDER BY clause
    order_by: Option<String>,
    /// LIMIT
    limit: Option<usize>,
    /// OFFSET
    offset: Option<usize>,
}

impl SearchQueryBuilder {
    /// Create a new query builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by prefix (exact match by default)
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    /// Include sub-prefixes (more specific prefixes)
    pub fn include_sub_prefixes(mut self) -> Self {
        self.include_sub = true;
        self
    }

    /// Include super-prefixes (less specific prefixes)
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

    /// Filter by peer IP
    pub fn with_peer_ip(mut self, ip: impl Into<String>) -> Self {
        self.peer_ips.push(ip.into());
        self
    }

    /// Filter by multiple peer IPs
    pub fn with_peer_ips(mut self, ips: Vec<String>) -> Self {
        self.peer_ips.extend(ips);
        self
    }

    /// Filter by element type (A=announcement, W=withdrawal)
    pub fn with_elem_type(mut self, elem_type: impl Into<String>) -> Self {
        self.elem_type = Some(elem_type.into());
        self
    }

    /// Filter by AS path regex
    pub fn with_as_path_regex(mut self, regex: impl Into<String>) -> Self {
        self.as_path_regex = Some(regex.into());
        self
    }

    /// Filter by start timestamp
    pub fn with_start_ts(mut self, ts: i64) -> Self {
        self.start_ts = Some(ts);
        self
    }

    /// Filter by end timestamp
    pub fn with_end_ts(mut self, ts: i64) -> Self {
        self.end_ts = Some(ts);
        self
    }

    /// Filter by time range
    pub fn with_time_range(mut self, start: i64, end: i64) -> Self {
        self.start_ts = Some(start);
        self.end_ts = Some(end);
        self
    }

    /// Filter by collector
    pub fn with_collector(mut self, collector: impl Into<String>) -> Self {
        self.collector = Some(collector.into());
        self
    }

    /// Add a custom WHERE condition
    pub fn with_condition(mut self, condition: impl Into<String>) -> Self {
        self.additional_conditions.push(condition.into());
        self
    }

    /// Set SELECT columns
    pub fn select(mut self, columns: impl Into<String>) -> Self {
        self.select_columns = Some(columns.into());
        self
    }

    /// Set ORDER BY clause
    pub fn order_by(mut self, order: impl Into<String>) -> Self {
        self.order_by = Some(order.into());
        self
    }

    /// Order by timestamp descending
    pub fn order_by_timestamp_desc(mut self) -> Self {
        self.order_by = Some("timestamp DESC".to_string());
        self
    }

    /// Order by timestamp ascending
    pub fn order_by_timestamp_asc(mut self) -> Self {
        self.order_by = Some("timestamp ASC".to_string());
        self
    }

    /// Order by prefix (alphabetically)
    pub fn order_by_prefix(mut self) -> Self {
        self.order_by = Some("prefix".to_string());
        self
    }

    /// Set LIMIT
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set OFFSET
    pub fn offset(mut self, offset: usize) -> Self {
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

        // Prefix filter
        if let Some(prefix) = &self.prefix {
            let prefix_clause = build_prefix_filter(prefix, self.include_sub, self.include_super);
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
                .map(|ip| format!("peer_ip = '{}'", ip))
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

        // AS path regex filter (SQLite uses GLOB or LIKE, not full regex)
        if let Some(regex) = &self.as_path_regex {
            // Convert simple patterns to SQLite LIKE
            // This is a simplified version - full regex not supported in SQLite
            let pattern = regex.replace('*', "%").replace('?', "_");
            conditions.push(format!("as_path LIKE '%{}%'", pattern));
        }

        // Timestamp filters
        if let Some(ts) = self.start_ts {
            conditions.push(format!("timestamp >= {}", ts));
        }
        if let Some(ts) = self.end_ts {
            conditions.push(format!("timestamp <= {}", ts));
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
}

/// Build a simple prefix filter clause for SQLite
///
/// Note: SQLite doesn't support INET operations, so prefix matching
/// is done via string comparison which is not as accurate for CIDR
/// containment checks. For proper prefix containment, filtering should
/// be done during parsing, not in SQL.
pub fn build_prefix_filter(prefix: &str, include_sub: bool, include_super: bool) -> String {
    if include_sub && include_super {
        // For both sub and super, we do a broad match
        // This is approximate - proper containment requires INET ops
        let base = prefix.split('/').next().unwrap_or(prefix);
        format!("(prefix = '{}' OR prefix LIKE '{}%')", prefix, base)
    } else if include_sub {
        // Sub-prefixes: match prefix or anything that starts with it
        let base = prefix.split('/').next().unwrap_or(prefix);
        format!("(prefix = '{}' OR prefix LIKE '{}%')", prefix, base)
    } else if include_super {
        // Super-prefixes: harder to do in SQLite without INET
        // Just do exact match as approximation
        format!("prefix = '{}'", prefix)
    } else {
        // Exact match
        format!("prefix = '{}'", prefix)
    }
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
            builder = builder.with_prefix(prefix.clone());
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
            builder = builder.with_peer_ips(self.peer_ips.clone());
        }
        if let Some(elem_type) = &self.elem_type {
            builder = builder.with_elem_type(elem_type.clone());
        }
        if let Some(regex) = &self.as_path_regex {
            builder = builder.with_as_path_regex(regex.clone());
        }
        if let Some(ts) = self.start_ts {
            builder = builder.with_start_ts(ts);
        }
        if let Some(ts) = self.end_ts {
            builder = builder.with_end_ts(ts);
        }
        if let Some(collector) = &self.collector {
            builder = builder.with_collector(collector.clone());
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
        assert!(query.contains("prefix = '10.0.0.0/8'"));
    }

    #[test]
    fn test_prefix_sub_match() {
        let query = SearchQueryBuilder::new()
            .with_prefix("10.0.0.0/8")
            .include_sub_prefixes()
            .build();
        assert!(query.contains("prefix = '10.0.0.0/8'"));
        assert!(query.contains("prefix LIKE '10.0.0.0%'"));
    }

    #[test]
    fn test_origin_asn_filter() {
        let query = SearchQueryBuilder::new().with_origin_asn(13335).build();
        assert!(query.contains("origin_asn = 13335"));
    }

    #[test]
    fn test_peer_asn_filter() {
        let query = SearchQueryBuilder::new().with_peer_asn(65000).build();
        assert!(query.contains("peer_asn = 65000"));
    }

    #[test]
    fn test_peer_ip_filter() {
        let query = SearchQueryBuilder::new()
            .with_peer_ip("192.168.1.1")
            .build();
        assert!(query.contains("peer_ip = '192.168.1.1'"));
    }

    #[test]
    fn test_multiple_peer_ips() {
        let query = SearchQueryBuilder::new()
            .with_peer_ip("192.168.1.1")
            .with_peer_ip("10.0.0.1")
            .build();
        assert!(query.contains("peer_ip = '192.168.1.1'"));
        assert!(query.contains("peer_ip = '10.0.0.1'"));
        assert!(query.contains(" OR "));
    }

    #[test]
    fn test_elem_type_filter() {
        let query = SearchQueryBuilder::new().with_elem_type("A").build();
        assert!(query.contains("elem_type = 'A'"));
    }

    #[test]
    fn test_time_range() {
        let query = SearchQueryBuilder::new()
            .with_start_ts(1000)
            .with_end_ts(2000)
            .build();
        assert!(query.contains("timestamp >= 1000"));
        assert!(query.contains("timestamp <= 2000"));
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
            .select("prefix, origin_asn")
            .build();
        assert!(query.contains("SELECT prefix, origin_asn FROM"));
    }

    #[test]
    fn test_combined_filters() {
        let query = SearchQueryBuilder::new()
            .with_prefix("10.0.0.0/8")
            .with_origin_asn(13335)
            .with_elem_type("A")
            .order_by_timestamp_desc()
            .limit(100)
            .build();

        assert!(query.contains("prefix = '10.0.0.0/8'"));
        assert!(query.contains("origin_asn = 13335"));
        assert!(query.contains("elem_type = 'A'"));
        assert!(query.contains("ORDER BY timestamp DESC"));
        assert!(query.contains("LIMIT 100"));
    }

    #[test]
    fn test_count_query() {
        let query = SearchQueryBuilder::new()
            .with_origin_asn(13335)
            .limit(100) // Should be ignored in count
            .build_count();

        assert!(query.contains("SELECT COUNT(*)"));
        assert!(query.contains("origin_asn = 13335"));
        assert!(!query.contains("LIMIT"));
    }

    #[test]
    fn test_custom_condition() {
        let query = SearchQueryBuilder::new()
            .with_condition("custom_field > 100")
            .build();
        assert!(query.contains("custom_field > 100"));
    }

    #[test]
    fn test_filter_spec_conversion() {
        let spec = SearchFilterSpec {
            prefix: Some("10.0.0.0/8".to_string()),
            include_sub: true,
            origin_asn: Some(13335),
            ..Default::default()
        };

        let query = spec.to_query_builder().build();
        assert!(query.contains("10.0.0.0"));
        assert!(query.contains("origin_asn = 13335"));
    }

    #[test]
    fn test_build_prefix_filter() {
        let filter = build_prefix_filter("10.0.0.0/8", false, false);
        assert_eq!(filter, "prefix = '10.0.0.0/8'");

        let filter = build_prefix_filter("10.0.0.0/8", true, false);
        assert!(filter.contains("LIKE"));
    }
}
