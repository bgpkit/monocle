mod config;
mod database;
mod datasets;

use anyhow::{anyhow, Result};
use bgpkit_parser::BgpkitParser;
use chrono::{DateTime, TimeZone, Utc};
use chrono_humanize::HumanTime;
use itertools::Itertools;
use std::io::Read;
use std::net::IpAddr;
use tabled::settings::Style;
use tabled::{Table, Tabled};

pub use crate::config::MonocleConfig;
pub use crate::database::*;
pub use crate::datasets::*;

#[allow(clippy::too_many_arguments)]
pub fn parser_with_filters(
    file_path: &str,
    origin_asn: &Option<u32>,
    prefix: &Option<String>,
    include_super: &bool,
    include_sub: &bool,
    peer_ip: &[IpAddr],
    peer_asn: &Option<u32>,
    elem_type: &Option<String>,
    start_ts: &Option<String>,
    end_ts: &Option<String>,
    as_path: &Option<String>,
) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
    let mut parser = BgpkitParser::new(file_path).unwrap().disable_warnings();

    if let Some(v) = as_path {
        parser = parser
            .add_filter("as_path", v.to_string().as_str())
            .unwrap();
    }
    if let Some(v) = origin_asn {
        parser = parser
            .add_filter("origin_asn", v.to_string().as_str())
            .unwrap();
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
    if !peer_ip.is_empty() {
        let v = peer_ip.iter().map(|p| p.to_string()).join(",");
        parser = parser.add_filter("peer_ips", v.as_str()).unwrap();
    }
    if let Some(v) = peer_asn {
        parser = parser
            .add_filter("peer_asn", v.to_string().as_str())
            .unwrap();
    }
    if let Some(v) = elem_type {
        parser = parser.add_filter("type", v.to_string().as_str()).unwrap();
    }
    if let Some(v) = start_ts {
        let ts = string_to_time(v.as_str())?;
        parser = parser
            .add_filter("start_ts", ts.to_string().as_str())
            .unwrap();
    }
    if let Some(v) = end_ts {
        let ts = string_to_time(v.as_str())?;
        parser = parser
            .add_filter("end_ts", ts.to_string().as_str())
            .unwrap();
    }
    Ok(parser)
}

#[derive(Tabled)]
struct BgpTime {
    unix: i64,
    rfc3339: String,
    human: String,
}

pub fn string_to_time(time_string: &str) -> Result<i64> {
    let ts = match chrono::DateTime::parse_from_rfc3339(time_string) {
        Ok(ts) => ts.timestamp(),
        Err(_) => match time_string.parse::<f64>() {
            Ok(ts) => ts as i64,
            Err(_) => {
                return Err(anyhow!(
                "Input time must be either Unix timestamp or time string compliant with RFC3339"
            ))
            }
        },
    };

    Ok(ts)
}

pub fn convert_time_string(time_vec: &[String]) -> Result<String> {
    let time_strings = match time_vec.len() {
        0 => vec![Utc::now().to_rfc3339()],
        _ => {
            // check if ts is a valid Unix timestamp
            time_vec
                .iter()
                .map(|ts| {
                    match ts.parse::<f64>() {
                        Ok(timestamp) => {
                            let dt = Utc.timestamp_opt(timestamp as i64, 0).unwrap();
                            dt.to_rfc3339()
                        }
                        Err(_) => {
                            // not a time stamp, check if it is a valid RFC3339 string,
                            // if so, return the unix timestamp as string; otherwise, return error
                            match chrono::DateTime::parse_from_rfc3339(ts) {
                                Ok(dt) => dt.timestamp().to_string(),
                                Err(_) => "".to_string(),
                            }
                        }
                    }
                })
                .collect()
        }
    };

    Ok(time_strings.join("\n"))
}

pub fn time_to_table(time_vec: &[String]) -> Result<String> {
    let now_ts = Utc::now().timestamp();
    let ts_vec = match time_vec.is_empty() {
        true => vec![now_ts],
        false => time_vec
            .iter()
            .map(|ts| string_to_time(ts.as_str()).unwrap_or_default())
            .collect(),
    };

    let bgptime_vec = ts_vec
        .into_iter()
        .map(|ts| {
            let ht = HumanTime::from(chrono::Local::now() - chrono::Duration::seconds(now_ts - ts));
            let human = ht.to_string();
            let rfc3339 = Utc
                .from_utc_datetime(&DateTime::from_timestamp(ts, 0).unwrap().naive_utc())
                .to_rfc3339();
            BgpTime {
                unix: ts,
                rfc3339,
                human,
            }
        })
        .collect::<Vec<BgpTime>>();

    Ok(Table::new(bgptime_vec).with(Style::rounded()).to_string())
}
