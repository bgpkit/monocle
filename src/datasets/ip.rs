use anyhow::Result;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RpkiValidationState {
    #[serde(rename = "valid")]
    Valid,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "unknown")]
    NotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsnRouteInfo {
    pub asn: i64,
    #[serde(rename(serialize = "prefix"))]
    pub prefix: IpNet,
    pub rpki: RpkiValidationState,
    pub name: String,
    pub country: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpInfo {
    pub ip: String,
    #[serde(rename(serialize = "location"))]
    pub country: String,
    #[serde(rename(serialize = "network"))]
    pub asn: Option<AsnRouteInfo>,
}

pub fn fetch_ip_info(ip_opt: Option<IpAddr>) -> Result<IpInfo> {
    let url = match ip_opt {
        Some(ip) => format!("https://api.bgpkit.com/v3/utils/ip?ip={}", ip),
        None => "https://api.bgpkit.com/v3/utils/ip".to_string(),
    };
    let resp = ureq::get(&url).call()?.into_json::<IpInfo>()?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_ip_info() {
        let my_public_ip_info = fetch_ip_info(None).unwrap();
        dbg!(my_public_ip_info);
    }
}
