use clap::Args;
use json_to_table::json_to_table;
use monocle::fetch_ip_info;
use serde_json::json;
use std::net::IpAddr;

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

pub fn run(args: IpArgs, json: bool) {
    let IpArgs { ip, simple } = args;

    match fetch_ip_info(ip, simple) {
        Ok(ipinfo) => {
            if simple {
                println!("{}", ipinfo.ip);
                return;
            }

            let json_value = json!(&ipinfo);
            if json {
                if let Err(e) = serde_json::to_writer_pretty(std::io::stdout(), &json_value) {
                    eprintln!("Error writing JSON to stdout: {}", e);
                }
            } else {
                let mut table = json_to_table(&json_value);
                table.collapse();
                println!("{}", table);
            }
        }
        Err(e) => {
            eprintln!("ERROR: unable to get ip information: {e}");
        }
    }
}
