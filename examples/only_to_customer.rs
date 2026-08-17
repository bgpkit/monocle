//! Only-to-Customer (RFC 9234) filters and display example
//!
//! Demonstrates filtering BGP update data by the only-to-customer attribute:
//! a concrete ASN value, `*` (attribute present), and `!*` (attribute absent),
//! via [`ParseFilters::only_to_customer`].
//!
//! # Running
//!
//! ```bash
//! cargo run --example only_to_customer --features lib
//! ```
//!
//! The equivalent CLI (same real data, Route Views route server peer AS37100
//! propagating OTC values on AS12654 announcements):
//!
//! ```bash
//! monocle parse \
//!   http://archive.routeviews.org/bgpdata/2026.08/UPDATES/updates.20260816.1200.bz2 \
//!   --only-to-customer 6777 --fields timestamp,prefix,as_path,only-to-customer
//! ```

use monocle::lens::parse::{ParseFilters, ParseLens};

fn main() -> anyhow::Result<()> {
    let lens = ParseLens::new();

    // Route Views update file with real RFC 9234 OTC data: peer AS37100 (a
    // route-server or provider-facing peer) tags AS12654 announcements with
    // only-to-customer values such as 6777 and 8714.
    let url = "http://archive.routeviews.org/bgpdata/2026.08/UPDATES/updates.20260816.1200.bz2";

    // 1. Filter by a concrete only-to-customer ASN value.
    let value_filters = ParseFilters {
        only_to_customer: Some("6777".to_string()),
        ..Default::default()
    };
    let elems = lens.parse_with_progress(&value_filters, url, None)?;
    println!("Elements with only-to-customer = 6777: {}", elems.len());
    for elem in elems.iter().take(3) {
        let otc = elem
            .only_to_customer
            .map(|asn| asn.to_string())
            .unwrap_or_default();
        println!("  {} {} via {otc}", elem.timestamp, elem.prefix);
    }

    // 2. Presence wildcard: any element that carries an only-to-customer value.
    let presence_filters = ParseFilters {
        only_to_customer: Some("*".to_string()),
        ..Default::default()
    };
    let present = lens.parse_with_progress(&presence_filters, url, None)?;
    println!(
        "Elements carrying an only-to-customer value: {}",
        present.len()
    );

    // 3. Absence wildcard: elements without the only-to-customer attribute.
    let absence_filters = ParseFilters {
        only_to_customer: Some("!*".to_string()),
        ..Default::default()
    };
    let absent = lens.parse_with_progress(&absence_filters, url, None)?;
    println!(
        "Elements without an only-to-customer value: {}",
        absent.len()
    );

    Ok(())
}
