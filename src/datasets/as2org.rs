//! AS2Org data handling utility.
//!
//! Data source: bgpkit-commons asinfo module with as2org data from CAIDA.
//! The data is loaded from bgpkit-commons and cached in a local SQLite database.

use crate::database::MonocleDatabase;
use crate::CountryLookup;
use anyhow::{anyhow, Result};
use bgpkit_commons::BgpkitCommons;
use itertools::Itertools;
use rusqlite::Statement;
use std::collections::HashSet;
use tabled::Tabled;
use tracing::info;

pub struct As2org {
    db: MonocleDatabase,
    country_lookup: CountryLookup,
}

#[derive(Debug, Default)]
pub enum SearchType {
    AsnOnly,
    NameOnly,
    CountryOnly,
    #[default]
    Guess,
}

#[derive(Debug, Tabled)]
pub struct SearchResult {
    pub asn: u32,
    pub as_name: String,
    pub org_name: String,
    pub org_id: String,
    pub org_country: String,
    pub org_size: u32,
}

#[derive(Debug, Tabled)]
pub struct SearchResultConcise {
    pub asn: u32,
    pub as_name: String,
    pub org_name: String,
    pub org_country: String,
}

impl As2org {
    pub fn new(db_path: &Option<String>) -> Result<As2org> {
        let mut db = MonocleDatabase::new(db_path)?;
        As2org::initialize_db(&mut db)?;
        let country_lookup = CountryLookup::new();
        Ok(As2org { db, country_lookup })
    }

    fn stmt_to_results(
        &self,
        stmt: &mut Statement,
        full_country_name: bool,
    ) -> Result<Vec<SearchResult>> {
        let res_iter = stmt.query_map([], |row| {
            let code: String = row.get(4)?;
            let country: String = match full_country_name {
                true => {
                    let res = self.country_lookup.lookup_code(code.as_str());
                    match res {
                        None => code,
                        Some(c) => c.to_string(),
                    }
                }
                false => code,
            };
            Ok(SearchResult {
                asn: row.get(0)?,
                as_name: row.get(1)?,
                org_name: row.get(2)?,
                org_id: row.get(3)?,
                org_country: country,
                org_size: row.get(5)?,
            })
        })?;
        Ok(res_iter.filter_map(|x| x.ok()).collect())
    }

    pub fn is_db_empty(&self) -> bool {
        let count: u32 = self
            .db
            .conn
            .query_row("select count(*) from as2org_as", [], |row| row.get(0))
            .unwrap_or(0);
        count == 0
    }

    fn initialize_db(db: &mut MonocleDatabase) -> Result<()> {
        db.conn.execute(
            r#"
        create table if not exists as2org_as (
        asn INTEGER PRIMARY KEY,
        name TEXT,
        org_id TEXT,
        source TEXT
        );
        "#,
            [],
        )?;
        db.conn.execute(
            r#"
        create table if not exists as2org_org (
        org_id TEXT PRIMARY KEY,
        name TEXT,
        country TEXT,
        source TEXT
        );
        "#,
            [],
        )?;

        // Add indexes for better query performance
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_as_org_id ON as2org_as(org_id)",
            [],
        )?;
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_as_name ON as2org_as(name)",
            [],
        )?;
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_org_name ON as2org_org(name)",
            [],
        )?;
        db.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_org_country ON as2org_org(country)",
            [],
        )?;

        // Enable SQLite performance optimizations
        let _: String = db
            .conn
            .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        db.conn.execute("PRAGMA synchronous=NORMAL", [])?;
        db.conn.execute("PRAGMA cache_size=100000", [])?;
        db.conn.execute("PRAGMA temp_store=MEMORY", [])?;

        // views

        db.conn.execute(
            r#"
        create view if not exists as2org_both as
        select a.asn, a.name as 'as_name', b.name as 'org_name', b.org_id, b.country
        from as2org_as as a join as2org_org as b on a.org_id = b.org_id
        ;
        "#,
            [],
        )?;

        db.conn.execute(
            r#"
            create view if not exists as2org_count as
            select org_id, org_name, count(*) as count
            from as2org_both group by org_name
            order by count desc;
        "#,
            [],
        )?;

        db.conn.execute(
            r#"
            create view if not exists as2org_all as
            select a.*, b.count
            from as2org_both as a join as2org_count as b on a.org_id = b.org_id;
        "#,
            [],
        )?;
        Ok(())
    }

    pub fn clear_db(&self) -> Result<()> {
        self.db.conn.execute(
            r#"
        DELETE FROM as2org_as
        "#,
            [],
        )?;
        self.db.conn.execute(
            r#"
        DELETE FROM as2org_org
        "#,
            [],
        )?;
        Ok(())
    }

    /// Load as2org data from bgpkit-commons and insert into monocle sqlite database
    pub fn parse_insert_as2org(&self, _url: Option<&str>) -> Result<()> {
        self.clear_db()?;

        info!("loading AS info with as2org data from bgpkit-commons...");

        // Load AS info with as2org data from bgpkit-commons
        let mut commons = BgpkitCommons::new();
        commons
            .load_asinfo(true, false, false, false)
            .map_err(|e| anyhow!("Failed to load asinfo from bgpkit-commons: {}", e))?;

        let asinfo_map = commons
            .asinfo_all()
            .map_err(|e| anyhow!("Failed to get asinfo map: {}", e))?;

        info!(
            "loaded {} AS entries from bgpkit-commons, inserting to sqlite db now",
            asinfo_map.len()
        );

        // Use a transaction for all inserts
        let tx = self.db.conn.unchecked_transaction()?;

        // Track which org_ids we've already inserted to avoid duplicates
        let mut inserted_orgs: HashSet<String> = HashSet::new();

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
                            "bgpkit-commons", // source
                        ))?;
                        inserted_orgs.insert(as2org.org_id.clone());
                    }

                    // Insert AS entry
                    stmt_as.execute((
                        *asn,
                        info.name.as_str(),
                        as2org.org_id.as_str(),
                        "bgpkit-commons", // source
                    ))?;
                } else {
                    // AS without as2org data - create a synthetic org entry
                    let synthetic_org_id = format!("UNKNOWN-{}", asn);

                    if !inserted_orgs.contains(&synthetic_org_id) {
                        stmt_org.execute((
                            synthetic_org_id.as_str(),
                            info.name.as_str(),     // use AS name as org name
                            info.country.as_str(),  // use AS country
                            "bgpkit-commons-synth", // source
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
            }
        } // statements are dropped here

        tx.commit()?;
        info!(
            "as2org data loading finished: {} ASes, {} organizations",
            asinfo_map.len(),
            inserted_orgs.len()
        );
        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        search_type: &SearchType,
        full_country_name: bool,
    ) -> Result<Vec<SearchResult>> {
        #[allow(clippy::upper_case_acronyms)]
        enum QueryType {
            ASN,
            NAME,
            COUNTRY,
        }
        let res: Vec<SearchResult>;
        let mut query_type = QueryType::ASN;

        match search_type {
            SearchType::AsnOnly => {
                let asn = query.parse::<u32>()?;
                let mut stmt = self.db.conn.prepare(
                    format!(
                        "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where asn='{asn}'").as_str()
                )?;
                res = self.stmt_to_results(&mut stmt, full_country_name)?;
            }
            SearchType::NameOnly => {
                query_type = QueryType::NAME;
                let mut stmt = self.db.conn.prepare(
                    format!(
                        "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where org_name like '%{query}%' or as_name like '%{query}%' order by count desc").as_str()
                )?;
                res = self.stmt_to_results(&mut stmt, full_country_name)?;
            }
            SearchType::CountryOnly => {
                query_type = QueryType::COUNTRY;
                let countries = self.country_lookup.lookup(query);
                if countries.is_empty() {
                    return Err(anyhow!("no country found with the query ({})", query));
                } else if countries.len() > 1 {
                    let countries = countries.into_iter().map(|x| x.name).join(" ; ");
                    return Err(anyhow!(
                        "more than one countries found with the query ({query}): {countries}"
                    ));
                }

                let first_country = countries
                    .first()
                    .ok_or_else(|| anyhow!("No country found"))?;
                let mut stmt = self.db.conn.prepare(
                    format!(
                        "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where LOWER(country)='{}' order by count desc", first_country.code.to_lowercase()).as_str()
                )?;
                res = self.stmt_to_results(&mut stmt, full_country_name)?;
            }
            SearchType::Guess => match query.parse::<u32>() {
                Ok(asn) => {
                    query_type = QueryType::ASN;
                    let mut stmt = self.db.conn.prepare(
                            format!(
                                "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where asn='{asn}'").as_str()
                        )?;
                    res = self.stmt_to_results(&mut stmt, full_country_name)?;
                }
                Err(_) => {
                    query_type = QueryType::NAME;
                    let mut stmt = self.db.conn.prepare(
                            format!(
                                "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where org_name like '%{query}%' or as_name like '%{query}%' or org_id like '%{query}%' order by count desc").as_str()
                        )?;
                    res = self.stmt_to_results(&mut stmt, full_country_name)?;
                }
            },
        }

        match res.is_empty() {
            true => {
                let new_res = match query_type {
                    QueryType::ASN => SearchResult {
                        asn: query.parse::<u32>().unwrap_or(0),
                        as_name: "?".to_string(),
                        org_name: "?".to_string(),
                        org_id: "?".to_string(),
                        org_country: "?".to_string(),
                        org_size: 0,
                    },
                    QueryType::NAME => SearchResult {
                        asn: 0,
                        as_name: "?".to_string(),
                        org_name: query.to_string(),
                        org_id: "?".to_string(),
                        org_country: "?".to_string(),
                        org_size: 0,
                    },
                    QueryType::COUNTRY => SearchResult {
                        asn: 0,
                        as_name: "?".to_string(),
                        org_name: "?".to_string(),
                        org_id: "?".to_string(),
                        org_country: query.to_string(),
                        org_size: 0,
                    },
                };
                Ok(vec![new_res])
            }
            false => Ok(res),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creating_db() {
        let as2org = As2org::new(&Some("./test.sqlite3".to_string())).unwrap();
        as2org.clear_db().unwrap();
    }

    #[test]
    fn test_search_empty() {
        let as2org = As2org::new(&Some("./test.sqlite3".to_string())).unwrap();

        // Test that searching with empty DB returns "?" placeholder
        let res = as2org.search("12345", &SearchType::AsnOnly, false);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].as_name, "?");
    }
}
