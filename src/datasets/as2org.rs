//! AS2Org data handling utility.
//!
//! Data source:
//! The CAIDA AS Organizations Dataset,
//!      http://www.caida.org/data/as-organizations

use crate::database::MonocleDatabase;
use crate::CountryLookup;
use anyhow::{anyhow, Result};
use itertools::Itertools;
use regex::Regex;
use rusqlite::Statement;
use serde::{Deserialize, Serialize};
use tabled::Tabled;
use tracing::info;

/// Organization JSON format
///
/// --------------------
/// Organization fields
/// --------------------
/// org_id  : unique ID for the given organization
///            some will be created by the WHOIS entry and others will be
///            created by our scripts
/// changed : the changed date provided by its WHOIS entry
/// name    : name could be selected from the AUT entry tied to the
///            organization, the AUT entry with the largest customer cone,
///           listed for the organization (if there existed an stand alone
///            organization), or a human maintained file.
/// country : some WHOIS provide as a individual field. In other cases
///            we inferred it from the addresses
/// source  : the RIR or NIR database which was contained this entry
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonOrg {
    #[serde(alias = "organizationId")]
    org_id: String,

    changed: Option<String>,

    #[serde(default)]
    name: String,

    country: String,

    /// The RIR or NIR database that contained this entry
    source: String,

    #[serde(alias = "type")]
    data_type: String,
}

/// AS Json format
///
/// ----------
/// AS fields
/// ----------
/// asn     : the AS number
/// changed : the changed date provided by its WHOIS entry
/// name    : the name provide for the individual AS number
/// org_id  : maps to an organization entry
/// opaque_id   : opaque identifier used by RIR extended delegation format
/// source  : the RIR or NIR database which was contained this entry
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonAs {
    asn: String,

    changed: Option<String>,

    #[serde(default)]
    name: String,

    #[serde(alias = "opaqueId")]
    opaque_id: Option<String>,

    #[serde(alias = "organizationId")]
    org_id: String,

    /// The RIR or NIR database that contained this entry
    source: String,

    #[serde(rename = "type")]
    data_type: String,
}

#[derive(Debug)]
pub enum DataEntry {
    Org(JsonOrg),
    As(JsonAs),
}

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

    /// parse as2org data and insert into monocle sqlite database
    pub fn parse_insert_as2org(&self, url: Option<&str>) -> Result<()> {
        self.clear_db()?;
        let url = match url {
            Some(u) => u.to_string(),
            None => As2org::get_most_recent_data()?,
        };
        info!("start parsing as2org file at {}", url.as_str());
        let entries = As2org::parse_as2org_file(url.as_str())?;
        info!("parsing as2org file done. inserting to sqlite db now");

        // Use a transaction for all inserts
        let tx = self.db.conn.unchecked_transaction()?;

        {
            // Prepare statements for better performance
            let mut stmt_as = tx.prepare(
                "INSERT INTO as2org_as (asn, name, org_id, source) VALUES (?1, ?2, ?3, ?4)",
            )?;
            let mut stmt_org = tx.prepare(
                "INSERT INTO as2org_org (org_id, name, country, source) VALUES (?1, ?2, ?3, ?4)",
            )?;

            for entry in &entries {
                match entry {
                    DataEntry::Org(e) => {
                        stmt_org.execute((
                            e.org_id.as_str(),
                            e.name.as_str(),
                            e.country.as_str(),
                            e.source.as_str(),
                        ))?;
                    }
                    DataEntry::As(e) => {
                        let asn = e
                            .asn
                            .parse::<u32>()
                            .map_err(|_| anyhow!("Failed to parse ASN: {}", e.asn))?;
                        stmt_as.execute((
                            asn,
                            e.name.as_str(),
                            e.org_id.as_str(),
                            e.source.as_str(),
                        ))?;
                    }
                }
            }
        } // statements are dropped here

        tx.commit()?;
        info!("as2org data loading finished");
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

    /// parse remote AS2Org file into Vec of DataEntry
    pub fn parse_as2org_file(path: &str) -> Result<Vec<DataEntry>> {
        let mut res: Vec<DataEntry> = vec![];

        for line in oneio::read_lines(path)? {
            let line = line?;
            if line.contains(r#""type":"ASN""#) {
                let data = serde_json::from_str::<JsonAs>(line.as_str());
                match data {
                    Ok(data) => {
                        res.push(DataEntry::As(data));
                    }
                    Err(e) => {
                        eprintln!("error parsing line:\n{}", line.as_str());
                        return Err(anyhow!(e));
                    }
                }
            } else {
                let data = serde_json::from_str::<JsonOrg>(line.as_str());
                match data {
                    Ok(data) => {
                        res.push(DataEntry::Org(data));
                    }
                    Err(e) => {
                        eprintln!("error parsing line:\n{}", line.as_str());
                        return Err(anyhow!(e));
                    }
                }
            }
        }
        Ok(res)
    }

    pub fn get_most_recent_data() -> Result<String> {
        let data_link: Regex = Regex::new(r".*(........\.as-org2info\.jsonl\.gz).*")
            .map_err(|e| anyhow!("Failed to create regex: {}", e))?;
        let content = ureq::get("https://publicdata.caida.org/datasets/as-organizations/")
            .call()
            .map_err(|e| anyhow!("Failed to fetch data: {}", e))?
            .body_mut()
            .read_to_string()
            .map_err(|e| anyhow!("Failed to parse response: {}", e))?;
        let res: Vec<String> = data_link
            .captures_iter(content.as_str())
            .map(|cap| cap[1].to_owned())
            .collect();
        let file = res.last().ok_or_else(|| anyhow!("No data files found"))?;

        Ok(format!(
            "https://publicdata.caida.org/datasets/as-organizations/{file}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parsing_json_organization() {
        let test_str1 = r#"{"changed":"20121010","country":"US","name":"99MAIN NETWORK SERVICES","organizationId":"9NS-ARIN","source":"ARIN","type":"Organization"}
"#;
        let test_str2 = r#"{"country":"JP","name":"Nagasaki Cable Media Inc.","organizationId":"@aut-10000-JPNIC","source":"JPNIC","type":"Organization"}
"#;
        assert!(serde_json::from_str::<JsonOrg>(test_str1).is_ok());
        assert!(serde_json::from_str::<JsonOrg>(test_str2).is_ok());
    }
    #[test]
    fn test_parsing_json_as() {
        let test_str1 = r#"{"asn":"400644","changed":"20220418","name":"BGPKIT-LLC","opaqueId":"059b5fb85e8a50e0f722f235be7457a0_ARIN","organizationId":"BL-1057-ARIN","source":"ARIN","type":"ASN"}"#;
        assert!(serde_json::from_str::<JsonAs>(test_str1).is_ok());
    }

    #[test]
    fn test_creating_db() {
        let as2org = As2org::new(&Some("./test.sqlite3".to_string())).unwrap();
        // approximately one minute insert time
        let _res = as2org.parse_insert_as2org(Some("tests/test-as2org.jsonl.gz"));

        as2org.clear_db().unwrap();
    }

    #[test]
    fn test_search() {
        let as2org = As2org::new(&Some("./test.sqlite3".to_string())).unwrap();
        as2org.clear_db().unwrap();
        assert!(as2org.is_db_empty());
        as2org
            .parse_insert_as2org(Some("tests/test-as2org.jsonl.gz"))
            .unwrap();

        let res = as2org.search("400644", &SearchType::AsnOnly, false);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);

        let res = as2org.search("0", &SearchType::AsnOnly, false);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].as_name, "?");

        let res = as2org.search("bgpkit", &SearchType::NameOnly, false);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);

        let res = as2org.search("400644", &SearchType::Guess, false);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);

        let res = as2org.search("bgpkit", &SearchType::Guess, false);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);
        assert_eq!(data[0].as_name, "BGPKIT-LLC");
        assert_eq!(data[0].org_name, "BGPKIT LLC");
        assert_eq!(data[0].org_id, "BL-1057-ARIN");
        assert_eq!(data[0].org_country, "US");
        assert_eq!(data[0].org_size, 1);

        as2org.clear_db().unwrap();
    }

    #[test]
    fn test_crawling() {
        match As2org::get_most_recent_data() {
            Ok(data) => println!("{}", data),
            Err(e) => eprintln!("Error getting most recent data: {}", e),
        }
    }
}
