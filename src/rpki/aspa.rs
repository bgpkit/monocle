use anyhow::Result;
use rpki::repository::aspa::Aspa;
use tabled::Tabled;

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
    let customer = aspa.content().customer_as().into_u32();
    let res = aspa.content().provider_as_set().iter()
        .map(|p|
            AspaObject{ asn: customer, allowed_upstream: p.provider().into_u32()}
        ).collect();
    Ok(res)
}
