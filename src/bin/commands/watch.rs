//! Watch command: stream live BGP messages from RIPE RIS Live.
//!
//! Filter semantics match `monocle parse` where the dimensions overlap. Filters
//! are pushed down to the RIS Live subscription (server-side) whenever the API
//! can express them; the remainder are applied client-side. The stream can be
//! recorded to an MRT updates file for offline replay with `monocle parse`.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use bgpkit_parser::encoder::MrtUpdatesEncoder;
use bgpkit_parser::RisLiveClientMessage;
use clap::Args;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use monocle::lens::watch::{parse_live_frame, WatchFilters, RECONNECT_BACKOFF, RIS_LIVE_URL};
use monocle::utils::TimestampFormat;

use super::elem_format::{format_elem, get_header};

/// Arguments for the Watch command
#[derive(Args)]
pub(crate) struct WatchArgs {
    /// RRC host to subscribe to (e.g. rrc00). All active RRCs if omitted.
    #[clap(long, short = 'H', value_name = "HOST")]
    pub host: Option<String>,

    /// Accept an unfiltered full feed. Without this flag, watch refuses to run
    /// when no filter is given: an unfiltered live stream is heavy for both
    /// the client and RIPE's servers.
    #[clap(long)]
    pub all: bool,

    /// Disable automatic reconnection on abnormal disconnects
    #[clap(long)]
    pub no_reconnect: bool,

    /// Record the filtered stream to an MRT updates file for offline replay
    /// (e.g. `monocle parse out.mrt.bz2`)
    #[clap(long, short = 'M', value_name = "PATH")]
    pub record: Option<PathBuf>,

    /// Pretty-print JSON output
    #[clap(long)]
    pub pretty: bool,

    /// Comma-separated list of fields to output
    #[clap(long, short = 'f', value_name = "FIELDS")]
    pub fields: Option<String>,

    /// Timestamp output format (unix or rfc3339)
    #[clap(long, value_enum, default_value = "unix")]
    pub time_format: TimestampFormat,

    /// Live-stream filters (pushed down to RIS Live where supported)
    #[clap(flatten)]
    pub filters: WatchFilters,
}

pub fn run(mut args: WatchArgs, output_format: monocle::utils::OutputFormat) {
    // Open the record writer before entering the async runtime: oneio wraps
    // reqwest::blocking, whose internal tokio runtime must not be created or
    // dropped from within an async context.
    let recorder = match args.record.take() {
        Some(path) => match MrtRecorder::new(path) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("watch: cannot open record file: {e}");
                std::process::exit(1);
            }
        },
        None => None,
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create async runtime: {e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = rt.block_on(run_async(args, recorder, output_format)) {
        eprintln!("watch: {e}");
        std::process::exit(1);
    }
}

async fn run_async(
    args: WatchArgs,
    mut recorder: Option<MrtRecorder>,
    output_format: monocle::utils::OutputFormat,
) -> Result<()> {
    let WatchArgs {
        host,
        all,
        no_reconnect,
        pretty,
        fields,
        time_format,
        filters,
        ..
    } = args;

    // Firehose guard: an unscoped subscription is heavy for both ends.
    if !all && filters.is_empty() && host.is_none() {
        bail!(
            "refusing to open an unfiltered live stream: pass at least one filter \
             (e.g. --origin-asn, --prefix, --peer-asn, --host) or use --all to accept the full feed"
        );
    }

    let (subscribe, report) = filters.to_ris_subscribe(host.as_deref())?;
    let client_filters = filters.compile_client_filters()?;

    // Report pushdown state so the user knows what runs where.
    eprintln!("source: RIPE RIS Live ({RIS_LIVE_URL})");
    if let Some(h) = &report.host {
        eprintln!("  host: {h}");
    }
    if !report.origin_path_patterns.is_empty() {
        eprintln!(
            "  server-side path patterns: {}",
            report.origin_path_patterns.join(", ")
        );
    }
    if !report.prefixes.is_empty() {
        eprintln!("  server-side prefixes: {}", report.prefixes.join(", "));
    }
    if !report.peers.is_empty() {
        eprintln!("  server-side peers: {}", report.peers.join(", "));
    }
    if let Some(req) = &report.require {
        eprintln!("  server-side require: {req}");
    }
    if !client_filters.is_empty() {
        eprintln!("  client-side filters: {}", client_filters.len());
    }
    eprintln!("  live vantage is RIS collectors only, not global visibility");
    eprintln!("press Ctrl-C to stop");

    let fields = super::elem_format::parse_fields(&fields, false)
        .map_err(|e| anyhow!("invalid --fields value: {e}"))?;

    let out_format = if pretty {
        monocle::utils::OutputFormat::JsonPretty
    } else {
        output_format
    };
    let header = get_header(out_format, &fields);
    if let Some(h) = header {
        println!("{h}");
    }

    let url = RIS_LIVE_URL.to_string();

    let mut running = true;
    let mut stats = WatchStats::default();

    loop {
        let connect = connect_async(url.as_str()).await;
        let (ws_stream, _) = match connect {
            Ok(c) => c,
            Err(e) => {
                if running && !no_reconnect {
                    eprintln!(
                        "connection failed ({e}); retrying in {:?}s",
                        RECONNECT_BACKOFF
                    );
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                    continue;
                }
                return Err(anyhow!("connection failed: {e}"));
            }
        };

        eprintln!("connected");
        let (mut write, mut read) = ws_stream.split();

        let sub_msg = subscribe.to_json_string();
        if let Err(e) = write.send(Message::Text(sub_msg.clone().into())).await {
            if running && !no_reconnect {
                eprintln!("subscribe send failed ({e}); reconnecting");
                tokio::time::sleep(RECONNECT_BACKOFF).await;
                continue;
            }
            return Err(anyhow!("subscribe send failed: {e}"));
        }
        eprintln!("subscribed: {sub_msg}");

        let mut stdout = std::io::stdout();
        let mut sig = std::pin::pin!(tokio::signal::ctrl_c());

        loop {
            let msg = tokio::select! {
                m = read.next() => match m {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        eprintln!("stream error: {e}");
                        break;
                    }
                    None => {
                        eprintln!("stream closed by server");
                        break;
                    }
                },
                _ = &mut sig => {
                    running = false;
                    break;
                }
            };

            if msg.is_empty() {
                continue;
            }
            let Ok(text) = msg.to_text() else {
                continue;
            };

            stats.messages += 1;
            let live = parse_live_frame(text)?;
            for elem in live.elems {
                if !filters.matches(&elem, &client_filters) {
                    continue;
                }
                stats.elems += 1;
                if recorder.is_some() {
                    stats.recorded += 1;
                }
                if let Some(rec) = recorder.as_mut() {
                    rec.process(&elem);
                }
                if let Some(line) = format_elem(
                    &elem,
                    out_format,
                    &fields,
                    live.host.as_deref(),
                    time_format,
                ) {
                    if let Err(e) = writeln!(stdout, "{line}") {
                        if e.kind() != std::io::ErrorKind::BrokenPipe {
                            eprintln!("ERROR: {e}");
                        }
                        // Broken pipe (e.g. `| head`): stop cleanly.
                        running = false;
                        break;
                    }
                }
            }

            if !running {
                break;
            }
        }

        // Flush recorder before deciding on reconnect.
        if let Some(rec) = recorder.as_mut() {
            if let Err(e) = rec.flush() {
                eprintln!("record flush failed: {e}");
            }
        }

        eprintln!(
            "disconnected after {} messages; total elements: {}",
            stats.messages, stats.elems
        );

        if !running || no_reconnect {
            break;
        }
        eprintln!("reconnecting in {:?}s", RECONNECT_BACKOFF);
        tokio::time::sleep(RECONNECT_BACKOFF).await;
    }

    if let Some(rec) = recorder.as_mut() {
        rec.finish()?;
        eprintln!("recorded {} elements to {:?}", stats.recorded, rec.path);
    }
    Ok(())
}

#[derive(Default)]
struct WatchStats {
    messages: u64,
    elems: u64,
    recorded: u64,
}

/// Incremental MRT updates recorder.
///
/// `MrtUpdatesEncoder` accumulates elements in memory and exports once, which
/// would make an unbounded live stream grow without bound. The recorder instead
/// flushes the encoder to the output file periodically: each flush writes a
/// self-contained batch of BGP4MP messages, and the concatenation of batches is
/// itself a valid updates MRT stream.
struct MrtRecorder {
    path: PathBuf,
    encoder: MrtUpdatesEncoder,
    writer: Box<dyn Write>,
    count: u64,
}

impl MrtRecorder {
    const FLUSH_INTERVAL: u64 = 500;

    fn new(path: PathBuf) -> Result<Self> {
        let path_str = path
            .to_str()
            .ok_or_else(|| anyhow!("record path is not valid UTF-8"))?;
        let writer =
            oneio::get_writer(path_str).map_err(|e| anyhow!("cannot open record file: {e}"))?;
        Ok(Self {
            path,
            encoder: MrtUpdatesEncoder::new(),
            writer,
            count: 0,
        })
    }

    fn process(&mut self, elem: &bgpkit_parser::BgpElem) {
        self.encoder.process_elem(elem);
        self.count += 1;
        if self.count.is_multiple_of(Self::FLUSH_INTERVAL) {
            if let Err(e) = self.flush() {
                eprintln!("record flush failed: {e}");
            }
        }
    }

    fn flush(&mut self) -> Result<()> {
        let bytes = self
            .encoder
            .export_bytes()
            .map_err(|e| anyhow!("MRT encode failed: {e}"))?;
        if !bytes.is_empty() {
            self.writer
                .write_all(&bytes)
                .map_err(|e| anyhow!("record write failed: {e}"))?;
        }
        self.encoder.reset();
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.flush()?;
        self.writer
            .flush()
            .map_err(|e| anyhow!("record flush failed: {e}"))?;
        Ok(())
    }
}
