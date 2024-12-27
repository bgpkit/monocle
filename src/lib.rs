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
        parser = parser.add_filter("as_path", v.to_string().as_str())?;
    }
    if let Some(v) = origin_asn {
        parser = parser.add_filter("origin_asn", v.to_string().as_str())?;
    }
    if let Some(v) = prefix {
        let filter_type = match (include_super, include_sub) {
            (false, false) => "prefix",
            (true, false) => "prefix_super",
            (false, true) => "prefix_sub",
            (true, true) => "prefix_super_sub",
        };
        parser = parser.add_filter(filter_type, v.as_str())?;
    }
    if !peer_ip.is_empty() {
        let v = peer_ip.iter().map(|p| p.to_string()).join(",");
        parser = parser.add_filter("peer_ips", v.as_str())?;
    }
    if let Some(v) = peer_asn {
        parser = parser.add_filter("peer_asn", v.to_string().as_str())?;
    }
    if let Some(v) = elem_type {
        parser = parser.add_filter("type", v.to_string().as_str())?;
    }
    if let Some(v) = start_ts {
        let ts = string_to_time(v.as_str())?.timestamp();
        parser = parser.add_filter("start_ts", ts.to_string().as_str())?;
    }
    if let Some(v) = end_ts {
        let ts = string_to_time(v.as_str())?.timestamp();
        parser = parser.add_filter("end_ts", ts.to_string().as_str())?;
    }
    Ok(parser)
}

#[derive(Tabled)]
struct BgpTime {
    unix: i64,
    rfc3339: String,
    human: String,
}

pub fn string_to_time(time_string: &str) -> Result<DateTime<Utc>> {
    let ts = match dateparser::parse_with(
        time_string,
        &Utc,
        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
    ) {
        Ok(ts) => ts,
        Err(_) => {
            return Err(anyhow!(
                "Input time must be either Unix timestamp or time string compliant with RFC3339"
            ))
        }
    };

    Ok(ts)
}

pub fn parse_time_string_to_rfc3339(time_vec: &[String]) -> Result<String> {
    let mut time_strings = vec![];
    if time_vec.is_empty() {
        time_strings.push(Utc::now().to_rfc3339())
    } else {
        for ts in time_vec {
            match string_to_time(ts) {
                Ok(ts) => time_strings.push(ts.to_rfc3339()),
                Err(_) => return Err(anyhow!("unable to parse timestring: {}", ts)),
            }
        }
    }

    Ok(time_strings.join("\n"))
}

pub fn time_to_table(time_vec: &[String]) -> Result<String> {
    let now_ts = Utc::now().timestamp();
    let ts_vec = match time_vec.is_empty() {
        true => vec![now_ts],
        false => time_vec
            .iter()
            .map(|ts| string_to_time(ts.as_str()).map(|dt| dt.timestamp()))
            .collect::<Result<Vec<_>>>()?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_to_time() {
        use chrono::TimeZone;

        // Test with a valid Unix timestamp
        let unix_ts = "1697043600"; // Example timestamp
        let result = string_to_time(unix_ts);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Utc.timestamp_opt(1697043600, 0).unwrap());

        // Test with a valid RFC3339 string
        let rfc3339_str = "2023-10-11T00:00:00Z";
        let result = string_to_time(rfc3339_str);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Utc.timestamp_opt(1696982400, 0).unwrap());

        // Test with an incorrect date string
        let invalid_date = "not-a-date";
        let result = string_to_time(invalid_date);
        assert!(result.is_err());

        // Test with an empty string
        let empty_string = "";
        let result = string_to_time(empty_string);
        assert!(result.is_err());

        // Test with incomplete RFC3339 string
        let incomplete_rfc3339 = "2023-10-11T";
        let result = string_to_time(incomplete_rfc3339);
        assert!(result.is_err());

        // Test with a human-readable date string allowed by `dateparser`
        let human_readable = "October 11, 2023";
        let result = string_to_time(human_readable);
        assert!(result.is_ok());
        let expected_time = Utc.with_ymd_and_hms(2023, 10, 11, 0, 0, 0).unwrap();
        assert_eq!(result.unwrap(), expected_time);
    }
}
