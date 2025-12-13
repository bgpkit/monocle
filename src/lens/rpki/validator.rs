use anyhow::Result;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use tabled::Tabled;

#[derive(Debug, Tabled, Serialize)]
pub struct RpkiValidity {
    asn: u32,
    prefix: IpNet,
    validity: RpkiValidationState,
}

const CLOUDFLARE_RPKI_GRAPHQL: &str = "https://rpki.cloudflare.com/api/graphql";

#[derive(Debug, Serialize, Deserialize)]
pub struct RpkiValidationResult {
    covering: Vec<RpkiRoa>,
    state: RpkiValidationState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct RpkiRoa {
    asn: u32,
    prefix: RpkiRoaPrefix,
}

#[derive(Debug, Clone, Serialize, Deserialize, Tabled)]
pub struct RpkiRoaPrefix {
    pub prefix: String,
    #[serde(rename(deserialize = "maxLength"))]
    pub max_length: u8,
}

#[derive(Tabled, Serialize)]
pub struct RpkiRoaTableItem {
    asn: u32,
    prefix: String,
    max_length: u8,
}

impl From<RpkiRoa> for RpkiRoaTableItem {
    fn from(value: RpkiRoa) -> Self {
        RpkiRoaTableItem {
            asn: value.asn,
            prefix: value.prefix.prefix,
            max_length: value.prefix.max_length,
        }
    }
}

impl From<RpkiRoaResource> for Vec<RpkiRoaTableItem> {
    fn from(value: RpkiRoaResource) -> Self {
        value
            .roas
            .into_iter()
            .map(|p| RpkiRoaTableItem {
                asn: value.asn,
                prefix: p.prefix,
                max_length: p.max_length,
            })
            .collect()
    }
}

#[derive(Tabled, Serialize)]
pub struct RpkiSummaryTableItem {
    asn: u32,
    signed: usize,
    routed_valid: usize,
    routed_invalid: usize,
    routed_unknown: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpkiRoaResource {
    pub asn: u32,
    pub roas: Vec<RpkiRoaPrefix>,
    pub ta: String,
    #[serde(rename(deserialize = "validFrom"))]
    pub valid_from: i64,
    #[serde(rename(deserialize = "validTo"))]
    pub valid_to: i64,
}

impl Display for RpkiRoaPrefix {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (max length: {})", self.prefix, self.max_length)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum RpkiValidationState {
    NotFound,
    Valid,
    Invalid,
}

impl Display for RpkiValidationState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RpkiValidationState::NotFound => {
                write!(f, "unknown")
            }
            RpkiValidationState::Valid => {
                write!(f, "valid")
            }
            RpkiValidationState::Invalid => {
                write!(f, "invalid")
            }
        }
    }
}

/// https://rpki-validator.ripe.net/api/v1/validity/13335/1.1.0.0/23
pub fn validate(asn: u32, prefix_str: &str) -> Result<(RpkiValidity, Vec<RpkiRoa>)> {
    let prefix = IpNet::from_str(prefix_str)?;
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
        .header("Content-Type", "application/json")
        .send_json(serde_json::json! ({ "query": query_string }))?
        .body_mut()
        .read_json::<Value>()?;

    let validation_res: RpkiValidationResult = serde_json::from_value(
        res.get("data")
            .ok_or_else(|| anyhow::anyhow!("No 'data' field in response"))?
            .get("validation")
            .ok_or_else(|| anyhow::anyhow!("No 'validation' field in response data"))?
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

pub fn list_by_prefix(prefix: &IpNet) -> Result<Vec<RpkiRoaResource>> {
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
    let response = ureq::post(CLOUDFLARE_RPKI_GRAPHQL)
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({ "query": query_string }))?
        .body_mut()
        .read_json::<Value>()?;

    let res = response
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("No 'data' field in response"))?
        .get("roas")
        .ok_or_else(|| anyhow::anyhow!("No 'roas' field in response data"))?
        .to_owned();

    let resources: Vec<RpkiRoaResource> = serde_json::from_value(res)
        .map_err(|e| anyhow::anyhow!("Failed to parse ROA resources: {}", e))?;
    Ok(resources)
}

pub fn list_by_asn(asn: u32) -> Result<Vec<RpkiRoaResource>> {
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

    let response = ureq::post(CLOUDFLARE_RPKI_GRAPHQL)
        .header("Content-Type", "application/json")
        .send_json(serde_json::json!({ "query": query_string }))?
        .body_mut()
        .read_json::<Value>()?;

    let res = response
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("No 'data' field in response"))?
        .get("roas")
        .ok_or_else(|| anyhow::anyhow!("No 'roas' field in response data"))?
        .to_owned();

    let resources: Vec<RpkiRoaResource> = serde_json::from_value(res)
        .map_err(|e| anyhow::anyhow!("Failed to parse ROA resources: {}", e))?;
    Ok(resources)
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
        let res = list_by_prefix(&"1.0.0.0/25".parse::<IpNet>().unwrap()).unwrap();
        dbg!(&res);
    }

    #[test]
    fn test_list_asn() {
        let res = list_by_asn(400644).unwrap();
        dbg!(&res);
    }
}
