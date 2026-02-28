use clap::Args;
use monocle::lens::ip::{IpInfo, IpLens, IpLookupArgs};
use monocle::utils::OutputFormat;
use serde_json::json;
use std::net::IpAddr;
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the Ip command
#[derive(Args)]
pub struct IpArgs {
    /// IP address to look up (optional)
    #[clap()]
    pub ip: Option<IpAddr>,

    /// Print IP address only (e.g., for getting the public IP address quickly)
    #[clap(long)]
    pub simple: bool,
}

pub fn run(args: IpArgs, output_format: OutputFormat) {
    let IpArgs { ip, simple } = args;

    let lens = IpLens::new();
    let lookup_args = IpLookupArgs {
        ip,
        simple,
        ..Default::default()
    };

    match lens.lookup(&lookup_args) {
        Ok(ipinfo) => {
            if simple {
                println!("{}", ipinfo.ip);
                return;
            }

            format_output(&ipinfo, output_format);
        }
        Err(e) => {
            eprintln!("ERROR: unable to get ip information: {}", e);
        }
    }
}

fn format_output(ipinfo: &IpInfo, output_format: OutputFormat) {
    match output_format {
        OutputFormat::Table => {
            let rows = build_display_rows(ipinfo);
            println!("{}", Table::new(rows).with(Style::rounded()));
        }
        OutputFormat::Markdown => {
            let rows = build_display_rows(ipinfo);
            println!("{}", Table::new(rows).with(Style::markdown()));
        }
        OutputFormat::Json => {
            let json_value = json!(ipinfo);
            match serde_json::to_string(&json_value) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonPretty => {
            let json_value = json!(ipinfo);
            match serde_json::to_string_pretty(&json_value) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::JsonLine => {
            // For a single object, json-line is the same as json
            let json_value = json!(ipinfo);
            match serde_json::to_string(&json_value) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("ERROR: Failed to serialize to JSON: {}", e),
            }
        }
        OutputFormat::Psv => {
            println!("field|value");
            println!("ip|{}", ipinfo.ip);
            if let Some(ref country) = ipinfo.country {
                println!("location|{}", country);
            }
            if let Some(ref asn_info) = ipinfo.asn {
                println!("asn|{}", asn_info.asn);
                println!("name|{}", asn_info.name);
                println!("prefix|{}", asn_info.prefix);
                println!("country|{}", asn_info.country.as_deref().unwrap_or(""));
                println!("rpki|{:?}", asn_info.rpki);
            }
        }
    }
}

#[derive(tabled::Tabled)]
struct DisplayRow {
    field: String,
    value: String,
}

fn build_display_rows(ipinfo: &IpInfo) -> Vec<DisplayRow> {
    let mut rows = vec![DisplayRow {
        field: "ip".to_string(),
        value: ipinfo.ip.clone(),
    }];

    if let Some(ref country) = ipinfo.country {
        rows.push(DisplayRow {
            field: "location".to_string(),
            value: country.clone(),
        });
    }

    if let Some(ref asn_info) = ipinfo.asn {
        rows.push(DisplayRow {
            field: "asn".to_string(),
            value: asn_info.asn.to_string(),
        });
        rows.push(DisplayRow {
            field: "name".to_string(),
            value: asn_info.name.clone(),
        });
        rows.push(DisplayRow {
            field: "prefix".to_string(),
            value: asn_info.prefix.to_string(),
        });
        rows.push(DisplayRow {
            field: "country".to_string(),
            value: asn_info.country.clone().unwrap_or_default(),
        });
        rows.push(DisplayRow {
            field: "rpki".to_string(),
            value: format!("{:?}", asn_info.rpki),
        });
    }

    rows
}
