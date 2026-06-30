//! Database Refresh Performance Benchmark
//!
//! Measures the insert/store performance of each monocle repository using
//! **real data files** when available in `/tmp/monocle-bench/`, falling back
//! to synthetic data of comparable size.
//!
//! Each repository's refresh is broken into phases (download, parse, store,
//! clear) so we can pinpoint where time is spent.
//!
//! # Running
//!
//! ```bash
//! # Fetch real data first (optional — benchmark falls back to synthetic):
//! curl -sL -o /tmp/monocle-bench/asninfo.jsonl \
//!   http://spaces.bgpkit.org/broker/asninfo.jsonl
//! curl -sL -o /tmp/monocle-bench/as2rel.json.bz2 \
//!   https://data.bgpkit.com/as2rel/as2rel-latest.json.bz2
//! curl -sL -o /tmp/monocle-bench/pfx2as.json.bz2 \
//!   https://data.bgpkit.com/pfx2as/pfx2as-latest.json.bz2
//!
//! cargo run --example db_refresh_bench --features lib --release
//! ```

use monocle::database::{MonocleDatabase, RpkiAspaRecord, RpkiRoaRecord};
use std::path::Path;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Benchmark data directory
// ---------------------------------------------------------------------------

const BENCH_DIR: &str = "/tmp/monocle-bench";

fn real_file(name: &str) -> Option<String> {
    let p = format!("{}/{}", BENCH_DIR, name);
    if Path::new(&p).exists() {
        Some(p)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Synthetic fallback sizes
// ---------------------------------------------------------------------------

const ASINFO_ROWS: usize = 120_000;
const AS2REL_ROWS: usize = 900_000;
const RPKI_ROA_ROWS: usize = 300_000;
const RPKI_ASPA_ROWS: usize = 20_000;
const PFX2AS_ROWS: usize = 1_600_000;

fn main() -> anyhow::Result<()> {
    // Suppress tracing so benchmark output is clean.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::ERROR)
        .try_init();

    println!("monocle database refresh performance benchmark");
    println!("===============================================\n");
    println!("data dir: {}", BENCH_DIR);

    let asinfo_file = real_file("asninfo.jsonl");
    let as2rel_file = real_file("as2rel.json.bz2");
    let pfx2as_file = real_file("pfx2as.json.bz2");
    println!(
        "  asninfo.jsonl:      {}",
        asinfo_file.as_deref().unwrap_or("(synthetic fallback)")
    );
    println!(
        "  as2rel.json.bz2:    {}",
        as2rel_file.as_deref().unwrap_or("(synthetic fallback)")
    );
    println!(
        "  pfx2as.json.bz2:    {}",
        pfx2as_file.as_deref().unwrap_or("(synthetic fallback)")
    );
    println!("  rpki:               (synthetic — Cloudflare fetch not included)\n");

    println!(
        "{:<22} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "repository", "rows", "parse_ms", "store_ms", "total_ms", "rows/sec"
    );
    println!("{}", "-".repeat(78));

    bench_asinfo(asinfo_file.as_deref())?;
    bench_as2rel(as2rel_file.as_deref())?;
    bench_rpki()?;
    bench_pfx2as(pfx2as_file.as_deref())?;

    println!();
    bench_queries(pfx2as_file.as_deref())?;

    println!();
    bench_pragma_toggle()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn report(label: &str, rows: usize, parse_ms: u128, store_ms: u128, total_ms: u128) {
    let rps = if total_ms > 0 {
        (rows as f64 / total_ms as f64 * 1000.0) as u64
    } else {
        rows as u64 * 1000
    };
    println!(
        "{:<22} {:>10} {:>10} {:>10} {:>10} {:>12}",
        label, rows, parse_ms, store_ms, total_ms, rps
    );
}

fn ms(elapsed: std::time::Duration) -> u128 {
    elapsed.as_millis()
}

// ---------------------------------------------------------------------------
// ASInfo
// ---------------------------------------------------------------------------

fn bench_asinfo(real: Option<&str>) -> anyhow::Result<()> {
    use monocle::database::{AsinfoSchemaDefinitions, JsonlRecord};

    let db = MonocleDatabase::open_in_memory()?;
    for sql in AsinfoSchemaDefinitions::all_tables() {
        db.connection().execute_batch(sql)?;
    }
    for sql in AsinfoSchemaDefinitions::ASINFO_INDEXES {
        db.connection().execute_batch(sql)?;
    }

    if let Some(path) = real {
        // load_from_path = parse JSONL + store. We cannot easily separate
        // parse from store here without modifying the repo, so we measure
        // the combined path and also time a parse-only pass.
        let t0 = Instant::now();
        let counts = db.asinfo().load_from_path(path)?;
        let total_ms = ms(t0.elapsed());

        // Parse-only timing (read + deserialize, no DB writes)
        let t0 = Instant::now();
        let reader = oneio::get_reader(path)?;
        let buf = std::io::BufReader::new(reader);
        use std::io::BufRead;
        let mut n = 0usize;
        for line in buf.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let _: JsonlRecord = serde_json::from_str(&line)?;
            n += 1;
        }
        let _ = n; // parsed count, unused
        let parse_ms = ms(t0.elapsed());

        report(
            "asinfo(real)",
            counts.core,
            parse_ms,
            total_ms.saturating_sub(parse_ms),
            total_ms,
        );
        Ok(())
    } else {
        // Synthetic JSONL
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("asinfo.jsonl");
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
            for i in 0..ASINFO_ROWS as u32 {
                let mut obj = format!(
                    r#"{{"asn":{},"name":"AS{}","country":"{}""#,
                    1000 + i,
                    1000 + i,
                    if i % 2 == 0 { "US" } else { "DE" }
                );
                if i % 3 == 0 {
                    obj.push_str(&format!(
                        r#","as2org":{{"country":"US","name":"Org{}","org_id":"ORG{}","org_name":"Org Name {}"}}"#,
                        i, i, i
                    ));
                }
                if i % 5 == 0 {
                    obj.push_str(&format!(
                        r#","peeringdb":{{"aka":"aka{}","asn":{},"name":"PDB{}","name_long":"PDB Long {}","website":"https://{}.example","irr_as_set":"AS-SET-{}"}}"#,
                        i, 1000 + i, i, i, i, i
                    ));
                }
                if i % 7 == 0 {
                    obj.push_str(&format!(
                        r#","hegemony":{{"asn":{},"ipv4":{},"ipv6":{}}}"#,
                        1000 + i,
                        0.001 * (i % 100) as f64,
                        0.0005 * (i % 100) as f64
                    ));
                }
                if i % 11 == 0 {
                    obj.push_str(&format!(
                        r#","population":{{"percent_country":{},"percent_global":{},"sample_count":{},"user_count":{}}}"#,
                        0.1 * (i % 50) as f64,
                        0.01 * (i % 50) as f64,
                        100 + i,
                        1000 * (i + 1)
                    ));
                }
                obj.push('}');
                obj.push('\n');
                use std::io::Write;
                f.write_all(obj.as_bytes())?;
            }
        }

        let t0 = Instant::now();
        let counts = db.asinfo().load_from_path(path.to_str().unwrap())?;
        let total_ms = ms(t0.elapsed());

        report("asinfo(synth)", counts.core, 0, total_ms, total_ms);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// AS2Rel
// ---------------------------------------------------------------------------

fn bench_as2rel(real: Option<&str>) -> anyhow::Result<()> {
    use monocle::database::As2relEntry;

    let db = MonocleDatabase::open_in_memory()?;

    let (entries, source_label) = if let Some(path) = real {
        // Parse the real bz2-compressed JSON with oneio (same as production).
        let t0 = Instant::now();
        let entries: Vec<As2relEntry> = oneio::read_json_struct(path)?;
        let parse_ms = ms(t0.elapsed());

        let n = entries.len();

        // Store-only timing using the internal store path. We reuse
        // load_from_path on a re-serialized temp file to exercise the exact
        // production code (clear + PRAGMA toggle + insert + restore), then
        // also do a store-only measurement via a second in-memory db.
        // Simpler: measure load_from_path on the real file (parse+store
        // combined) and subtract the parse_ms we already measured.
        let db2 = MonocleDatabase::open_in_memory()?;
        let t0 = Instant::now();
        let loaded = db2.as2rel().load_from_path(path)?;
        let total_ms = ms(t0.elapsed());
        assert_eq!(loaded, n);

        report(
            "as2rel(real)",
            n,
            parse_ms,
            total_ms.saturating_sub(parse_ms),
            total_ms,
        );

        // Also measure clear cost on the populated db.
        let t0 = Instant::now();
        db2.as2rel().clear()?;
        println!("  (clear after {} rows: {} ms)", n, ms(t0.elapsed()));
        return Ok(());
    } else {
        // Synthetic with unique keys
        let entries: Vec<As2relEntry> = (0..AS2REL_ROWS as u32)
            .map(|i| As2relEntry {
                asn1: 1000 + (i / (AS2REL_ROWS as u32 / 100)),
                asn2: 100000 + i,
                paths_count: 1 + (i % 200),
                peers_count: 1 + (i % 100),
                rel: match i % 3 {
                    0 => 0,
                    1 => 1,
                    _ => -1,
                },
            })
            .collect();
        (entries, "as2rel(synth)")
    };

    // Synthetic path: write to temp file, then load_from_path
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("as2rel.json");
    let t0 = Instant::now();
    let json = serde_json::to_string(&entries)?;
    let parse_ms = ms(t0.elapsed());
    std::fs::write(&path, json)?;

    let t0 = Instant::now();
    let n = db.as2rel().load_from_path(path.to_str().unwrap())?;
    let total_ms = ms(t0.elapsed());

    report(
        source_label,
        n,
        parse_ms,
        total_ms.saturating_sub(parse_ms),
        total_ms,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// RPKI — synthetic data only (real fetch requires Cloudflare API call)
// ---------------------------------------------------------------------------

fn bench_rpki() -> anyhow::Result<()> {
    let db = MonocleDatabase::open_in_memory()?;
    db.rpki().initialize_schema()?;

    let roas: Vec<RpkiRoaRecord> = (0..RPKI_ROA_ROWS as u32)
        .map(|i| RpkiRoaRecord {
            prefix: format!("{}.{}.0/24", (i >> 8) & 0xff, i & 0xff),
            max_length: 24,
            origin_asn: 1000 + (i % 50000),
            ta: if i % 2 == 0 { "apnic" } else { "ripe" }.to_string(),
        })
        .collect();

    let aspas: Vec<RpkiAspaRecord> = (0..RPKI_ASPA_ROWS as u32)
        .map(|i| RpkiAspaRecord {
            customer_asn: 1000 + i,
            provider_asns: vec![2000 + i, 3000 + i],
        })
        .collect();

    let total = roas.len() + aspas.len() * 2;

    let t0 = Instant::now();
    db.rpki().store(&roas, &aspas, "bench", "bench")?;
    let store_ms = ms(t0.elapsed());

    report("rpki(synth)", total, 0, store_ms, store_ms);
    Ok(())
}

// ---------------------------------------------------------------------------
// Pfx2as
// ---------------------------------------------------------------------------

fn bench_pfx2as(real: Option<&str>) -> anyhow::Result<()> {
    use monocle::database::Pfx2asDbRecord;

    let db = MonocleDatabase::open_in_memory()?;
    db.pfx2as().initialize_schema()?;

    if let Some(path) = real {
        // Measure parse separately from store.
        #[derive(serde::Deserialize)]
        struct Pfx2asEntry {
            prefix: String,
            asn: u32,
        }

        let t0 = Instant::now();
        let entries: Vec<Pfx2asEntry> = oneio::read_json_struct(path)?;
        let parse_ms = ms(t0.elapsed());

        let records: Vec<Pfx2asDbRecord> = entries
            .into_iter()
            .filter(|e| !e.prefix.ends_with("/0"))
            .map(|e| Pfx2asDbRecord {
                prefix: e.prefix,
                origin_asn: e.asn,
                validation: "unknown".to_string(),
            })
            .collect();
        let n = records.len();

        let t0 = Instant::now();
        db.pfx2as().store(&records, path)?;
        let store_ms = ms(t0.elapsed());

        report("pfx2as(real)", n, parse_ms, store_ms, parse_ms + store_ms);

        // ---- Optimized variant: drop indexes, insert, recreate indexes ----
        // Tests the hypothesis that per-row index maintenance is the bottleneck.
        let db2 = MonocleDatabase::open_in_memory()?;
        db2.pfx2as().initialize_schema()?;
        // Clear the indexes but keep the table
        let index_names = [
            "idx_pfx2as_prefix_range",
            "idx_pfx2as_origin_asn",
            "idx_pfx2as_prefix_length",
            "idx_pfx2as_prefix_str",
            "idx_pfx2as_validation",
        ];
        db2.connection().execute_batch("PRAGMA synchronous = OFF")?;
        db2.connection()
            .query_row("PRAGMA journal_mode = MEMORY", [], |_| Ok(()))?;
        for idx in &index_names {
            db2.connection()
                .execute_batch(&format!("DROP INDEX IF EXISTS {}", idx))?;
        }
        let t0 = Instant::now();
        db2.connection().execute_batch("BEGIN TRANSACTION")?;
        {
            let mut stmt = db2.connection().prepare(
                "INSERT INTO pfx2as (prefix_start, prefix_end, prefix_length, origin_asn, prefix_str, validation)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for r in &records {
                if let Ok((start, end, len)) = parse_prefix_to_range(&r.prefix) {
                    stmt.execute(rusqlite::params![
                        start.as_slice(),
                        end.as_slice(),
                        len,
                        r.origin_asn,
                        r.prefix,
                        r.validation,
                    ])?;
                }
            }
        }
        db2.connection().execute_batch("COMMIT")?;
        let insert_ms = ms(t0.elapsed());

        // Recreate indexes (SQLite builds them efficiently in one pass)
        let t0 = Instant::now();
        for sql in monocle::database::Pfx2asSchemaDefinitions::PFX2AS_INDEXES {
            db2.connection().execute_batch(sql)?;
        }
        let reindex_ms = ms(t0.elapsed());

        println!(
            "  optimized: insert_wo_indexes={} ms, recreate_indexes={} ms, total={} ms (vs {} ms)",
            insert_ms,
            reindex_ms,
            insert_ms + reindex_ms,
            store_ms
        );
        Ok(())
    } else {
        let records: Vec<Pfx2asDbRecord> = (0..PFX2AS_ROWS as u32)
            .map(|i| Pfx2asDbRecord {
                prefix: format!("{}.{}.0/24", (i >> 8) & 0xff, i & 0xff),
                origin_asn: 1000 + (i % 60000),
                validation: match i % 3 {
                    0 => "valid",
                    1 => "invalid",
                    _ => "unknown",
                }
                .to_string(),
            })
            .collect();

        let t0 = Instant::now();
        db.pfx2as().store(&records, "bench://pfx2as")?;
        let store_ms = ms(t0.elapsed());

        report("pfx2as(synth)", PFX2AS_ROWS, 0, store_ms, store_ms);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Query performance after refresh — confirms the tradeoff is acceptable
// ---------------------------------------------------------------------------

fn bench_queries(pfx2as_file: Option<&str>) -> anyhow::Result<()> {
    use monocle::database::Pfx2asDbRecord;

    let path = match pfx2as_file {
        Some(p) => p,
        None => {
            println!("--- Query benchmark: skipped (no real pfx2as data) ---");
            return Ok(());
        }
    };

    println!("--- Query performance after refresh (pfx2as, 1.6M rows) ---");
    println!("    Target: < 1 second for low-traffic queries\n");

    // Load data once
    #[derive(serde::Deserialize)]
    struct Pfx2asEntry {
        prefix: String,
        asn: u32,
    }
    let entries: Vec<Pfx2asEntry> = oneio::read_json_struct(path)?;
    let records: Vec<Pfx2asDbRecord> = entries
        .into_iter()
        .filter(|e| !e.prefix.ends_with("/0"))
        .map(|e| Pfx2asDbRecord {
            prefix: e.prefix,
            origin_asn: e.asn,
            validation: "unknown".to_string(),
        })
        .collect();

    // --- Scenario A: all 5 indexes (current optimized store) ---
    let db_a = MonocleDatabase::open_in_memory()?;
    db_a.pfx2as().initialize_schema()?;
    db_a.pfx2as().store(&records, path)?;

    // --- Scenario B: only 3 indexes (drop validation + prefix_str) ---
    let db_b = MonocleDatabase::open_in_memory()?;
    db_b.pfx2as().initialize_schema()?;
    // Drop the two low-value indexes
    db_b.connection().execute_batch(
        "DROP INDEX IF EXISTS idx_pfx2as_validation;
         DROP INDEX IF EXISTS idx_pfx2as_prefix_str;",
    )?;
    // Re-store with the reduced index set (store will drop/recreate all
    // indexes, so we need to manually re-store and then drop the two again)
    db_b.pfx2as().store(&records, path)?;
    db_b.connection().execute_batch(
        "DROP INDEX IF EXISTS idx_pfx2as_validation;
         DROP INDEX IF EXISTS idx_pfx2as_prefix_str;",
    )?;

    println!("{:<32} {:>12} {:>12}", "query", "5_idx_ms", "3_idx_ms");
    println!("{}", "-".repeat(58));

    // Query 1: lookup_exact (uses prefix_str index in scenario A, full scan in B)
    let test_prefix = "1.1.1.0/24";
    let t = Instant::now();
    let r_a = db_a.pfx2as().lookup_exact(test_prefix)?;
    let ms_a = ms(t.elapsed());
    let t = Instant::now();
    let r_b = db_b.pfx2as().lookup_exact(test_prefix)?;
    let ms_b = ms(t.elapsed());
    assert_eq!(r_a, r_b);
    println!(
        "{:<32} {:>12} {:>12}",
        "lookup_exact(\"1.1.1.0/24\")", ms_a, ms_b
    );

    // Query 1b: lookup_exact rewritten to use BLOB range index (no prefix_str needed)
    let (bs, be, bl) = parse_prefix_to_range("1.1.1.0/24")?;
    let t = Instant::now();
    let r_blob: Vec<u32> = {
        let mut stmt = db_b.connection().prepare(
            "SELECT DISTINCT origin_asn FROM pfx2as WHERE prefix_start = ?1 AND prefix_end = ?2 AND prefix_length = ?3"
        )?;
        let rows = stmt.query_map(rusqlite::params![bs.as_slice(), be.as_slice(), bl], |r| {
            r.get(0)
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };
    let ms_blob = ms(t.elapsed());
    assert_eq!(r_blob, r_b);
    println!(
        "{:<32} {:>12} {:>12}",
        "  → rewritten with BLOB index", 0, ms_blob
    );

    // Query 2: lookup_longest (uses prefix_range BLOB index — kept in both)
    let t = Instant::now();
    let _ = db_a.pfx2as().lookup_longest("1.1.1.128/32")?;
    let ms_a = ms(t.elapsed());
    let t = Instant::now();
    let _ = db_b.pfx2as().lookup_longest("1.1.1.128/32")?;
    let ms_b = ms(t.elapsed());
    println!(
        "{:<32} {:>12} {:>12}",
        "lookup_longest(\"1.1.1.128/32\")", ms_a, ms_b
    );

    // Query 3: lookup_covering (uses prefix_range BLOB index)
    let t = Instant::now();
    let _ = db_a.pfx2as().lookup_covering("1.1.1.0/24")?;
    let ms_a = ms(t.elapsed());
    let t = Instant::now();
    let _ = db_b.pfx2as().lookup_covering("1.1.1.0/24")?;
    let ms_b = ms(t.elapsed());
    println!(
        "{:<32} {:>12} {:>12}",
        "lookup_covering(\"1.1.1.0/24\")", ms_a, ms_b
    );

    // Query 4: get_by_asn (uses origin_asn index — kept in both)
    let t = Instant::now();
    let r_a = db_a.pfx2as().get_by_asn(13335)?;
    let ms_a = ms(t.elapsed());
    let t = Instant::now();
    let r_b = db_b.pfx2as().get_by_asn(13335)?;
    let ms_b = ms(t.elapsed());
    assert_eq!(r_a.len(), r_b.len());
    println!("{:<32} {:>12} {:>12}", "get_by_asn(13335)", ms_a, ms_b);

    // Query 5: validation_stats (GROUP BY — full scan in both)
    let t = Instant::now();
    let _ = db_a.pfx2as().validation_stats()?;
    let ms_a = ms(t.elapsed());
    let t = Instant::now();
    let _ = db_b.pfx2as().validation_stats()?;
    let ms_b = ms(t.elapsed());
    println!("{:<32} {:>12} {:>12}", "validation_stats()", ms_a, ms_b);

    // Query 6: record_count (COUNT(*) — full scan)
    let t = Instant::now();
    let _ = db_a.pfx2as().record_count()?;
    let ms_a = ms(t.elapsed());
    let t = Instant::now();
    let _ = db_b.pfx2as().record_count()?;
    let ms_b = ms(t.elapsed());
    println!("{:<32} {:>12} {:>12}", "record_count()", ms_a, ms_b);

    // --- Store time comparison with 3 vs 5 indexes ---
    println!();
    println!("--- Store time: 5 indexes vs 3 indexes ---");
    let db_c = MonocleDatabase::open_in_memory()?;
    db_c.pfx2as().initialize_schema()?;
    let t = Instant::now();
    db_c.pfx2as().store(&records, path)?;
    let ms_5 = ms(t.elapsed());

    // For 3-index store, we need to modify the store to skip 2 indexes.
    // Since we can't easily do that without code changes, we measure
    // insert-only + rebuild 3 indexes manually.
    let db_d = MonocleDatabase::open_in_memory()?;
    db_d.pfx2as().initialize_schema()?;
    // Drop all indexes first
    for idx in [
        "idx_pfx2as_prefix_range",
        "idx_pfx2as_origin_asn",
        "idx_pfx2as_prefix_length",
        "idx_pfx2as_prefix_str",
        "idx_pfx2as_validation",
    ] {
        db_d.connection()
            .execute_batch(&format!("DROP INDEX IF EXISTS {}", idx))?;
    }
    // Insert without indexes
    let t = Instant::now();
    db_d.connection().execute_batch("BEGIN TRANSACTION")?;
    {
        let mut stmt = db_d.connection().prepare(
            "INSERT INTO pfx2as (prefix_start, prefix_end, prefix_length, origin_asn, prefix_str, validation)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for r in &records {
            if let Ok((start, end, len)) = parse_prefix_to_range(&r.prefix) {
                stmt.execute(rusqlite::params![
                    start.as_slice(),
                    end.as_slice(),
                    len,
                    r.origin_asn,
                    r.prefix,
                    r.validation,
                ])?;
            }
        }
    }
    db_d.connection().execute_batch("COMMIT")?;
    let insert_ms = ms(t.elapsed());

    // Rebuild only 3 indexes (skip validation + prefix_str)
    let t = Instant::now();
    for sql in [
        "CREATE INDEX IF NOT EXISTS idx_pfx2as_prefix_range ON pfx2as(prefix_start, prefix_end)",
        "CREATE INDEX IF NOT EXISTS idx_pfx2as_origin_asn ON pfx2as(origin_asn)",
        "CREATE INDEX IF NOT EXISTS idx_pfx2as_prefix_length ON pfx2as(prefix_length)",
    ] {
        db_d.connection().execute_batch(sql)?;
    }
    let reindex_3_ms = ms(t.elapsed());
    let ms_3 = insert_ms + reindex_3_ms;

    println!(
        "  5 indexes: {} ms (insert={} + reindex={})",
        ms_5,
        insert_ms,
        ms_5.saturating_sub(insert_ms)
    );
    println!(
        "  3 indexes: {} ms (insert={} + reindex={})",
        ms_3, insert_ms, reindex_3_ms
    );
    println!(
        "  savings:   {} ms ({:.0}%)",
        ms_5.saturating_sub(ms_3),
        (ms_5.saturating_sub(ms_3)) as f64 / ms_5 as f64 * 100.0
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// PRAGMA toggle overhead + connection state analysis
// ---------------------------------------------------------------------------

/// Parse a prefix string into (start_bytes, end_bytes, prefix_length)
fn parse_prefix_to_range(prefix: &str) -> anyhow::Result<([u8; 16], [u8; 16], u8)> {
    let net: ipnet::IpNet = prefix
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid prefix '{}': {}", prefix, e))?;
    let start = ip_to_bytes(net.network());
    let end = ip_to_bytes(net.broadcast());
    Ok((start, end, net.prefix_len()))
}

/// Convert an IP address to 16-byte representation
fn ip_to_bytes(ip: std::net::IpAddr) -> [u8; 16] {
    match ip {
        std::net::IpAddr::V4(v4) => v4.to_ipv6_mapped().octets(),
        std::net::IpAddr::V6(v6) => v6.octets(),
    }
}

fn bench_pragma_toggle() -> anyhow::Result<()> {
    let db = MonocleDatabase::open_in_memory()?;

    println!("--- Connection PRAGMA state after refresh ---");
    println!("    (store() no longer modifies PRAGMAs — connection stays at defaults)\n");

    let jm: String = db
        .connection()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    let sync: i64 = db
        .connection()
        .query_row("PRAGMA synchronous", [], |r| r.get(0))?;
    println!(
        "after open:          journal_mode={}, synchronous={} (0=OFF,1=NORMAL,2=FULL)",
        jm, sync
    );

    // Simulate a store cycle — with the optimization, store() does NOT
    // touch PRAGMAs at all. The connection stays in its configured state.
    use monocle::database::Pfx2asDbRecord;
    let records = vec![Pfx2asDbRecord {
        prefix: "1.1.1.0/24".to_string(),
        origin_asn: 13335,
        validation: "valid".to_string(),
    }];
    db.pfx2as().initialize_schema()?;
    db.pfx2as().store(&records, "pragma-test")?;

    let jm_after: String = db
        .connection()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    let sync_after: i64 = db
        .connection()
        .query_row("PRAGMA synchronous", [], |r| r.get(0))?;
    println!(
        "after store():       journal_mode={}, synchronous={}",
        jm_after, sync_after
    );

    if jm == jm_after && sync == sync_after {
        println!("✓ PRAGMA state preserved across refresh — no post-refresh degradation");
    } else {
        println!("✗ PRAGMA state changed — refresh degrades query performance!");
    }

    Ok(())
}
