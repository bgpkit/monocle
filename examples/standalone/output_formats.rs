//! Output Formats Example
//!
//! This example demonstrates the unified OutputFormat type and how to work
//! with different output formats in monocle.
//!
//! # Feature Requirements
//!
//! This example only requires the `lens-core` feature, which has minimal
//! dependencies.
//!
//! # Running
//!
//! ```bash
//! cargo run --example output_formats --features lens-core
//! ```

use monocle::lens::utils::OutputFormat;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Example data structure that can be formatted in different ways
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExampleRecord {
    id: u32,
    name: String,
    value: f64,
    active: bool,
}

fn main() -> anyhow::Result<()> {
    println!("=== Monocle Output Formats Example ===\n");

    // Example 1: List all available format names
    println!("1. Available output formats:");
    for name in OutputFormat::all_names() {
        println!("   - {}", name);
    }

    // Example 2: Parse format from string
    println!("\n2. Parsing format names:");
    let format_strings = [
        "table",
        "markdown",
        "md",
        "json",
        "json-pretty",
        "jsonpretty",
        "json-line",
        "jsonl",
        "ndjson",
        "psv",
        "pipe",
    ];

    for s in &format_strings {
        match OutputFormat::from_str(s) {
            Ok(fmt) => println!("   '{}' -> {:?}", s, fmt),
            Err(e) => println!("   '{}' -> Error: {}", s, e),
        }
    }

    // Example 3: Check format type
    println!("\n3. Format type checking:");
    let formats = [
        OutputFormat::Table,
        OutputFormat::Markdown,
        OutputFormat::Json,
        OutputFormat::JsonPretty,
        OutputFormat::JsonLine,
        OutputFormat::Psv,
    ];

    for fmt in &formats {
        println!(
            "   {:?}: is_json={}, is_table={}",
            fmt,
            fmt.is_json(),
            fmt.is_table()
        );
    }

    // Example 4: Display format names
    println!("\n4. Format display names:");
    for fmt in &formats {
        println!("   {:?} displays as '{}'", fmt, fmt);
    }

    // Example 5: Using formats with data
    println!("\n5. Formatting example data:");

    let records = vec![
        ExampleRecord {
            id: 1,
            name: "Cloudflare".to_string(),
            value: 99.9,
            active: true,
        },
        ExampleRecord {
            id: 2,
            name: "Google".to_string(),
            value: 98.5,
            active: true,
        },
        ExampleRecord {
            id: 3,
            name: "Example".to_string(),
            value: 50.0,
            active: false,
        },
    ];

    // JSON format
    println!("   JSON format:");
    let json = serde_json::to_string(&records)?;
    println!("   {}", json);

    // JSON Pretty format
    println!("\n   JSON Pretty format:");
    let json_pretty = serde_json::to_string_pretty(&records)?;
    println!("{}", json_pretty);

    // JSON Lines format
    println!("   JSON Lines format:");
    for record in &records {
        println!("   {}", serde_json::to_string(record)?);
    }

    // PSV (Pipe-Separated Values) format
    println!("\n   PSV format:");
    println!("   id|name|value|active");
    for record in &records {
        println!(
            "   {}|{}|{}|{}",
            record.id, record.name, record.value, record.active
        );
    }

    // Example 6: Default format
    println!("\n6. Default format:");
    let default_fmt = OutputFormat::default();
    println!("   Default format is: {:?}", default_fmt);

    // Example 7: Pattern matching on formats
    println!("\n7. Pattern matching for format-specific logic:");
    for fmt in &formats {
        let description = match fmt {
            OutputFormat::Table => "Pretty table with borders - great for terminal output",
            OutputFormat::Markdown => "Markdown table - great for documentation",
            OutputFormat::Json => "Compact JSON - great for piping to jq",
            OutputFormat::JsonPretty => "Pretty JSON - great for human reading",
            OutputFormat::JsonLine => "JSON Lines - great for streaming/logs",
            OutputFormat::Psv => "Pipe-separated - great for simple parsing",
        };
        println!("   {:?}: {}", fmt, description);
    }

    // Example 8: Error handling for invalid formats
    println!("\n8. Error handling for invalid format:");
    match OutputFormat::from_str("invalid_format") {
        Ok(_) => println!("   Unexpectedly succeeded"),
        Err(e) => println!("   Error (expected): {}", e),
    }

    println!("\n=== Example Complete ===");
    Ok(())
}
