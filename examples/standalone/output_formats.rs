//! Output Formats Example
//!
//! Demonstrates the unified OutputFormat type used across monocle commands.
//!
//! # Running
//!
//! ```bash
//! cargo run --example output_formats --features lib
//! ```

use monocle::lens::utils::OutputFormat;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExampleRecord {
    id: u32,
    name: String,
    value: f64,
}

fn main() -> anyhow::Result<()> {
    let records = vec![
        ExampleRecord {
            id: 1,
            name: "Cloudflare".to_string(),
            value: 99.9,
        },
        ExampleRecord {
            id: 2,
            name: "Google".to_string(),
            value: 98.5,
        },
    ];

    // Available formats
    println!("Available formats: {:?}", OutputFormat::all_names());

    // Parse from string
    let format = OutputFormat::from_str("json").map_err(|e| anyhow::anyhow!("{}", e))?;
    println!("\nParsed format: {:?}", format);

    // Format data
    println!("\nJSON output:");
    println!("{}", serde_json::to_string(&records)?);

    println!("\nPretty JSON:");
    println!("{}", serde_json::to_string_pretty(&records)?);

    Ok(())
}
