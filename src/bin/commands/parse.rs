use std::io::Write;
use std::path::PathBuf;

use bgpkit_parser::encoder::{MrtRibEncoder, MrtUpdatesEncoder};
use bgpkit_parser::parser::filter::Filterable;
use bgpkit_parser::BgpElem;
use clap::Args;

use monocle::lens::parse::filter_file::{load_prefix_file, merge_prefix_file, FilterFile};
use monocle::lens::parse::text_dump;
use monocle::lens::parse::{MrtType, ParseFilters, ParseLens};
use monocle::utils::{OrderByField, OrderDirection, OutputFormat, TimestampFormat};

use super::elem_format::{
    available_fields_help, format_elem, format_elems_table, get_header, parse_fields, sort_elems,
};

/// Arguments for the Parse command
#[derive(Args)]
pub(crate) struct ParseArgs {
    /// File path to an MRT file or Cisco `sh ip bgp` text dump, local or remote.
    /// Format is auto-detected: binary MRT vs plaintext BGP table dump.
    #[clap(name = "FILE")]
    pub file_path: PathBuf,

    /// Pretty-print JSON output
    #[clap(long)]
    pub pretty: bool,

    /// MRT output file path
    #[clap(long, short = 'M')]
    pub mrt_path: Option<PathBuf>,

    /// MRT output subtype: `rib` for TABLE_DUMP_V2 RIB entries, `updates` for BGP4MP messages.
    /// Default: auto-inferred (`rib` for text dumps, `updates` for MRT files).
    #[clap(long, value_enum, value_name = "TYPE")]
    pub mrt_type: Option<MrtType>,

    /// Comma-separated list of fields to output
    #[clap(long, short = 'f', value_name = "FIELDS", help = available_fields_help())]
    pub fields: Option<String>,

    /// Order output by field (enables buffering)
    #[clap(long, value_enum)]
    pub order_by: Option<OrderByField>,

    /// Order direction (asc or desc, default: asc)
    #[clap(long, value_enum, default_value = "asc")]
    pub order: OrderDirection,

    /// Timestamp output format (unix or rfc3339), applied to all output formats including JSON
    #[clap(long, value_enum, default_value = "unix")]
    pub time_format: TimestampFormat,

    /// Load filters from a JSON file (prefixes, origin_asns, peer_asns, etc.)
    /// Merged with CLI filter flags (AND across dimensions, union within each)
    #[clap(long, value_name = "PATH")]
    pub filter_file: Option<PathBuf>,

    /// Load a newline-delimited list of prefixes from a file
    /// Lines starting with # and blank lines are ignored
    #[clap(long, value_name = "PATH")]
    pub prefix_file: Option<PathBuf>,

    /// Filter by AS path regex string
    #[clap(flatten)]
    pub filters: ParseFilters,
}

pub fn run(args: ParseArgs, output_format: OutputFormat) {
    let ParseArgs {
        file_path,
        pretty,
        mrt_path,
        mrt_type,
        fields: fields_arg,
        order_by,
        order,
        time_format,
        filter_file,
        prefix_file,
        mut filters,
    } = args;

    // Load and merge file-based filters into CLI filters
    if let Some(ref pf) = filter_file {
        if let Err(e) = FilterFile::load(pf).and_then(|ff| ff.merge_into(&mut filters)) {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
    }
    if let Some(ref pf) = prefix_file {
        match load_prefix_file(pf) {
            Ok(prefixes) => merge_prefix_file(prefixes, &mut filters),
            Err(e) => {
                eprintln!("ERROR: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Parse and validate fields (false = parse command, no collector in defaults)
    let fields = match parse_fields(&fields_arg, false) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
    };

    let lens = ParseLens::new();

    if let Err(e) = lens.validate_filters(&filters) {
        eprintln!("ERROR: {e}");
        return;
    }

    let file_path_str = match file_path.to_str() {
        Some(path) => path,
        None => {
            eprintln!("Invalid file path");
            std::process::exit(1);
        }
    };

    // ── Format detection ──────────────────────────────────────────
    let reader = match oneio::get_reader(file_path_str) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to open {}: {}", file_path_str, e);
            std::process::exit(1);
        }
    };

    let (is_text, _peeked) = match text_dump::detect_text_dump(reader) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("Failed to read {}: {}", file_path_str, e);
            std::process::exit(1);
        }
    };

    // Determine MRT type: explicit flag takes precedence, otherwise infer.
    let mrt_type = mrt_type.unwrap_or_else(|| MrtType::infer(is_text));

    // Preserve streaming output for ordinary MRT parsing. Text dumps and MRT
    // exports require materialization for their respective encoders/parsers.
    let output_format = if pretty && output_format == OutputFormat::Json {
        OutputFormat::JsonPretty
    } else {
        output_format
    };
    let needs_buffering = output_format == OutputFormat::Table || order_by.is_some();
    if !is_text && mrt_path.is_none() && !needs_buffering {
        let parser = match lens.create_parser(&filters, file_path_str) {
            Ok(parser) => parser,
            Err(error) => {
                eprintln!("Failed to create parser for {}: {}", file_path_str, error);
                std::process::exit(1);
            }
        };
        let mut stdout = std::io::stdout();
        if let Some(header) = get_header(output_format, &fields) {
            if let Err(error) = writeln!(stdout, "{}", header) {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    eprintln!("ERROR: {error}");
                }
                std::process::exit(1);
            }
        }
        for elem in parser {
            if let Some(output) = format_elem(&elem, output_format, &fields, None, time_format) {
                if let Err(error) = writeln!(stdout, "{}", output) {
                    if error.kind() != std::io::ErrorKind::BrokenPipe {
                        eprintln!("ERROR: {error}");
                    }
                    std::process::exit(1);
                }
            }
        }
        return;
    }

    // ── Parse ─────────────────────────────────────────────────────
    let elems: Vec<(BgpElem, Option<String>)> = if is_text {
        // Re-open the file for full parsing (detect_text_dump consumed the reader)
        let reader = match oneio::get_reader(file_path_str) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to re-open {}: {}", file_path_str, e);
                std::process::exit(1);
            }
        };
        let buf_reader = std::io::BufReader::new(reader);
        let text_filters = match filters.to_filters() {
            Ok(filters) => filters,
            Err(e) => {
                eprintln!("Failed to build text dump filters: {e}");
                std::process::exit(1);
            }
        };
        let timestamp = text_dump::infer_timestamp_from_path(file_path_str).unwrap_or(0.0);
        match text_dump::parse_text_dump_with_timestamp(buf_reader, timestamp) {
            Ok(elems) => elems
                .into_iter()
                .filter(|elem| elem.match_filters(&text_filters))
                .map(|elem| (elem, None))
                .collect(),
            Err(e) => {
                eprintln!("Failed to parse text dump {}: {}", file_path_str, e);
                std::process::exit(1);
            }
        }
    } else {
        // Standard MRT parse: re-open since detect_text_dump consumed the reader
        let parser = match lens.create_parser(&filters, file_path_str) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to create parser for {}: {}", file_path_str, e);
                std::process::exit(1);
            }
        };
        parser.into_iter().map(|elem| (elem, None)).collect()
    };

    if elems.is_empty() {
        return;
    }

    let mut stdout = std::io::stdout();

    // ── MRT output ─────────────────────────────────────────────────
    if let Some(ref mrt_out_path) = mrt_path {
        let mrt_out_str = match mrt_out_path.to_str() {
            Some(s) => s,
            None => {
                eprintln!("Invalid MRT output path");
                std::process::exit(1);
            }
        };

        eprintln!("writing MRT ({mrt_type:?}) to {mrt_out_str}...");

        match mrt_type {
            MrtType::Rib => {
                let mut encoder = MrtRibEncoder::new();
                for (elem, _) in &elems {
                    encoder.process_elem(elem);
                }
                let bytes = encoder.export_bytes();
                match oneio::get_writer(mrt_out_str) {
                    Ok(mut w) => {
                        if let Err(e) = w.write_all(&bytes) {
                            eprintln!("Failed to write MRT data: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to create MRT writer: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            MrtType::Updates => {
                let mut encoder = MrtUpdatesEncoder::new();
                for (elem, _) in &elems {
                    encoder.process_elem(elem);
                }
                let bytes = encoder.export_bytes();
                match oneio::get_writer(mrt_out_str) {
                    Ok(mut w) => {
                        if let Err(e) = w.write_all(&bytes) {
                            eprintln!("Failed to write MRT data: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to create MRT writer: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }

        eprintln!("done. total of {} messages written", elems.len());
        return;
    }

    // ── Text output ────────────────────────────────────────────────

    // Sort if ordering is requested
    let mut elems = elems;
    if let Some(order_field) = order_by {
        sort_elems(&mut elems, order_field, order);
    }

    // Output based on format
    if output_format == OutputFormat::Table {
        println!("{}", format_elems_table(&elems, &fields, time_format));
    } else {
        // Print header for markdown format
        if let Some(header) = get_header(output_format, &fields) {
            if let Err(e) = writeln!(stdout, "{}", header) {
                if e.kind() != std::io::ErrorKind::BrokenPipe {
                    eprintln!("ERROR: {e}");
                }
                std::process::exit(1);
            }
        }

        // Output elements
        for (elem, collector) in &elems {
            if let Some(output_str) = format_elem(
                elem,
                output_format,
                &fields,
                collector.as_deref(),
                time_format,
            ) {
                if let Err(e) = writeln!(stdout, "{}", output_str) {
                    if e.kind() != std::io::ErrorKind::BrokenPipe {
                        eprintln!("ERROR: {e}");
                    }
                    std::process::exit(1);
                }
            }
        }
    }
}
