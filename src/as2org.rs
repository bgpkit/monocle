use std::io::{BufRead, BufReader};
/// AS2Org data handling

use serde::{Serialize, Deserialize};
use anyhow::{anyhow, Result};
use flate2::read::GzDecoder;


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

pub fn parse_as2org_file(url: &str) -> Result<Vec<DataEntry>> {
    let mut res: Vec<DataEntry> = vec![];
    let req = reqwest::blocking::get(url)?;
    let reader = BufReader::new(GzDecoder::new(req));
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
    fn test_parsing_file() {
        let res = parse_as2org_file("https://publicdata.caida.org/datasets/as-organizations/20220701.as-org2info.jsonl.gz");
        if !res.is_ok() {
            dbg!(&res);
        }
        assert!(res.is_ok());
        assert_eq!(res.unwrap().len(), 201272);
    }
}