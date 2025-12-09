pub mod as2rel;
pub mod broker;
pub mod country;
pub mod ip;
pub mod parse;
pub mod pfx2as;
pub mod radar;
pub mod rpki;
pub mod search;
pub mod time;
pub mod whois;

pub(crate) fn elem_to_string(
    elem: &bgpkit_parser::BgpElem,
    json: bool,
    pretty: bool,
    collector: &str,
) -> Result<String, anyhow::Error> {
    if json {
        let mut val = serde_json::json!(elem);
        val.as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected JSON object"))?
            .insert("collector".to_string(), collector.into());
        if pretty {
            Ok(serde_json::to_string_pretty(&val)?)
        } else {
            Ok(val.to_string())
        }
    } else {
        Ok(format!("{}|{}", elem, collector))
    }
}
