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
    pub country: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpInfo {
    pub ip: String,
    #[serde(rename(serialize = "location"))]
    pub country: Option<String>,
    #[serde(rename(serialize = "network"))]
    pub asn: Option<AsnRouteInfo>,
}

const IP_INFO_API: &str = "https://api.bgpkit.com/v3/utils/ip";
pub fn fetch_ip_info(ip_opt: Option<IpAddr>, simple: bool) -> Result<IpInfo> {
    let mut params = vec![];
    if let Some(ip) = ip_opt {
        params.push(format!("ip={}", ip));
    }
    if simple {
        params.push("simple=true".to_string());
    }
    let url = format!("{}?{}", IP_INFO_API, params.join("&"));
    let resp = ureq::get(&url).call()?.body_mut().read_json::<IpInfo>()?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fetch_ip_info() {
        let my_public_ip_info = fetch_ip_info(None, false).unwrap();
        dbg!(my_public_ip_info);
    }
}
