use clap::Subcommand;
use radar_rs::RadarClient;
use serde::Serialize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Subcommand)]
pub enum RadarCommands {
    /// get routing stats
    Stats {
        /// a two-letter country code or asn number (e.g., US or 13335)
        #[clap(name = "QUERY")]
        query: Option<String>,
    },

    /// look up prefix-to-origin mapping on the most recent global routing table snapshot
    Pfx2as {
        /// an IP prefix or an AS number (e.g., 1.1.1.0/24 or 13335)
        #[clap(name = "QUERY")]
        query: String,

        /// filter by RPKI validation status, valid, invalid, or unknown
        #[clap(short, long)]
        rpki_status: Option<String>,
    },
}

pub fn run(commands: RadarCommands, json: bool) {
    let client = match RadarClient::new() {
        Ok(client) => client,
        Err(e) => {
            eprintln!("Failed to create Radar client: {}", e);
            std::process::exit(1);
        }
    };

    match commands {
        RadarCommands::Stats { query } => run_stats(&client, query, json),
        RadarCommands::Pfx2as { query, rpki_status } => {
            run_pfx2as(&client, query, rpki_status, json)
        }
    }
}

fn run_stats(client: &RadarClient, query: Option<String>, json: bool) {
    let (country, asn) = match query {
        None => (None, None),
        Some(q) => match q.parse::<u32>() {
            Ok(asn) => (None, Some(asn)),
            Err(_) => (Some(q), None),
        },
    };

    let res = match client.get_bgp_routing_stats(asn, country.clone()) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("ERROR: unable to get routing stats: {}", e);
            return;
        }
    };

    let scope = match (country, &asn) {
        (None, None) => "global".to_string(),
        (Some(c), None) => c,
        (None, Some(asn)) => format!("as{}", asn),
        (Some(_), Some(_)) => {
            eprintln!("ERROR: cannot specify both country and ASN");
            return;
        }
    };

    #[derive(Tabled, Serialize)]
    struct Stats {
        pub scope: String,
        pub origins: u32,
        pub prefixes: u32,
        pub rpki_valid: String,
        pub rpki_invalid: String,
        pub rpki_unknown: String,
    }
    let table_data = vec![
        Stats {
            scope: scope.clone(),
            origins: res.stats.distinct_origins,
            prefixes: res.stats.distinct_prefixes,
            rpki_valid: format!(
                "{} ({:.2}%)",
                res.stats.routes_valid,
                (res.stats.routes_valid as f64 / res.stats.routes_total as f64) * 100.0
            ),
            rpki_invalid: format!(
                "{} ({:.2}%)",
                res.stats.routes_invalid,
                (res.stats.routes_invalid as f64 / res.stats.routes_total as f64) * 100.0
            ),
            rpki_unknown: format!(
                "{} ({:.2}%)",
                res.stats.routes_unknown,
                (res.stats.routes_unknown as f64 / res.stats.routes_total as f64) * 100.0
            ),
        },
        Stats {
            scope: format!("{} ipv4", scope),
            origins: res.stats.distinct_origins_ipv4,
            prefixes: res.stats.distinct_prefixes_ipv4,
            rpki_valid: format!(
                "{} ({:.2}%)",
                res.stats.routes_valid_ipv4,
                (res.stats.routes_valid_ipv4 as f64 / res.stats.routes_total_ipv4 as f64) * 100.0
            ),
            rpki_invalid: format!(
                "{} ({:.2}%)",
                res.stats.routes_invalid_ipv4,
                (res.stats.routes_invalid_ipv4 as f64 / res.stats.routes_total_ipv4 as f64) * 100.0
            ),
            rpki_unknown: format!(
                "{} ({:.2}%)",
                res.stats.routes_unknown_ipv4,
                (res.stats.routes_unknown_ipv4 as f64 / res.stats.routes_total_ipv4 as f64) * 100.0
            ),
        },
        Stats {
            scope: format!("{} ipv6", scope),
            origins: res.stats.distinct_origins_ipv6,
            prefixes: res.stats.distinct_prefixes_ipv6,
            rpki_valid: format!(
                "{} ({:.2}%)",
                res.stats.routes_valid_ipv6,
                (res.stats.routes_valid_ipv6 as f64 / res.stats.routes_total_ipv6 as f64) * 100.0
            ),
            rpki_invalid: format!(
                "{} ({:.2}%)",
                res.stats.routes_invalid_ipv6,
                (res.stats.routes_invalid_ipv6 as f64 / res.stats.routes_total_ipv6 as f64) * 100.0
            ),
            rpki_unknown: format!(
                "{} ({:.2}%)",
                res.stats.routes_unknown_ipv6,
                (res.stats.routes_unknown_ipv6 as f64 / res.stats.routes_total_ipv6 as f64) * 100.0
            ),
        },
    ];
    if json {
        match serde_json::to_string_pretty(&table_data) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("Failed to serialize JSON: {}", e),
        }
    } else {
        println!("{}", Table::new(table_data).with(Style::modern()));
        println!("\nData generated at {} UTC.", res.meta.data_time);
    }
}

fn run_pfx2as(client: &RadarClient, query: String, rpki_status: Option<String>, json: bool) {
    let (asn, prefix) = match query.parse::<u32>() {
        Ok(asn) => (Some(asn), None),
        Err(_) => (None, Some(query)),
    };

    let rpki = if let Some(rpki_status) = rpki_status {
        match rpki_status.to_lowercase().as_str() {
            "valid" | "invalid" | "unknown" => Some(rpki_status),
            _ => {
                eprintln!("ERROR: invalid rpki status: {}", rpki_status);
                return;
            }
        }
    } else {
        None
    };

    let res = match client.get_bgp_prefix_origins(asn, prefix, rpki) {
        Ok(res) => res,
        Err(e) => {
            eprintln!("ERROR: unable to get prefix origins: {}", e);
            return;
        }
    };

    #[derive(Tabled, Serialize)]
    struct Pfx2origin {
        pub prefix: String,
        pub origin: String,
        pub rpki: String,
        pub visibility: String,
    }

    if res.prefix_origins.is_empty() {
        println!("no prefix origins found for the given query");
        return;
    }

    fn count_to_visibility(count: u32, total: u32) -> String {
        let ratio = count as f64 / total as f64;
        if ratio > 0.8 {
            format!("high ({:.2}%)", ratio * 100.0)
        } else if ratio < 0.2 {
            format!("low ({:.2}%)", ratio * 100.0)
        } else {
            format!("mid ({:.2}%)", ratio * 100.0)
        }
    }

    let table_data = res
        .prefix_origins
        .into_iter()
        .map(|entry| Pfx2origin {
            prefix: entry.prefix,
            origin: format!("as{}", entry.origin),
            rpki: entry.rpki_validation.to_lowercase(),
            visibility: count_to_visibility(entry.peer_count as u32, res.meta.total_peers as u32),
        })
        .collect::<Vec<Pfx2origin>>();
    if json {
        match serde_json::to_string_pretty(&table_data) {
            Ok(json_str) => println!("{}", json_str),
            Err(e) => eprintln!("Error serializing data to JSON: {}", e),
        }
    } else {
        println!("{}", Table::new(table_data).with(Style::modern()));
        println!("\nData generated at {} UTC.", res.meta.data_time);
    }
}
