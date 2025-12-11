use clap::Args;
use monocle::lens::time::{TimeLens, TimeOutputFormat, TimeParseArgs};

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

    let lens = TimeLens::new();
    let parse_args = TimeParseArgs::new(time);

    let result = if simple {
        lens.parse_to_rfc3339(&parse_args.times)
            .map(|v| v.join("\n"))
    } else {
        lens.parse(&parse_args)
            .map(|results| lens.format_results(&results, &TimeOutputFormat::Table))
    };

    match result {
        Ok(t) => {
            println!("{t}")
        }
        Err(e) => {
            eprintln!("ERROR: {e}")
        }
    };
}
