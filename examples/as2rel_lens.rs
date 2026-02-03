//! AS Relationships Example
//!
//! Demonstrates querying AS-level relationships (upstream/downstream/peer).
//!
//! # Running
//!
//! ```bash
//! cargo run --example as2rel_lens --features lib
//! ```

use monocle::database::MonocleDatabase;
use monocle::lens::as2rel::{As2relLens, As2relSearchArgs};

fn main() -> anyhow::Result<()> {
    let db = MonocleDatabase::open_in_memory()?;
    let lens = As2relLens::new(&db);

    // Update data if needed
    if lens.needs_update() {
        println!("Updating AS2Rel data...");
        lens.update()?;
    }

    // Search for AS relationships
    println!("\nSearching for AS13335 relationships:");
    let args = As2relSearchArgs::new(13335).with_names().upstream_only();

    let results = lens.search(&args)?;

    println!("Found {} downstream relationships:", results.len());
    for r in results.iter().take(5) {
        println!(
            "  AS{} -> AS{} ({} peers see this)",
            r.asn1, r.asn2, r.connected
        );
    }

    Ok(())
}
