/*!

## Algorithm

For a given time range to query, we first split the time range into time bins. The time bin size is
defined as:
- with RouteViews collectors: 15-minute bin
- only RIPE RIS collectors: 5-minute bin
Each bin is aligned to the full 5/15 minute mark, which would then be aligned with the existing data
from RouteViews and RIPE RIS.

For each bin, we will then parse and collect all BGP messages in the range, and sort them by time
and collector. This allows us the produce a single stream of BGP messages in chronological order.
We use priority queue for sorting.
*/

use std::collections::BinaryHeap;
use bgpkit_broker::{BrokerItem, QueryParams, SortOrder};
use bgpkit_parser::BgpElem;
use itertools::Itertools;
use rayon::prelude::*;
use tracing::{debug, error, info, span, warn, Level};

#[derive(Debug)]
pub struct DataFileBin {
    ts_start: i64,
    ts_end: i64,
    files: Vec<String>
}

/// find and collect data file bins, each bin contains a group of files that should be processed together.f:w
pub fn find_bins(ts_start: f64, ts_end: f64, bin_width: i64) -> Vec<DataFileBin> {
    let ts_start_rounded = ((ts_start /bin_width as f64).floor() as i64)*bin_width;
    let ts_end_rounded = ((ts_end /bin_width as f64).ceil() as i64 )*bin_width;
    let broker = bgpkit_broker::BgpkitBroker::new("https://api.broker.bgpkit.com/v1");
    let items = broker.query_all(
        &QueryParams{
            start_ts: Some(ts_start_rounded),
            end_ts: Some(ts_end_rounded),
            data_type: Some("update".to_string()),
            order: SortOrder::ASC,
            page_size: 1000,
            ..Default::default()
        }
    ).unwrap();

    let mut bins = vec![];

    for (key, group) in &items.into_iter().group_by(|item| item.timestamp / bin_width) {
        let files: Vec<String> = group.map(|g| g.url).collect();
        bins.push(DataFileBin{
            ts_start: key*bin_width,
            ts_end: (key+1)*bin_width,
            files
        })
    }
    bins
}

#[tracing::instrument]
pub fn parse_bin(bin: &DataFileBin) -> Vec<BgpElem> {
    let mut heap = BinaryHeap::new();
    bin.files.par_iter().map(|path|{
        info!("parsing file: {}... ", path);
        let parser = bgpkit_parser::BgpkitParser::new(path.as_str()).unwrap();
        let elems = parser.into_elem_iter().collect::<Vec<BgpElem>>();
        info!("parsing file: {}... done", path);
        elems
    }).collect::<Vec<Vec<BgpElem>>>()
        .into_iter().for_each(|elems|{
        for e in elems{
            heap.push(e)
        }
    });
    heap.into_sorted_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bins() {
        let bins = find_bins(1638316800 as f64, 1638318600 as f64, 15*60);
        dbg!(bins);
    }

    #[test]
    fn test_parse_bin() {
        tracing_subscriber::fmt()
            // filter spans/events with level TRACE or higher.
            .with_max_level(Level::INFO)
            .init();
        info!("start test parsing bin.");
        let bins = find_bins(1638316800 as f64, 1638318600 as f64, 15*60);
        let elems = parse_bin(bins.first().unwrap());
        info!("number of total elems: {}", elems.len());
    }
}