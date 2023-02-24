use std::io::Read;
use std::str::FromStr;
use rpki::repository::Roa;
use anyhow::{anyhow, Result};
use ipnetwork::IpNetwork;
use rpki::repository::aspa::Aspa;
use serde_json::Value;
use tabled::Tabled;

#[derive(Debug, Tabled)]
pub struct RoaObject {
    pub asn: u32,
    pub prefix: IpNetwork,
    pub max_len: u8,
}

pub fn read_roa(file_path: &str) -> Result<Vec<RoaObject>> {
    let mut data = vec![];
    let mut reader = oneio::get_reader(file_path)?;
    reader.read_to_end(&mut data)?;
    let roa = Roa::decode(data.as_ref(), true)?;
    let asn: u32 = roa.content.as_id().into_u32();
    let objects = roa.content.iter()
        .map(|addr| {
            let prefix_str = addr.to_string();
            let fields = prefix_str.as_str().split('/').collect::<Vec<&str>>();
            let p = format!("{}/{}", fields[0], fields[1]);
            let prefix = IpNetwork::from_str(p.as_str()).unwrap();
            let max_len = addr.max_length();
            RoaObject{
                asn,
                prefix,
                max_len
            }
        }).collect();
    Ok(objects)
}

#[derive(Debug, Tabled)]
pub struct AspaObject {
    pub asn: u32,
    pub allowed_upstream: u32,
}

pub fn read_aspa(file_path: &str) -> Result<Vec<AspaObject>> {
    let mut data = vec![];
    let mut reader = oneio::get_reader(file_path)?;
    reader.read_to_end(&mut data)?;
    let aspa = Aspa::decode(data.as_ref(), true)?;
    let customer = aspa.content.customer_as().into_u32();
    let res = aspa.content.provider_as_set().iter()
        .map(|p|
            AspaObject{ asn: customer, allowed_upstream: p.provider().into_u32()}
            ).collect();
    Ok(res)
}

#[derive(Debug, Tabled)]
pub struct RpkiValidity {
    asn: u32,
    prefix: IpNetwork,
    validity: String,
}

/// https://rpki-validator.ripe.net/api/v1/validity/13335/1.1.0.0/23
pub fn rpki_validate(asn: u32, prefix_str: &str) -> Result<RpkiValidity> {
    let prefix = IpNetwork::from_str(prefix_str)?;
    let url = format!("https://rpki-validator.ripe.net/api/v1/validity/{}/{}/{}", asn, prefix.network(), prefix.prefix());
    match oneio::read_to_string(url.as_str()) {
        Ok(content) => {
            let res: Value = match serde_json::from_str(content.as_str()){
                Ok(v) => {v},
                Err(_e) => {
                    return Err(anyhow!("invalid asn or prefix"))
                }
            };
            let validity = res.get("validated_route").unwrap().get("validity").unwrap().as_object().unwrap().get("state").unwrap().as_str().unwrap().to_string();
            Ok(RpkiValidity{asn, prefix, validity})
        }
        Err(_e) => {
            Err(anyhow!("invalid asn or prefix"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_read_roa() {
        let objects = read_roa("https://spaces.bgpkit.org/parser/bgpkit.roa").unwrap();
        dbg!(objects);

        let res = read_aspa("https://spaces.bgpkit.org/parser/as945.asa").unwrap();
        dbg!(res);
    }

    #[test]
    fn test_rpki_validate() -> Result<()> {
        let res = rpki_validate(13335, "1.1.1.0/23")?;
        dbg!(res);
        let res = rpki_validate(13335, "1.1.1.0/24")?;
        dbg!(res);
        let res = rpki_validate(13335, "1.1.1.0/25")?;
        dbg!(res);
        let res = rpki_validate(400644, "2620:AA:A000::/48")?;
        dbg!(res);
        Ok(())
    }
}