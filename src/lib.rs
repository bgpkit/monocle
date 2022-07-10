use std::net::IpAddr;
use bgpkit_parser::BgpkitParser;
use itertools::Itertools;

pub fn parser_with_filters(
    file_path: &str,
    origin_asn: &Option<u32>,
    prefix: &Option<String>,
    include_super: &bool,
    include_sub: &bool,
    peer_ip: &Vec<IpAddr>,
    peer_asn: &Option<u32>,
    elem_type: &Option<String>,
    start_ts: &Option<f64>,
    end_ts: &Option<f64>,
    as_path: &Option<String>,
) -> BgpkitParser {

    let mut parser = BgpkitParser::new(file_path).unwrap().disable_warnings();

    if let Some(v) = as_path {
        parser = parser.add_filter("as_path", v.to_string().as_str()).unwrap();
    }
    if let Some(v) = origin_asn {
        parser = parser.add_filter("origin_asn", v.to_string().as_str()).unwrap();
    }
    if let Some(v) = prefix {
        let filter_type = match (include_super, include_sub) {
            (false, false) => "prefix",
            (true, false) => "prefix_super",
            (false, true) => "prefix_sub",
            (true, true) => "prefix_super_sub",
        };
        parser = parser.add_filter(filter_type, v.as_str()).unwrap();
    }
    if !peer_ip.is_empty(){
        let v = peer_ip.iter().map(|p| p.to_string()).join(",").to_string();
        parser = parser.add_filter("peer_ips", v.as_str()).unwrap();
    }
    if let Some(v) = peer_asn {
        parser = parser.add_filter("peer_asn", v.to_string().as_str()).unwrap();
    }
    if let Some(v) = elem_type {
        parser = parser.add_filter("type", v.to_string().as_str()).unwrap();
    }
    if let Some(v) = start_ts {
        parser = parser.add_filter("start_ts", v.to_string().as_str()).unwrap();
    }
    if let Some(v) = end_ts {
        parser = parser.add_filter("end_ts", v.to_string().as_str()).unwrap();
    }
    return parser
}