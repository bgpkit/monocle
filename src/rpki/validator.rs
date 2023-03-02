use anyhow::Result;
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use tabled::Tabled;

#[derive(Debug, Tabled)]
pub struct RpkiValidity {
    asn: u32,
    prefix: IpNetwork,
    validity: ValidationState,
}

const CLOUDFLARE_RPKI_GRAPHQL: &str = "https://rpki.cloudflare.com/api/graphql";

#[derive(Debug, Serialize, Deserialize)]
pub struct ValidationResult {
    covering: Vec<Roa>,
    state: ValidationState,
}

#[derive(Debug, Serialize, Deserialize, Tabled)]
pub struct Roa {
    asn: u32,
    prefix: RoaPrefix,
}

#[derive(Debug, Serialize, Deserialize, Tabled)]
pub struct RoaPrefix {
    pub prefix: String,
    #[serde(rename(deserialize = "maxLength"))]
    pub max_length: u8,
}

#[derive(Tabled)]
pub struct RoaTableItem {
    asn: u32,
    prefix: String,
    max_length: u8,
}

impl From<Roa> for RoaTableItem {
    fn from(value: Roa) -> Self {
        RoaTableItem {
            asn: value.asn,
            prefix: value.prefix.prefix,
            max_length: value.prefix.max_length,
        }
    }
}

impl From<RoaResource> for Vec<RoaTableItem> {
    fn from(value: RoaResource) -> Self {
        value
            .roas
            .into_iter()
            .map(|p| RoaTableItem {
                asn: value.asn,
                prefix: p.prefix,
                max_length: p.max_length,
            })
            .collect()
    }
}

#[derive(Tabled)]
pub struct SummaryTableItem {
    asn: u32,
    signed: usize,
    routed_valid: usize,
    routed_invalid: usize,
    routed_unknown: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RoaResource {
    pub asn: u32,
    pub roas: Vec<RoaPrefix>,
    pub ta: String,
    #[serde(rename(deserialize = "validFrom"))]
    pub valid_from: i64,
    #[serde(rename(deserialize = "validTo"))]
    pub valid_to: i64,
}

impl Display for RoaPrefix {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (max length: {})", self.prefix, self.max_length)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ValidationState {
    NotFound,
    Valid,
    Invalid,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BgpEntry {
    asn: u32,
    prefix: String,
    validation: ValidationResult,
}

impl Display for ValidationState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationState::NotFound => {
                write!(f, "unknown")
            }
            ValidationState::Valid => {
                write!(f, "valid")
            }
            ValidationState::Invalid => {
                write!(f, "invalid")
            }
        }
    }
}

/// https://rpki-validator.ripe.net/api/v1/validity/13335/1.1.0.0/23
pub fn validate(asn: u32, prefix_str: &str) -> Result<(RpkiValidity, Vec<Roa>)> {
    let prefix = IpNetwork::from_str(prefix_str)?;
    let query_string = format!(
        r#"
    query GetValidation {{
          validation(prefix:"{}", asn:{}){{
            state
            covering {{
              asn
              prefix {{
                maxLength
                prefix
              }}
            }}
          }}
      }}"#,
        prefix, asn
    );

    let res = ureq::post(CLOUDFLARE_RPKI_GRAPHQL)
        .set("Content-Type", "application/json")
        .send_json(ureq::json!({ "query": query_string }))?
        .into_json::<Value>()?;

    let validation_res: ValidationResult = serde_json::from_value(
        res.get("data")
            .unwrap()
            .get("validation")
            .unwrap()
            .to_owned(),
    )?;

    Ok((
        RpkiValidity {
            asn,
            prefix,
            validity: validation_res.state,
        },
        validation_res.covering,
    ))
}

pub fn list_by_prefix(prefix: &IpNetwork) -> Result<Vec<RoaResource>> {
    let query_string = format!(
        r#"
    query GetResources {{
  roas: resource(type: ROA, prefixFilters: {{prefix:"{}", equal:true}}) {{
    ta
    validFrom
    validTo
    ... on ROA {{
      asn
      roas (prefixFilters: {{prefix: "{}", equal: true}}){{
        prefix
        maxLength
      }}
    }}
  }}
}}
    "#,
        &prefix, &prefix
    );
    let res = ureq::post(CLOUDFLARE_RPKI_GRAPHQL)
        .set("Content-Type", "application/json")
        .send_json(ureq::json!({ "query": query_string }))?
        .into_json::<Value>()?
        .get("data")
        .unwrap()
        .get("roas")
        .unwrap()
        .to_owned();

    let resources: Vec<RoaResource> = serde_json::from_value(res).unwrap();
    Ok(resources)
}

pub fn list_by_asn(asn: u32) -> Result<Vec<RoaResource>> {
    let query_string = format!(
        r#"
    query GetResources {{
  roas: resource(type: ROA, asn:{}) {{
    ta
    validFrom
    validTo
    ... on ROA {{
      asn
      roas {{
        prefix
        maxLength
      }}
    }}
  }}
}}
    "#,
        asn
    );

    let res = ureq::post(CLOUDFLARE_RPKI_GRAPHQL)
        .set("Content-Type", "application/json")
        .send_json(ureq::json!({ "query": query_string }))?
        .into_json::<Value>()?
        .get("data")
        .unwrap()
        .get("roas")
        .unwrap()
        .to_owned();

    let resources: Vec<RoaResource> = serde_json::from_value(res).unwrap();
    Ok(resources)
}

pub fn list_routed_by_state(asn: u32, state: ValidationState) -> Result<Vec<BgpEntry>> {
    let route_state_str = match state {
        ValidationState::NotFound => "NotFound",
        ValidationState::Valid => "Valid",
        ValidationState::Invalid => "Invalid",
    };

    let query_string = format!(
        r#"
    query GetRouted {{
          bgp(asn:{}, state:{}){{
          asn
          prefix
          validation {{
                state
                covering {{
                  asn
                  prefix {{
                    maxLength
                    prefix
                  }}
                }}
              }}
          }}
      }}"#,
        asn, route_state_str
    );

    let res = ureq::post(CLOUDFLARE_RPKI_GRAPHQL)
        .set("Content-Type", "application/json")
        .send_json(ureq::json!({ "query": query_string }))?
        .into_json::<Value>()?;

    let bgp_res: Vec<BgpEntry> =
        serde_json::from_value(res.get("data").unwrap().get("bgp").unwrap().to_owned())?;
    Ok(bgp_res)
}

pub fn list_routed(asn: u32) -> Result<(Vec<BgpEntry>, Vec<BgpEntry>, Vec<BgpEntry>)> {
    Ok((
        list_routed_by_state(asn, ValidationState::Valid)?,
        list_routed_by_state(asn, ValidationState::Invalid)?,
        list_routed_by_state(asn, ValidationState::NotFound)?,
    ))
}

pub fn summarize_asn(asn: u32) -> Result<SummaryTableItem> {
    let (valid, invalid, unknown) = list_routed(asn)?;
    let signed = list_by_asn(asn)?;
    Ok(SummaryTableItem {
        asn,
        signed: signed.len(),
        routed_valid: valid.len(),
        routed_invalid: invalid.len(),
        routed_unknown: unknown.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation() {
        dbg!(validate(13335, "1.1.1.0/22").unwrap());
        dbg!(validate(13335, "1.1.1.0/24").unwrap());
        dbg!(validate(13335, "1.1.1.0/26").unwrap());
    }

    #[test]
    fn test_list_prefix() {
        let res = list_by_prefix(&"1.0.0.0/25".parse::<IpNetwork>().unwrap()).unwrap();
        dbg!(&res);
    }

    #[test]
    fn test_list_asn() {
        let res = list_by_asn(400644).unwrap();
        dbg!(&res);
    }

    #[test]
    fn test_bgp() {
        let (valid, invalid, unknown) = list_routed(701).unwrap();
        println!(
            "{} valid, {} invalid, {} unknown",
            valid.len(),
            invalid.len(),
            unknown.len()
        );
    }
}
