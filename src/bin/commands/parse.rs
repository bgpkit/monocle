use std::io::Write;
use std::path::PathBuf;

use bgpkit_parser::encoder::MrtUpdatesEncoder;
use clap::Args;

use monocle::lens::parse::{ParseFilters, ParseLens};

use crate::elem_to_string;

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

    /// Filter by AS path regex string
    #[clap(flatten)]
    pub filters: ParseFilters,
}

pub fn run(args: ParseArgs, json: bool) {
    let ParseArgs {
        file_path,
        pretty,
        mrt_path,
        filters,
    } = args;

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

    match mrt_path {
        None => {
            for elem in parser {
                // output to stdout
                let output_str = match elem_to_string(&elem, json, pretty, "") {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Failed to format element: {}", e);
                        continue;
                    }
                };
                if let Err(e) = writeln!(stdout, "{}", &output_str) {
                    if e.kind() != std::io::ErrorKind::BrokenPipe {
                        eprintln!("ERROR: {e}");
                    }
                    std::process::exit(1);
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
            if !json {
                eprintln!("processing. filtered messages output to {}...", &path);
            }
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
            if !json {
                eprintln!("done. total of {} message wrote", total_count);
            }
        }
    }
}
