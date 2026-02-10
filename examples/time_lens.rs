//! Time Parsing Example
//!
//! Demonstrates parsing timestamps from various formats and converting between them.
//!
//! # Running
//!
//! ```bash
//! cargo run --example time_parsing --features lib
//! ```

use monocle::lens::time::{TimeLens, TimeOutputFormat, TimeParseArgs};

fn main() -> anyhow::Result<()> {
    let lens = TimeLens::new();

    // Parse various time formats
    let args = TimeParseArgs::new(vec![
        "1697043600".to_string(),           // Unix timestamp
        "2023-10-11T15:00:00Z".to_string(), // RFC3339
        "October 11, 2023".to_string(),     // Human-readable
    ]);

    let results = lens.parse(&args)?;

    // Display results in different formats
    println!("Parsed Times:");
    println!(
        "{}",
        lens.format_results(&results, &TimeOutputFormat::Table)
    );

    // Convert to JSON for API usage
    println!("\nJSON Output:");
    println!("{}", lens.format_json(&results, false));

    Ok(())
}
