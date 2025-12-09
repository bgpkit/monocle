use clap::Args;
use monocle::{parse_time_string_to_rfc3339, time_to_table};

/// Arguments for the Time command
#[derive(Args)]
pub struct TimeArgs {
    /// Time stamp or time string to convert
    #[clap()]
    pub time: Vec<String>,

    /// Simple output, only print the converted time
    #[clap(short, long)]
    pub simple: bool,
}

pub fn run(args: TimeArgs) {
    let TimeArgs { time, simple } = args;

    let timestring_res = match simple {
        true => parse_time_string_to_rfc3339(&time),
        false => time_to_table(&time),
    };
    match timestring_res {
        Ok(t) => {
            println!("{t}")
        }
        Err(e) => {
            eprintln!("ERROR: {e}")
        }
    };
}
