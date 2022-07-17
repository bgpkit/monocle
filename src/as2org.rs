use std::fs::File;
use std::io::{BufRead, BufReader, Read};
/// AS2Org data handling

use serde::{Serialize, Deserialize};
use anyhow::{anyhow, Result};
use flate2::read::GzDecoder;
use rusqlite::Statement;
use tabled::Tabled;
use crate::MonocleDatabase;


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
    #[serde(alias="organizationId")]
    org_id: String,

    changed: Option<String>,

    #[serde(default)]
    name: String,

    country: String,

    /// The RIR or NIR database that contained this entry
    source: String,

    #[serde(alias="type")]
    data_type: String
}

/// AS Json format
///
/// ----------
/// AS fields
/// ----------
/// aut     : the AS number
/// changed : the changed date provided by its WHOIS entry
/// aut_name    : the name provide for the individual AS number
/// org_id  : maps to an organization entry
/// opaque_id   : opaque identifier used by RIR extended delegation format
/// source  : the RIR or NIR database which was contained this entry
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonAs {

    asn: String,

    changed: Option<String>,

    #[serde(default)]
    name: String,

    #[serde(alias="opaqueId")]
    opaque_id: Option<String>,

    #[serde(alias="organizationId")]
    org_id: String,

    /// The RIR or NIR database that contained this entry
    source: String,

    #[serde(rename="type")]
    data_type: String
}

#[derive(Debug)]
pub enum DataEntry {
    Org(JsonOrg),
    As(JsonAs),
}

pub struct As2org {
    db: MonocleDatabase,
}

#[derive(Debug)]
pub enum SearchType {
    AsnOnly,
    NameOnly,
    Guess,
}

impl Default for SearchType {
    fn default() -> Self {
        SearchType::Guess
    }
}

#[derive(Debug, Tabled)]
pub struct SearchResult {
    asn: u32,
    as_name: String,
    org_name: String,
    org_id: String,
    org_country: String,
    org_size: u32
}

fn stmt_to_results(stmt: &mut Statement) -> Result<Vec<SearchResult>> {
    let res_iter = stmt.query_map([], |row| {
        Ok(SearchResult {
            asn: row.get(0)?,
            as_name: row.get(1)?,
            org_name: row.get(2)?,
            org_id: row.get(3)?,
            org_country: row.get(4)?,
            org_size: row.get(5)?
        })
    })?;
    Ok(
        res_iter.filter_map(|x| x.ok()).collect()
    )
}

impl As2org {

    pub fn new(db_path: &Option<String>) -> Result<As2org> {
        let mut db = MonocleDatabase::new(db_path)?;
        As2org::initialize_db(&mut db);
        Ok(As2org{ db })
    }

    pub fn is_db_empty(&self) -> bool {
        let count: u32 = self.db.conn.query_row("select count(*) from as2org_as", [],
            |row| row.get(0),
        ).unwrap();
        count == 0
    }

    fn initialize_db(db: &mut MonocleDatabase) {
        db.conn.execute(r#"
        create table if not exists as2org_as (
        asn INTEGER PRIMARY KEY,
        name TEXT,
        org_id TEXT,
        source TEXT
        );
        "#,[]).unwrap();
        db.conn.execute(r#"
        create table if not exists as2org_org (
        org_id TEXT PRIMARY KEY,
        name TEXT,
        country TEXT,
        source TEXT
        );
        "#,[]).unwrap();

        // views

        db.conn.execute(r#"
        create view if not exists as2org_both as
        select a.asn, a.name as 'as_name', b.name as 'org_name', b.org_id, b.country
        from as2org_as as a join as2org_org as b on a.org_id = b.org_id
        ;
        "#,[]).unwrap();

        db.conn.execute(r#"
            create view if not exists as2org_count as
            select org_id, org_name, count(*) as count
            from as2org_both group by org_name
            order by count desc;
        "#,[]).unwrap();

        db.conn.execute(r#"
            create view if not exists as2org_all as
            select a.*, b.count
            from as2org_both as a join as2org_count as b on a.org_id = b.org_id;
        "#,[]).unwrap();
    }

    fn insert_as(&self, as_entry: &JsonAs) -> Result<()> {
        self.db.conn.execute( r#"
        INSERT INTO as2org_as (asn, name, org_id, source)
        VALUES (?1, ?2, ?3, ?4)
        "#, (
            as_entry.asn.parse::<u32>().unwrap(),
            as_entry.name.as_str(),
            as_entry.org_id.as_str(),
            as_entry.source.as_str(),
        )
        )?;
        Ok(())
    }

    fn insert_org(&self, org_entry: &JsonOrg) -> Result<()> {
        self.db.conn.execute( r#"
        INSERT INTO as2org_org (org_id, name, country, source)
        VALUES (?1, ?2, ?3, ?4)
        "#, (
            org_entry.org_id.as_str(),
            org_entry.name.as_str(),
            org_entry.country.as_str(),
            org_entry.source.as_str(),
        )
        )?;
        Ok(())
    }

    fn clear_db(&self) {
        self.db.conn.execute(r#"
        DELETE FROM as2org_as
        "#, []
        ).unwrap();
        self.db.conn.execute(r#"
        DELETE FROM as2org_org
        "#, []
        ).unwrap();
    }

    pub fn parse_as2org(&self, url: &str) -> Result<()>{
        self.clear_db();
        let entries = As2org::parse_as2org_file(url)?;
        for entry in &entries {
            match entry {
                DataEntry::Org(e) => {
                    self.insert_org(e)?;
                }
                DataEntry::As(e) => {
                    self.insert_as(e)?;
                }
            }
        }
        Ok(())
    }

    pub fn search(&self, query: &str, search_type: &SearchType) -> Result<Vec<SearchResult>>{
        let res: Vec<SearchResult>;
        match search_type {
            SearchType::AsnOnly => {
                let asn = query.parse::<u32>()?;
                let mut stmt = self.db.conn.prepare(
                    format!(
                        "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where asn='{}'", asn).as_str()
                )?;
                res = stmt_to_results(&mut stmt)?;
            }
            SearchType::NameOnly => {
                let mut stmt = self.db.conn.prepare(
                    format!(
                        "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where org_name like '%{}%' or as_name like '%{}%' order by count desc", query, query).as_str()
                )?;
                res = stmt_to_results(&mut stmt)?;
            }
            SearchType::Guess => {
                match query.parse::<u32>() {
                    Ok(asn) => {
                        let mut stmt = self.db.conn.prepare(
                            format!(
                                "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where asn='{}'", asn).as_str()
                        )?;
                        res = stmt_to_results(&mut stmt)?;
                    }
                    Err(_) => {
                        let mut stmt = self.db.conn.prepare(
                            format!(
                                "SELECT asn, as_name, org_name, org_id, country, count FROM as2org_all where org_name like '%{}%' order by count desc", query).as_str()
                        )?;
                        res = stmt_to_results(&mut stmt)?;
                    }
                }
            }
        }
        Ok(res)
    }

    /// parse remote AS2Org file into Vec of DataEntry
    pub fn parse_as2org_file(path: &str) -> Result<Vec<DataEntry>> {
        let mut res: Vec<DataEntry> = vec![];

        let raw_reader: Box<dyn Read> = match path.starts_with("http") {
            true => {
                let response = reqwest::blocking::get(path)?;
                Box::new(response)
            }
            false => {
                Box::new(File::open(path)?)
            }
        };

        let reader = BufReader::new(GzDecoder::new(raw_reader));
        for line in reader.lines() {
            let line = line?;
            if line.contains(r#""type":"ASN""#) {
                let data = serde_json::from_str::<JsonAs>(line.as_str());
                match data {
                    Ok(data) => {
                        res.push(DataEntry::As(data));
                    }
                    Err(e) => {
                        eprintln!("error parsing line:\n{}", line.as_str());
                        return Err(anyhow!(e))
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
                        return Err(anyhow!(e))
                    }
                }
            }
        }
        Ok(res)
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
        let _res = as2org.parse_as2org("tests/test-as2org.jsonl.gz");

        as2org.clear_db();
    }

    #[test]
    fn test_search() {
        let as2org = As2org::new(&Some("./test.sqlite3".to_string())).unwrap();
        as2org.clear_db();
        assert_eq!(as2org.is_db_empty(), true);
        as2org.parse_as2org("tests/test-as2org.jsonl.gz").unwrap();

        let res = as2org.search("400644", &SearchType::AsnOnly);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);

        let res = as2org.search("0", &SearchType::AsnOnly);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 0);

        let res = as2org.search("bgpkit", &SearchType::NameOnly);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);

        let res = as2org.search("400644", &SearchType::Guess);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);

        let res = as2org.search("bgpkit", &SearchType::Guess);
        assert!(res.is_ok());
        let data = res.unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0].asn, 400644);
        assert_eq!(data[0].as_name, "BGPKIT-LLC");
        assert_eq!(data[0].org_name, "BGPKIT LLC");
        assert_eq!(data[0].org_id, "BL-1057-ARIN");
        assert_eq!(data[0].org_country, "US");
        assert_eq!(data[0].org_size, 1);

        as2org.clear_db();
    }
}