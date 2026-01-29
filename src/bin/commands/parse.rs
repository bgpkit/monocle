use std::io::Write;
use std::path::PathBuf;

use bgpkit_parser::encoder::MrtUpdatesEncoder;
use bgpkit_parser::BgpElem;
use clap::Args;

use monocle::lens::parse::{ParseFilters, ParseLens};
use monocle::lens::utils::OutputFormat;

use super::elem_format::{
    available_fields_help, format_elem, format_elems_table, get_header, parse_fields,
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

    match mrt_path {
        None => {
            // For Table format, we need to buffer all elements
            if output_format == OutputFormat::Table {
                let elems: Vec<(BgpElem, Option<String>)> =
                    parser.into_iter().map(|elem| (elem, None)).collect();
                if !elems.is_empty() {
                    println!("{}", format_elems_table(&elems, &fields));
                }
                return;
            }

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
                if let Some(output_str) = format_elem(&elem, output_format, &fields, None) {
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
