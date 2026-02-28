use std::io::Write;
use std::path::PathBuf;

use bgpkit_parser::encoder::MrtUpdatesEncoder;
use bgpkit_parser::BgpElem;
use clap::Args;

use monocle::lens::parse::{ParseFilters, ParseLens};
use monocle::utils::{OrderByField, OrderDirection, OutputFormat, TimestampFormat};

use super::elem_format::{
    available_fields_help, format_elem, format_elems_table, get_header, parse_fields, sort_elems,
};

/// Arguments for the Parse command
#[derive(Args)]
pub(crate) struct ParseArgs {
    /// File path to an MRT file, local or remote.
    #[clap(name = "FILE")]
    pub file_path: PathBuf,

    /// Pretty-print JSON output
    #[clap(long)]
    pub pretty: bool,

    /// MRT output file path
    #[clap(long, short = 'M')]
    pub mrt_path: Option<PathBuf>,

    /// Comma-separated list of fields to output
    #[clap(long, short = 'f', value_name = "FIELDS", help = available_fields_help())]
    pub fields: Option<String>,

    /// Order output by field (enables buffering)
    #[clap(long, value_enum)]
    pub order_by: Option<OrderByField>,

    /// Order direction (asc or desc, default: asc)
    #[clap(long, value_enum, default_value = "asc")]
    pub order: OrderDirection,

    /// Timestamp output format for non-JSON output (unix or rfc3339)
    #[clap(long, value_enum, default_value = "unix")]
    pub time_format: TimestampFormat,

    /// Filter by AS path regex string
    #[clap(flatten)]
    pub filters: ParseFilters,
}

pub fn run(args: ParseArgs, output_format: OutputFormat) {
    let ParseArgs {
        file_path,
        pretty,
        mrt_path,
        fields: fields_arg,
        order_by,
        order,
        time_format,
        filters,
    } = args;

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

    let file_path = match file_path.to_str() {
        Some(path) => path,
        None => {
            eprintln!("Invalid file path");
            std::process::exit(1);
        }
    };
    let parser = match lens.create_parser(&filters, file_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to create parser for {}: {}", file_path, e);
            std::process::exit(1);
        }
    };

    let mut stdout = std::io::stdout();

    // Adjust output format for pretty flag
    let output_format = if pretty && output_format == OutputFormat::Json {
        OutputFormat::JsonPretty
    } else {
        output_format
    };

    // Determine if we need to buffer (for Table format or when ordering is requested)
    let needs_buffering = output_format == OutputFormat::Table || order_by.is_some();

    match mrt_path {
        None => {
            // If buffering is needed (Table format or ordering requested), collect and sort
            if needs_buffering {
                let mut elems: Vec<(BgpElem, Option<String>)> =
                    parser.into_iter().map(|elem| (elem, None)).collect();

                // Sort if ordering is requested
                if let Some(order_field) = order_by {
                    sort_elems(&mut elems, order_field, order);
                }

                if elems.is_empty() {
                    return;
                }

                // Output based on format
                if output_format == OutputFormat::Table {
                    println!("{}", format_elems_table(&elems, &fields, time_format));
                } else {
                    // Print header for markdown format
                    if let Some(header) = get_header(output_format, &fields) {
                        if let Err(e) = writeln!(stdout, "{}", &header) {
                            if e.kind() != std::io::ErrorKind::BrokenPipe {
                                eprintln!("ERROR: {e}");
                            }
                            std::process::exit(1);
                        }
                    }

                    // Output sorted elements
                    for (elem, collector) in &elems {
                        if let Some(output_str) = format_elem(
                            elem,
                            output_format,
                            &fields,
                            collector.as_deref(),
                            time_format,
                        ) {
                            if let Err(e) = writeln!(stdout, "{}", &output_str) {
                                if e.kind() != std::io::ErrorKind::BrokenPipe {
                                    eprintln!("ERROR: {e}");
                                }
                                std::process::exit(1);
                            }
                        }
                    }
                }
                return;
            }

            // Streaming output (no buffering needed)
            // Print header for markdown format before first element
            if let Some(header) = get_header(output_format, &fields) {
                if let Err(e) = writeln!(stdout, "{}", &header) {
                    if e.kind() != std::io::ErrorKind::BrokenPipe {
                        eprintln!("ERROR: {e}");
                    }
                    std::process::exit(1);
                }
            }

            for elem in parser {
                // output to stdout based on format
                if let Some(output_str) =
                    format_elem(&elem, output_format, &fields, None, time_format)
                {
                    if let Err(e) = writeln!(stdout, "{}", &output_str) {
                        if e.kind() != std::io::ErrorKind::BrokenPipe {
                            eprintln!("ERROR: {e}");
                        }
                        std::process::exit(1);
                    }
                }
            }
        }
        Some(p) => {
            let path = match p.to_str() {
                Some(path) => path.to_string(),
                None => {
                    eprintln!("Invalid MRT path");
                    std::process::exit(1);
                }
            };
            eprintln!("processing. filtered messages output to {}...", &path);
            let mut encoder = MrtUpdatesEncoder::new();
            let mut writer = match oneio::get_writer(&path) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("ERROR: {e}");
                    std::process::exit(1);
                }
            };
            let mut total_count = 0;
            for elem in parser {
                total_count += 1;
                encoder.process_elem(&elem);
            }
            if let Err(e) = writer.write_all(&encoder.export_bytes()) {
                eprintln!("Failed to write MRT data: {}", e);
            }
            drop(writer);
            eprintln!("done. total of {} message wrote", total_count);
        }
    }
}
