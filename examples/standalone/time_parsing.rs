//! Time Parsing Example
//!
//! This example demonstrates using TimeLens for time parsing and conversion.
//!
//! # Feature Requirements
//!
//! This example only requires the `lens-core` feature, which has minimal
//! dependencies (chrono, chrono-humanize, dateparser).
//!
//! # Running
//!
//! ```bash
//! cargo run --example time_parsing --features lens-core
//! ```

use monocle::lens::time::{TimeBgpTime, TimeLens, TimeOutputFormat, TimeParseArgs};

fn main() -> anyhow::Result<()> {
    println!("=== Monocle Time Parsing Example ===\n");

    let lens = TimeLens::new();

    // Example 1: Parse current time
    println!("1. Current time:");
    let args = TimeParseArgs::now();
    let results = lens.parse(&args)?;
    print_results(&results);

    // Example 2: Parse Unix timestamp
    println!("\n2. Parse Unix timestamp (1697043600):");
    let args = TimeParseArgs::new(vec!["1697043600".to_string()]);
    let results = lens.parse(&args)?;
    print_results(&results);

    // Example 3: Parse RFC3339 string
    println!("\n3. Parse RFC3339 string:");
    let args = TimeParseArgs::new(vec!["2023-10-11T15:00:00Z".to_string()]);
    let results = lens.parse(&args)?;
    print_results(&results);

    // Example 4: Parse human-readable date
    println!("\n4. Parse human-readable date:");
    let args = TimeParseArgs::new(vec!["October 11, 2023".to_string()]);
    let results = lens.parse(&args)?;
    print_results(&results);

    // Example 5: Parse multiple times at once
    println!("\n5. Parse multiple times:");
    let args = TimeParseArgs::new(vec![
        "1697043600".to_string(),
        "2024-01-01T00:00:00Z".to_string(),
        "January 15, 2024".to_string(),
    ]);
    let results = lens.parse(&args)?;
    print_results(&results);

    // Example 6: Using different output formats
    println!("\n6. Different output formats:");

    let args = TimeParseArgs::new(vec!["2024-06-15T12:30:00Z".to_string()]);
    let results = lens.parse(&args)?;

    println!("   RFC3339 format:");
    println!(
        "   {}",
        lens.format_results(&results, &TimeOutputFormat::Rfc3339)
    );

    println!("   Unix timestamp format:");
    println!(
        "   {}",
        lens.format_results(&results, &TimeOutputFormat::Unix)
    );

    println!("   JSON format:");
    println!("{}", lens.format_results(&results, &TimeOutputFormat::Json));

    // Example 7: Direct time string parsing
    println!("\n7. Direct time string parsing:");
    let dt = lens.parse_time_string("2024-03-14T09:26:53Z")?;
    println!("   Parsed DateTime<Utc>: {}", dt);
    println!("   Unix timestamp: {}", dt.timestamp());

    // Example 8: Convert to RFC3339 strings
    println!("\n8. Batch convert to RFC3339:");
    let times = vec![
        "1700000000".to_string(),
        "1710000000".to_string(),
        "1720000000".to_string(),
    ];
    let rfc3339_strings = lens.parse_to_rfc3339(&times)?;
    for (orig, converted) in times.iter().zip(rfc3339_strings.iter()) {
        println!("   {} -> {}", orig, converted);
    }

    // Example 9: Using format_json for API responses
    println!("\n9. JSON output for API integration:");
    let args = TimeParseArgs::new(vec!["2024-06-15T12:30:00Z".to_string()]);
    let results = lens.parse(&args)?;

    println!("   Compact JSON:");
    println!("   {}", lens.format_json(&results, false));

    println!("   Pretty JSON:");
    println!("{}", lens.format_json(&results, true));

    println!("\n=== Example Complete ===");
    Ok(())
}

fn print_results(results: &[TimeBgpTime]) {
    for t in results {
        println!("   Unix:    {}", t.unix);
        println!("   RFC3339: {}", t.rfc3339);
        println!("   Human:   {}", t.human);
        if results.len() > 1 {
            println!("   ---");
        }
    }
}
