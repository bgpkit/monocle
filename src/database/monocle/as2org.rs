//! AS2Org repository for the shared database
//!
//! This module provides data access operations for AS-to-Organization mappings.
//! Data is sourced from CAIDA's AS2Org dataset via bgpkit-commons.

use anyhow::{anyhow, Result};
use rusqlite::{Connection, Statement};
use std::collections::{HashMap, HashSet};
use tracing::info;

/// Repository for AS2Org data operations
///
/// Provides methods for querying and updating AS-to-Organization mappings
/// in the shared database.
pub struct As2orgRepository<'a> {
    conn: &'a Connection,
}

/// Result of an AS2Org search query
#[derive(Debug, Clone)]
pub struct As2orgRecord {
    pub asn: u32,
    pub as_name: String,
    pub org_name: String,
    pub org_id: String,
    pub country: String,
    pub org_size: u32,
}

impl<'a> As2orgRepository<'a> {
    /// Create a new AS2Org repository
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Check if the AS2Org data is empty
    pub fn is_empty(&self) -> bool {
        let count: u32 = self
            .conn
            .query_row("SELECT COUNT(*) FROM as2org_as", [], |row| row.get(0))
            .unwrap_or(0);
        count == 0
    }

    /// Get the count of AS entries
    pub fn as_count(&self) -> Result<u64> {
        let count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM as2org_as", [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to get AS count: {}", e))?;
        Ok(count)
    }

    /// Get the count of organization entries
    pub fn org_count(&self) -> Result<u64> {
        let count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM as2org_org", [], |row| row.get(0))
            .map_err(|e| anyhow!("Failed to get org count: {}", e))?;
        Ok(count)
    }

    /// Lookup organization name for a single ASN
    pub fn lookup_org_name(&self, asn: u32) -> Option<String> {
        self.conn
            .query_row(
                "SELECT org_name FROM as2org_all WHERE asn = ?1 LIMIT 1",
                [asn],
                |row| row.get(0),
            )
            .ok()
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
            "SELECT asn, org_name FROM as2org_all WHERE asn IN ({})",
            placeholders.join(",")
        );

        if let Ok(mut stmt) = self.conn.prepare(&query) {
            let params: Vec<&dyn rusqlite::ToSql> =
                asns.iter().map(|a| a as &dyn rusqlite::ToSql).collect();

            if let Ok(rows) = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            }) {
                for row in rows.flatten() {
                    result.insert(row.0, row.1);
                }
            }
        }

        result
    }

    /// Search by ASN
    pub fn search_by_asn(&self, asn: u32) -> Result<Vec<As2orgRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT asn, as_name, org_name, org_id, country, count
             FROM as2org_all WHERE asn = ?1",
        )?;

        self.stmt_to_records(&mut stmt, rusqlite::params![asn])
    }

    /// Search by name (AS name or organization name)
    pub fn search_by_name(&self, query: &str) -> Result<Vec<As2orgRecord>> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            "SELECT asn, as_name, org_name, org_id, country, count
             FROM as2org_all
             WHERE org_name LIKE ?1 OR as_name LIKE ?1 OR org_id LIKE ?1
             ORDER BY count DESC",
        )?;

        self.stmt_to_records(&mut stmt, rusqlite::params![pattern])
    }

    /// Search by country code
    pub fn search_by_country(&self, country_code: &str) -> Result<Vec<As2orgRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT asn, as_name, org_name, org_id, country, count
             FROM as2org_all
             WHERE LOWER(country) = LOWER(?1)
             ORDER BY count DESC",
        )?;

        self.stmt_to_records(&mut stmt, rusqlite::params![country_code])
    }

    /// Convert a prepared statement to records
    fn stmt_to_records<P: rusqlite::Params>(
        &self,
        stmt: &mut Statement,
        params: P,
    ) -> Result<Vec<As2orgRecord>> {
        let rows = stmt
            .query_map(params, |row| {
                Ok(As2orgRecord {
                    asn: row.get(0)?,
                    as_name: row.get(1)?,
                    org_name: row.get(2)?,
                    org_id: row.get(3)?,
                    country: row.get(4)?,
                    org_size: row.get(5)?,
                })
            })
            .map_err(|e| anyhow!("Failed to execute query: {}", e))?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Clear all AS2Org data
    pub fn clear(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM as2org_as", [])
            .map_err(|e| anyhow!("Failed to clear as2org_as: {}", e))?;
        self.conn
            .execute("DELETE FROM as2org_org", [])
            .map_err(|e| anyhow!("Failed to clear as2org_org: {}", e))?;
        Ok(())
    }

    /// Load AS2Org data from bgpkit-commons
    ///
    /// This method clears existing data and loads fresh data from bgpkit-commons.
    pub fn load_from_commons(&self) -> Result<(usize, usize)> {
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
            "Loaded {} AS entries from bgpkit-commons, inserting to sqlite db now",
            asinfo_map.len()
        );

        // Use a transaction for all inserts
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| anyhow!("Failed to begin transaction: {}", e))?;

        // Track which org_ids we've already inserted to avoid duplicates
        let mut inserted_orgs: HashSet<String> = HashSet::new();
        let mut as_count = 0usize;

        {
            // Prepare statements for better performance
            let mut stmt_as = tx.prepare(
                "INSERT OR REPLACE INTO as2org_as (asn, name, org_id, source) VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut stmt_org = tx.prepare(
                "INSERT OR REPLACE INTO as2org_org (org_id, name, country, source) VALUES (?1, ?2, ?3, ?4)",
            )?;

            for (asn, info) in &asinfo_map {
                // Get organization info from as2org data if available
                if let Some(as2org) = &info.as2org {
                    // Insert organization if not already inserted
                    if !inserted_orgs.contains(&as2org.org_id) {
                        stmt_org.execute((
                            as2org.org_id.as_str(),
                            as2org.org_name.as_str(),
                            as2org.country.as_str(),
                            "bgpkit-commons",
                        ))?;
                        inserted_orgs.insert(as2org.org_id.clone());
                    }

                    // Insert AS entry
                    stmt_as.execute((
                        *asn,
                        info.name.as_str(),
                        as2org.org_id.as_str(),
                        "bgpkit-commons",
                    ))?;
                } else {
                    // AS without as2org data - create a synthetic org entry
                    let synthetic_org_id = format!("UNKNOWN-{}", asn);

                    if !inserted_orgs.contains(&synthetic_org_id) {
                        stmt_org.execute((
                            synthetic_org_id.as_str(),
                            info.name.as_str(),
                            info.country.as_str(),
                            "bgpkit-commons-synth",
                        ))?;
                        inserted_orgs.insert(synthetic_org_id.clone());
                    }

                    stmt_as.execute((
                        *asn,
                        info.name.as_str(),
                        synthetic_org_id.as_str(),
                        "bgpkit-commons",
                    ))?;
                }
                as_count += 1;
            }
        }

        tx.commit()
            .map_err(|e| anyhow!("Failed to commit transaction: {}", e))?;

        let org_count = inserted_orgs.len();
        info!(
            "AS2Org data loading finished: {} ASes, {} organizations",
            as_count, org_count
        );

        Ok((as_count, org_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::core::{DatabaseConn, SchemaManager};

    fn setup_test_db() -> DatabaseConn {
        let db = DatabaseConn::open_in_memory().unwrap();
        let schema = SchemaManager::new(&db.conn);
        schema.initialize().unwrap();
        db
    }

    #[test]
    fn test_is_empty() {
        let db = setup_test_db();
        let repo = As2orgRepository::new(&db.conn);
        assert!(repo.is_empty());
    }

    #[test]
    fn test_insert_and_search() {
        let db = setup_test_db();

        // Insert test data directly
        db.conn
            .execute(
                "INSERT INTO as2org_org (org_id, name, country, source) VALUES ('TEST-ORG', 'Test Organization', 'US', 'test')",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO as2org_as (asn, name, org_id, source) VALUES (65000, 'Test AS', 'TEST-ORG', 'test')",
                [],
            )
            .unwrap();

        let repo = As2orgRepository::new(&db.conn);

        // Test is_empty
        assert!(!repo.is_empty());

        // Test counts
        assert_eq!(repo.as_count().unwrap(), 1);
        assert_eq!(repo.org_count().unwrap(), 1);

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
        let db = setup_test_db();

        db.conn
            .execute(
                "INSERT INTO as2org_org (org_id, name, country, source) VALUES ('TEST-ORG', 'Test Organization', 'US', 'test')",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO as2org_as (asn, name, org_id, source) VALUES (65000, 'Test AS', 'TEST-ORG', 'test')",
                [],
            )
            .unwrap();

        let repo = As2orgRepository::new(&db.conn);

        // Found
        let name = repo.lookup_org_name(65000);
        assert_eq!(name, Some("Test Organization".to_string()));

        // Not found
        let name = repo.lookup_org_name(99999);
        assert_eq!(name, None);
    }

    #[test]
    fn test_clear() {
        let db = setup_test_db();

        db.conn
            .execute(
                "INSERT INTO as2org_org (org_id, name, country, source) VALUES ('TEST-ORG', 'Test Org', 'US', 'test')",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO as2org_as (asn, name, org_id, source) VALUES (65000, 'Test AS', 'TEST-ORG', 'test')",
                [],
            )
            .unwrap();

        let repo = As2orgRepository::new(&db.conn);
        assert!(!repo.is_empty());

        repo.clear().unwrap();
        assert!(repo.is_empty());
    }
}
