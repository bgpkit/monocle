use std::str::FromStr;
use tabled::Tabled;
use ipnetwork::IpNetwork;
use anyhow::Result;

#[derive(Debug, Tabled)]
pub struct RpkiValidity {
    asn: u32,
    prefix: IpNetwork,
    validity: String,
    invalid_reason: String
}

const CLOUDFLARE_RPKI_GRAPHQL: &str = "https://rpki.cloudflare.com/api/graphql";

/// https://rpki-validator.ripe.net/api/v1/validity/13335/1.1.0.0/23
pub fn rpki_validate(asn: u32, prefix_str: &str) -> Result<RpkiValidity> {
    let prefix = IpNetwork::from_str(prefix_str)?;
    let query_string = format!(r#"
    {{
                "query": "query GetValidation {{
  validation(prefix:\\"{}\\", asn:{}){{
    state
    covering {{
      asn
      prefix {{
        maxLength
        prefix
      }}
    }}
  }}
  }}"
}}"#, prefix.to_string(), asn);

    println!("{}", query_string.as_str());
    let res = ureq::post(CLOUDFLARE_RPKI_GRAPHQL)
        .set("Content-Type", "application/json")
        .send_string(query_string.as_str())?.into_string()?;
    println!("{}", res.to_string().as_str());
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation() {
        rpki_validate(13335, "1.1.1.0/24");
    }
}
