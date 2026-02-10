//! IP Information Example
//!
//! Demonstrates looking up IP address information including ASN and RPKI status.
//!
//! # Running
//!
//! ```bash
//! cargo run --example ip_lens --features lib
//! ```

use monocle::lens::ip::{IpLens, IpLookupArgs};
use std::net::IpAddr;

fn main() -> anyhow::Result<()> {
    let lens = IpLens::new();

    // Look up a specific IP
    let ip = "1.1.1.1".parse::<IpAddr>()?;
    let args = IpLookupArgs::new(ip);
    let info = lens.lookup(&args)?;

    println!("IP: {}", info.ip);
    if let Some(country) = &info.country {
        println!("Location: {}", country);
    }

    if let Some(asn) = &info.asn {
        println!("\nNetwork Information:");
        println!("  ASN: {}", asn.asn);
        println!("  Name: {}", asn.name);
        println!("  Prefix: {}", asn.prefix);
        println!("  RPKI: {}", asn.rpki);
        if let Some(country) = &asn.country {
            println!("  Country: {}", country);
        }
    }

    Ok(())
}
