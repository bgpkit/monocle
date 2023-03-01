use std::str::FromStr;
use anyhow::Result;
use ipnetwork::IpNetwork;
use rpki::repository::Roa;
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
    let asn: u32 = roa.content().as_id().into_u32();
    let objects = roa.content().iter()
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

