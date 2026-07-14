use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{anyhow, Result};
use monocle::database::{MonocleDatabase, RibSqliteStore, StoredRibEntry};
use monocle::lens::rib::RibLens;
use monocle::utils::{OutputFormat, TimestampFormat};
use monocle::MonocleConfig;
use serde_json::json;
use tabled::builder::Builder;
use tabled::settings::Style;

use super::elem_format::get_header;

pub use monocle::lens::rib::RibArgs;

const DEFAULT_FIELDS_RIB: &[&str] = &[
    "collector",
    "timestamp",
    "peer_ip",
    "peer_asn",
    "prefix",
    "as_path",
    "origin_asns",
];

pub fn run(config: &MonocleConfig, args: RibArgs, output_format: OutputFormat, no_update: bool) {
    if let Err(error) = run_inner(config, args, output_format, no_update) {
        eprintln!("ERROR: {}", error);
        std::process::exit(1);
    }
}

fn run_inner(
    config: &MonocleConfig,
    args: RibArgs,
    output_format: OutputFormat,
    no_update: bool,
) -> Result<()> {
    let sqlite_path = config.sqlite_path();
    let db = MonocleDatabase::open(&sqlite_path)
        .map_err(|e| anyhow!("Failed to open database '{}': {}", sqlite_path, e))?;
    let lens = RibLens::new(&db, config);

    if args.sqlite_path.is_some() {
        run_sqlite_output(&lens, &args, no_update)
    } else {
        run_stdout(&lens, &args, output_format, no_update)
    }
}

fn run_stdout(
    lens: &RibLens<'_>,
    args: &RibArgs,
    output_format: OutputFormat,
    no_update: bool,
) -> Result<()> {
    let fields = parse_fields(&args.fields)?;
    let stdout = std::io::stdout();
    let mut stdout = BufWriter::new(stdout.lock());

    if output_format == OutputFormat::Table {
        let mut entries = Vec::<StoredRibEntry>::new();
        lens.reconstruct_snapshots(
            args,
            no_update,
            |_rib_ts, state_store, _filtered_updates| {
                state_store.visit_entries(|entry| {
                    entries.push(entry.clone());
                    Ok(())
                })
            },
        )?;

        if !entries.is_empty() {
            writeln!(stdout, "{}", format_entries_table(&entries, &fields))
                .map_err(|e| anyhow!("Failed to write table output: {}", e))?;
        }
        return Ok(());
    }

    let mut header_written = false;
    lens.reconstruct_snapshots(
        args,
        no_update,
        |_rib_ts, state_store, _filtered_updates| {
            if !header_written {
                if let Some(header) = get_header(output_format, &fields) {
                    writeln!(stdout, "{}", header)
                        .map_err(|e| anyhow!("Failed to write output header: {}", e))?;
                }
                header_written = true;
            }

            state_store.visit_entries(|entry| {
                if let Some(line) = format_entry(entry, output_format, &fields) {
                    writeln!(stdout, "{}", line)
                        .map_err(|e| anyhow!("Failed to write reconstructed RIB row: {}", e))?;
                }
                Ok(())
            })
        },
    )?;

    Ok(())
}

fn run_sqlite_output(lens: &RibLens<'_>, args: &RibArgs, no_update: bool) -> Result<()> {
    args.validate()?;
    let output_path = args
        .sqlite_path
        .as_deref()
        .ok_or_else(|| anyhow!("Missing --sqlite-path for SQLite output"))?;

    remove_existing_file(output_path)?;

    let mut sqlite_store = RibSqliteStore::new(path_to_str(output_path)?, true)?;
    let summary =
        lens.reconstruct_snapshots(args, no_update, |rib_ts, state_store, filtered_updates| {
            sqlite_store.insert_snapshot(rib_ts, state_store, filtered_updates)
        })?;
    sqlite_store.finalize_indexes()?;

    eprintln!(
        "wrote {} reconstructed RIB snapshot(s) to {}",
        summary.rib_ts.len(),
        output_path.display()
    );
    Ok(())
}

fn parse_fields(fields_arg: &Option<String>) -> Result<Vec<&'static str>> {
    match fields_arg {
        None => Ok(DEFAULT_FIELDS_RIB.to_vec()),
        Some(fields_arg) => {
            let fields = fields_arg
                .split(',')
                .map(str::trim)
                .filter(|field| !field.is_empty())
                .map(|field| {
                    DEFAULT_FIELDS_RIB
                        .iter()
                        .copied()
                        .find(|available| *available == field)
                        .ok_or_else(|| {
                            anyhow!(
                                "Unknown RIB output field '{}'. Available fields: {}",
                                field,
                                DEFAULT_FIELDS_RIB.join(", ")
                            )
                        })
                })
                .collect::<Result<Vec<_>>>()?;
            if fields.is_empty() {
                return Err(anyhow!("--fields must name at least one output field"));
            }
            Ok(fields)
        }
    }
}

fn format_entries_table(entries: &[StoredRibEntry], fields: &[&str]) -> String {
    let mut builder = Builder::default();
    builder.push_record(fields.iter().copied());

    for entry in entries {
        let row = fields
            .iter()
            .map(|field| entry_field_value(entry, field))
            .collect::<Vec<_>>();
        builder.push_record(row);
    }

    let mut table = builder.build();
    table.with(Style::rounded());
    table.to_string()
}

fn format_entry(
    entry: &StoredRibEntry,
    output_format: OutputFormat,
    fields: &[&str],
) -> Option<String> {
    match output_format {
        OutputFormat::Json | OutputFormat::JsonLine => {
            Some(serde_json::to_string(&build_json_object(entry, fields)).unwrap_or_default())
        }
        OutputFormat::JsonPretty => Some(
            serde_json::to_string_pretty(&build_json_object(entry, fields)).unwrap_or_default(),
        ),
        OutputFormat::Psv => Some(
            fields
                .iter()
                .map(|field| entry_field_value(entry, field))
                .collect::<Vec<_>>()
                .join("|"),
        ),
        OutputFormat::Table => None,
        OutputFormat::Markdown => Some(format!(
            "| {} |",
            fields
                .iter()
                .map(|field| entry_field_value(entry, field))
                .collect::<Vec<_>>()
                .join(" | ")
        )),
    }
}

fn build_json_object(entry: &StoredRibEntry, fields: &[&str]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();

    for field in fields {
        let value = match *field {
            "collector" => json!(entry.collector.to_string()),
            "timestamp" => json!(entry.timestamp),
            "peer_ip" => json!(entry.peer_ip.to_string()),
            "peer_asn" => json!(entry.peer_asn),
            "prefix" => json!(entry.prefix.to_string()),
            "as_path" => entry
                .as_path
                .as_ref()
                .map_or(serde_json::Value::Null, |value| json!(value)),
            "origin_asns" => entry
                .origin_asns
                .as_ref()
                .map_or(serde_json::Value::Null, |values| {
                    json!(values.iter().map(u32::to_string).collect::<Vec<_>>())
                }),
            _ => serde_json::Value::Null,
        };

        obj.insert((*field).to_string(), value);
    }

    serde_json::Value::Object(obj)
}

fn entry_field_value(entry: &StoredRibEntry, field: &str) -> String {
    match field {
        "collector" => entry.collector.to_string(),
        "timestamp" => TimestampFormat::Unix.format_timestamp(entry.timestamp),
        "peer_ip" => entry.peer_ip.to_string(),
        "peer_asn" => entry.peer_asn.to_string(),
        "prefix" => entry.prefix.to_string(),
        "as_path" => entry.as_path.clone().unwrap_or_default(),
        "origin_asns" => entry.origin_asns_string().unwrap_or_default(),
        _ => String::new(),
    }
}

fn remove_existing_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow!(
            "Failed to remove existing output file '{}': {}",
            path.display(),
            error
        )),
    }
}

fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow!("Path '{}' contains invalid UTF-8", path.display()))
}
