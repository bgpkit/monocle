//! AS2Org repository for the DuckDB database
//!
//! This module provides data access operations for AS-to-Organization mappings
//! using a denormalized schema optimized for DuckDB's columnar storage.
//! Data is sourced from CAIDA's AS2Org dataset via bgpkit-commons.

use anyhow::{anyhow, Result};
use duckdb::params;
use std::collections::HashMap;
use tracing::info;

use crate::database::core::DuckDbConn;

/// Repository for AS2Org data operations (DuckDB version)
///
/// Provides methods for querying and updating AS-to-Organization mappings
/// in the DuckDB database using a denormalized schema.
pub struct DuckDbAs2orgRepository<'a> {
    conn: &'a DuckDbConn,
}

/// Result of an AS2Org search query
#[derive(Debug, Clone)]
pub struct DuckDbAs2orgRecord {
    pub asn: u32,
    pub as_name: String,
    pub org_name: String,
    pub org_id: String,
    pub country: String,
}

impl<'a> DuckDbAs2orgRepository<'a> {
    /// Create a new AS2Org repository
    pub fn new(conn: &'a DuckDbConn) -> Self {
        Self { conn }
    }

    /// Check if the AS2Org data is empty
    pub fn is_empty(&self) -> bool {
        self.conn.table_count("as2org").unwrap_or(0) == 0
    }

    /// Get the count of AS entries
    pub fn count(&self) -> Result<u64> {
        self.conn.table_count("as2org")
    }

    /// Lookup organization name for a single ASN
    pub fn lookup_org_name(&self, asn: u32) -> Option<String> {
        let mut stmt = self
            .conn
            .conn
            .prepare("SELECT org_name FROM as2org WHERE asn = ? LIMIT 1")
            .ok()?;

        stmt.query_row(params![asn], |row| row.get(0)).ok()
    }

    /// Batch lookup of organization names for multiple ASNs
    pub fn lookup_org_names_batch(&self, asns: &[u32]) -> HashMap<u32, String> {
        let mut result = HashMap::new();

        if asns.is_empty() {
            return result;
        }

        // Build a query with IN clause for batch lookup
        let placeholders: Vec<String> = asns.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT asn, org_name FROM as2org WHERE asn IN ({})",
            placeholders.join(",")
        );

        if let Ok(mut stmt) = self.conn.conn.prepare(&query) {
            // Convert asns to params
            let params: Vec<&dyn duckdb::ToSql> =
                asns.iter().map(|a| a as &dyn duckdb::ToSql).collect();

            if let Ok(mut rows) = stmt.query(params.as_slice()) {
                while let Ok(Some(row)) = rows.next() {
                    if let (Ok(asn), Ok(name)) = (row.get::<_, u32>(0), row.get::<_, String>(1)) {
                        result.insert(asn, name);
                    }
                }
            }
        }

        result
    }

    /// Search by ASN
    pub fn search_by_asn(&self, asn: u32) -> Result<Vec<DuckDbAs2orgRecord>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT asn, as_name, org_name, org_id, country
             FROM as2org WHERE asn = ?",
        )?;

        let mut rows = stmt
            .query(params![asn])
            .map_err(|e| anyhow!("Failed to execute query: {}", e))?;

        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(DuckDbAs2orgRecord {
                asn: row.get(0)?,
                as_name: row.get(1)?,
                org_name: row.get(2)?,
                org_id: row.get(3)?,
                country: row.get(4)?,
            });
        }

        Ok(records)
    }

    /// Search by name (AS name or organization name)
    pub fn search_by_name(&self, query: &str) -> Result<Vec<DuckDbAs2orgRecord>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.conn.prepare(
            "SELECT asn, as_name, org_name, org_id, country
             FROM as2org
             WHERE org_name ILIKE ? OR as_name ILIKE ? OR org_id ILIKE ?
             ORDER BY asn",
        )?;

        let mut rows = stmt
            .query(params![pattern, pattern, pattern])
            .map_err(|e| anyhow!("Failed to execute query: {}", e))?;

        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(DuckDbAs2orgRecord {
                asn: row.get(0)?,
                as_name: row.get(1)?,
                org_name: row.get(2)?,
                org_id: row.get(3)?,
                country: row.get(4)?,
            });
        }

        Ok(records)
    }

    /// Search by country code
    pub fn search_by_country(&self, country_code: &str) -> Result<Vec<DuckDbAs2orgRecord>> {
        let mut stmt = self.conn.conn.prepare(
            "SELECT asn, as_name, org_name, org_id, country
             FROM as2org
             WHERE LOWER(country) = LOWER(?)
             ORDER BY asn",
        )?;

        let mut rows = stmt
            .query(params![country_code])
            .map_err(|e| anyhow!("Failed to execute query: {}", e))?;

        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(DuckDbAs2orgRecord {
                asn: row.get(0)?,
                as_name: row.get(1)?,
                org_name: row.get(2)?,
                org_id: row.get(3)?,
                country: row.get(4)?,
            });
        }

        Ok(records)
    }

    /// Clear all AS2Org data
    pub fn clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM as2org")
            .map_err(|e| anyhow!("Failed to clear as2org: {}", e))?;
        Ok(())
    }

    /// Load AS2Org data from bgpkit-commons
    ///
    /// This method clears existing data and loads fresh data from bgpkit-commons.
    /// Uses denormalized schema - each row contains full AS and org info.
    pub fn load_from_commons(&self) -> Result<usize> {
        use bgpkit_commons::BgpkitCommons;

        self.clear()?;

        info!("Loading AS info with as2org data from bgpkit-commons...");

        // Load AS info with as2org data from bgpkit-commons
        let mut commons = BgpkitCommons::new();
        commons
            .load_asinfo(true, false, false, false)
            .map_err(|e| anyhow!("Failed to load asinfo from bgpkit-commons: {}", e))?;

        let asinfo_map = commons
            .asinfo_all()
            .map_err(|e| anyhow!("Failed to get asinfo map: {}", e))?;

        info!(
            "Loaded {} AS entries from bgpkit-commons, inserting to DuckDB now",
            asinfo_map.len()
        );

        // Use a transaction for all inserts
        self.conn.transaction()?;

        let mut count = 0usize;

        {
            // Use prepared statement for better performance
            let mut stmt = self.conn.conn.prepare(
                "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )?;

            for (asn, info) in &asinfo_map {
                // Get organization info from as2org data if available
                let (org_id, org_name, country) = if let Some(as2org) = &info.as2org {
                    (
                        as2org.org_id.clone(),
                        as2org.org_name.clone(),
                        as2org.country.clone(),
                    )
                } else {
                    // AS without as2org data - create synthetic values
                    (
                        format!("UNKNOWN-{}", asn),
                        info.name.clone(),
                        info.country.clone(),
                    )
                };

                stmt.execute(params![
                    *asn,
                    info.name.as_str(),
                    org_id.as_str(),
                    org_name.as_str(),
                    country.as_str(),
                    "bgpkit-commons",
                ])?;

                count += 1;
            }
        }

        self.conn.commit()?;

        info!("AS2Org data loading finished: {} entries", count);

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::core::{DuckDbConn, DuckDbSchemaManager};

    fn setup_test_db() -> DuckDbConn {
        let conn = DuckDbConn::open_in_memory().unwrap();
        let schema = DuckDbSchemaManager::new(&conn);
        schema.initialize_core().unwrap();
        conn
    }

    #[test]
    fn test_is_empty() {
        let conn = setup_test_db();
        let repo = DuckDbAs2orgRepository::new(&conn);
        assert!(repo.is_empty());
    }

    #[test]
    fn test_insert_and_search() {
        let conn = setup_test_db();

        // Insert test data directly
        conn.execute(
            "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source)
             VALUES (65000, 'Test AS', 'TEST-ORG', 'Test Organization', 'US', 'test')",
        )
        .unwrap();

        let repo = DuckDbAs2orgRepository::new(&conn);

        // Test is_empty
        assert!(!repo.is_empty());

        // Test count
        assert_eq!(repo.count().unwrap(), 1);

        // Test search by ASN
        let results = repo.search_by_asn(65000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].asn, 65000);
        assert_eq!(results[0].as_name, "Test AS");
        assert_eq!(results[0].org_name, "Test Organization");

        // Test search by name
        let results = repo.search_by_name("Test").unwrap();
        assert_eq!(results.len(), 1);

        // Test search by country
        let results = repo.search_by_country("US").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_lookup_org_name() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source)
             VALUES (65000, 'Test AS', 'TEST-ORG', 'Test Organization', 'US', 'test')",
        )
        .unwrap();

        let repo = DuckDbAs2orgRepository::new(&conn);

        // Found
        let name = repo.lookup_org_name(65000);
        assert_eq!(name, Some("Test Organization".to_string()));

        // Not found
        let name = repo.lookup_org_name(99999);
        assert_eq!(name, None);
    }

    #[test]
    fn test_lookup_org_names_batch() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source) VALUES
             (65000, 'AS1', 'ORG1', 'Organization 1', 'US', 'test'),
             (65001, 'AS2', 'ORG2', 'Organization 2', 'UK', 'test'),
             (65002, 'AS3', 'ORG3', 'Organization 3', 'DE', 'test')",
        )
        .unwrap();

        let repo = DuckDbAs2orgRepository::new(&conn);

        let names = repo.lookup_org_names_batch(&[65000, 65001, 99999]);
        assert_eq!(names.len(), 2);
        assert_eq!(names.get(&65000), Some(&"Organization 1".to_string()));
        assert_eq!(names.get(&65001), Some(&"Organization 2".to_string()));
        assert_eq!(names.get(&99999), None);
    }

    #[test]
    fn test_clear() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source)
             VALUES (65000, 'Test AS', 'TEST-ORG', 'Test Organization', 'US', 'test')",
        )
        .unwrap();

        let repo = DuckDbAs2orgRepository::new(&conn);
        assert!(!repo.is_empty());

        repo.clear().unwrap();
        assert!(repo.is_empty());
    }

    #[test]
    fn test_case_insensitive_search() {
        let conn = setup_test_db();

        conn.execute(
            "INSERT INTO as2org (asn, as_name, org_id, org_name, country, source)
             VALUES (65000, 'CloudFlare Inc', 'CF-ORG', 'Cloudflare Inc.', 'US', 'test')",
        )
        .unwrap();

        let repo = DuckDbAs2orgRepository::new(&conn);

        // Search should be case-insensitive
        let results = repo.search_by_name("cloudflare").unwrap();
        assert_eq!(results.len(), 1);

        let results = repo.search_by_name("CLOUDFLARE").unwrap();
        assert_eq!(results.len(), 1);

        let results = repo.search_by_country("us").unwrap();
        assert_eq!(results.len(), 1);

        let results = repo.search_by_country("US").unwrap();
        assert_eq!(results.len(), 1);
    }
}
