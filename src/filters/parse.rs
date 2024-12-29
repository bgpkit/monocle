use crate::filters::MrtParserFilters;
use crate::time::string_to_time;
use crate::ElemTypeEnum;
use anyhow::anyhow;
use anyhow::Result;
use bgpkit_parser::BgpkitParser;
use clap::Args;
use itertools::Itertools;
use std::io::Read;
use std::net::IpAddr;

#[derive(Args, Debug, Clone)]
pub struct ParseFilters {
    /// Filter by origin AS Number
    #[clap(short = 'o', long)]
    pub origin_asn: Option<u32>,

    /// Filter by network prefix
    #[clap(short = 'p', long)]
    pub prefix: Option<String>,

    /// Include super-prefix when filtering
    #[clap(short = 's', long)]
    pub include_super: bool,

    /// Include sub-prefix when filtering
    #[clap(short = 'S', long)]
    pub include_sub: bool,

    /// Filter by peer IP address
    #[clap(short = 'j', long)]
    pub peer_ip: Vec<IpAddr>,

    /// Filter by peer ASN
    #[clap(short = 'J', long)]
    pub peer_asn: Option<u32>,

    /// Filter by elem type: announce (a) or withdraw (w)
    #[clap(short = 'm', long, value_enum)]
    pub elem_type: Option<ElemTypeEnum>,

    /// Filter by start unix timestamp inclusive
    #[clap(short = 't', long)]
    pub start_ts: Option<String>,

    /// Filter by end unix timestamp inclusive
    #[clap(short = 'T', long)]
    pub end_ts: Option<String>,

    /// Duration from the start-ts or end-ts, e.g. 1h
    #[clap(short = 'd', long)]
    pub duration: Option<String>,

    /// Filter by AS path regex string
    #[clap(short = 'a', long)]
    pub as_path: Option<String>,
}

impl ParseFilters {
    pub fn parse_start_end_strings(&self) -> Result<(String, String)> {
        let mut start_ts = None;
        let mut end_ts = None;
        if let Some(ts) = &self.start_ts {
            match string_to_time(ts.as_str()) {
                Ok(t) => start_ts = Some(t),
                Err(_) => return Err(anyhow!("start-ts is not a valid time string: {}", ts)),
            }
        }
        if let Some(ts) = &self.end_ts {
            match string_to_time(ts.as_str()) {
                Ok(t) => end_ts = Some(t),
                Err(_) => return Err(anyhow!("end-ts is not a valid time string: {}", ts)),
            }
        }

        match (&self.start_ts, &self.end_ts, &self.duration) {
            (Some(_), Some(_), Some(_)) => {
                return Err(anyhow!(
                    "cannot specify start_ts, end_ts, and duration all at the same time"
                ))
            }
            (Some(_), None, None) | (None, Some(_), None) => {
                // only one start_ts or end_ts specified
                return Err(anyhow!(
                    "must specify two from: start_ts, end_ts and duration"
                ));
            }
            (None, None, _) => {
                return Err(anyhow!(
                    "must specify two from: start_ts, end_ts and duration"
                ));
            }
            _ => {}
        }
        if let Some(duration) = &self.duration {
            // this case is duration + start_ts OR end_ts
            let duration = match humantime::parse_duration(duration) {
                Ok(d) => d,
                Err(_) => {
                    return Err(anyhow!(
                        "duration is not a valid time duration string: {}",
                        duration
                    ))
                }
            };

            if let Some(ts) = start_ts {
                return Ok((ts.to_rfc3339(), (ts + duration).to_rfc3339()));
            }
            if let Some(ts) = end_ts {
                return Ok(((ts - duration).to_rfc3339(), ts.to_rfc3339()));
            }
        } else {
            // this case is start_ts AND end_ts
            return Ok((start_ts.unwrap().to_rfc3339(), end_ts.unwrap().to_rfc3339()));
        }

        Err(anyhow!("unexpected time-string parsing result"))
    }
}

impl MrtParserFilters for ParseFilters {
    fn validate(&self) -> Result<()> {
        if let Some(ts) = &self.start_ts {
            if string_to_time(ts.as_str()).is_err() {
                return Err(anyhow!("start-ts is not a valid time string: {}", ts));
            }
        }
        if let Some(ts) = &self.end_ts {
            if string_to_time(ts.as_str()).is_err() {
                return Err(anyhow!("end-ts is not a valid time string: {}", ts));
            }
        }
        Ok(())
    }

    fn to_parser(&self, file_path: &str) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        let mut parser = BgpkitParser::new(file_path)?.disable_warnings();

        if let Some(v) = &self.as_path {
            parser = parser.add_filter("as_path", v.to_string().as_str())?;
        }
        if let Some(v) = &self.origin_asn {
            parser = parser.add_filter("origin_asn", v.to_string().as_str())?;
        }
        if let Some(v) = &self.prefix {
            let filter_type = match (self.include_super, self.include_sub) {
                (false, false) => "prefix",
                (true, false) => "prefix_super",
                (false, true) => "prefix_sub",
                (true, true) => "prefix_super_sub",
            };
            parser = parser.add_filter(filter_type, v.as_str())?;
        }
        if !self.peer_ip.is_empty() {
            let v = self.peer_ip.iter().map(|p| p.to_string()).join(",");
            parser = parser.add_filter("peer_ips", v.as_str())?;
        }
        if let Some(v) = &self.peer_asn {
            parser = parser.add_filter("peer_asn", v.to_string().as_str())?;
        }
        if let Some(v) = &self.elem_type {
            parser = parser.add_filter("type", v.to_string().as_str())?;
        }

        match self.parse_start_end_strings() {
            Ok((start_ts, end_ts)) => {
                // in case we have full start_ts and end_ts, like in `monocle search` command input,
                // we will use the parsed start_ts and end_ts.
                parser = parser.add_filter("start_ts", start_ts.to_string().as_str())?;
                parser = parser.add_filter("end_ts", end_ts.to_string().as_str())?;
            }
            Err(_) => {
                // we could also likely not have any time filters, in this case, add filters
                // as we see them, and no modification is needed.
                if let Some(v) = &self.start_ts {
                    let ts = string_to_time(v.as_str())?.timestamp();
                    parser = parser.add_filter("start_ts", ts.to_string().as_str())?;
                }
                if let Some(v) = &self.end_ts {
                    let ts = string_to_time(v.as_str())?.timestamp();
                    parser = parser.add_filter("end_ts", ts.to_string().as_str())?;
                }
            }
        }

        Ok(parser)
    }
}
