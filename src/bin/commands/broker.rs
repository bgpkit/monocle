use clap::Args;
use monocle::lens::time::TimeLens;
use serde_json;
use tabled::settings::Style;
use tabled::{Table, Tabled};

/// Arguments for the Broker command
#[derive(Args)]
pub struct BrokerArgs {
    /// starting timestamp (RFC3339 or unix epoch)
    #[clap(long, short = 't')]
    pub start_ts: String,

    /// ending timestamp (RFC3339 or unix epoch)
    #[clap(long, short = 'T')]
    pub end_ts: String,

    /// BGP collector name: e.g. rrc00, route-views2
    #[clap(long, short = 'c')]
    pub collector: Option<String>,

    /// BGP collection project name, e.g. routeviews, or riperis
    #[clap(long, short = 'P')]
    pub project: Option<String>,

    /// Data type, e.g., updates or rib
    #[clap(long)]
    pub data_type: Option<String>,

    /// Page number to fetch (1-based). If set, only this page will be fetched.
    #[clap(long)]
    pub page: Option<i64>,

    /// Page size for broker queries (default 1000)
    #[clap(long)]
    pub page_size: Option<i64>,
}

pub fn run(args: BrokerArgs, json: bool) {
    let BrokerArgs {
        start_ts,
        end_ts,
        collector,
        project,
        data_type,
        page,
        page_size,
    } = args;

    let time_lens = TimeLens::new();

    // parse time strings similar to Search subcommand
    let ts_start = match time_lens.parse_time_string(&start_ts) {
        Ok(t) => t.timestamp(),
        Err(_) => {
            eprintln!("start-ts is not a valid time string: {}", start_ts);
            std::process::exit(1);
        }
    };
    let ts_end = match time_lens.parse_time_string(&end_ts) {
        Ok(t) => t.timestamp(),
        Err(_) => {
            eprintln!("end-ts is not a valid time string: {}", end_ts);
            std::process::exit(1);
        }
    };

    let mut broker = bgpkit_broker::BgpkitBroker::new()
        .ts_start(ts_start)
        .ts_end(ts_end);

    if let Some(c) = collector {
        broker = broker.collector_id(c.as_str());
    }
    if let Some(p) = project {
        broker = broker.project(p.as_str());
    }
    if let Some(dt) = data_type {
        broker = broker.data_type(dt.as_str());
    }

    let page_size = page_size.unwrap_or(1000);
    broker = broker.page_size(page_size);

    let res = if let Some(p) = page {
        broker.page(p).query_single_page()
    } else {
        // Use query() and limit to at most 10 pages worth of items
        match broker.query() {
            Ok(mut v) => {
                let max_items = (page_size * 10) as usize;
                if v.len() > max_items {
                    v.truncate(max_items);
                }
                Ok(v)
            }
            Err(e) => Err(e),
        }
    };

    match res {
        Ok(items) => {
            if items.is_empty() {
                if json {
                    println!("[]");
                } else {
                    println!("No MRT files found");
                }
                return;
            }

            if json {
                match serde_json::to_string_pretty(&items) {
                    Ok(json_str) => println!("{}", json_str),
                    Err(e) => eprintln!("error serializing: {}", e),
                }
            } else {
                #[derive(Tabled)]
                struct BrokerItemDisplay {
                    #[tabled(rename = "Collector")]
                    collector_id: String,
                    #[tabled(rename = "Type")]
                    data_type: String,
                    #[tabled(rename = "Start Time (UTC)")]
                    ts_start: String,
                    #[tabled(rename = "URL")]
                    url: String,
                    #[tabled(rename = "Size (Bytes)")]
                    rough_size: i64,
                }

                let display_items: Vec<BrokerItemDisplay> = items
                    .into_iter()
                    .map(|item| BrokerItemDisplay {
                        collector_id: item.collector_id,
                        data_type: item.data_type,
                        ts_start: item.ts_start.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                        url: item.url,
                        rough_size: item.rough_size,
                    })
                    .collect();

                println!("{}", Table::new(display_items).with(Style::markdown()));
            }
        }
        Err(e) => {
            eprintln!("failed to query: {}", e);
        }
    }
}
