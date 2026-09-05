//! Watch command: stream live BGP messages from RIPE RIS Live.
//!
//! Filter semantics match `monocle parse` where the dimensions overlap. The
//! subscription pushes `host`/`prefix`/`peer`/`require` down to RIS Live to
//! reduce traffic, but all element predicates also run client-side with
//! parser semantics (RIS selects whole UPDATE messages; elements expand per
//! prefix). The stream can be recorded to an MRT updates file for offline
//! replay with `monocle parse`.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use bgpkit_parser::encoder::MrtUpdatesEncoder;
use bgpkit_parser::RisLiveClientMessage;
use clap::Args;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use monocle::lens::watch::{
    parse_live_frame, LiveFrame, WatchFilters, RECONNECT_BACKOFF, RIS_LIVE_URL,
};
use monocle::utils::{OutputFormat, TimestampFormat};

use super::elem_format::{format_elem, get_header, parse_fields};

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

pub fn run(mut args: WatchArgs, output_format: OutputFormat) {
    // The tree enables two rustls CryptoProviders (aws-lc-rs via oneio, ring
    // via reqwest); pick ring once so TLS setup cannot fail ambiguously.
    install_crypto_provider();

    // 1. Firehose guard: an unscoped subscription is heavy for both ends.
    if !args.all && args.filters.is_empty() && args.host.is_none() {
        eprintln!(
            "watch: refusing to open an unfiltered live stream: pass at least one filter \
             (e.g. --origin-asn, --prefix, --peer-asn, --host) or use --all to accept the full feed"
        );
        std::process::exit(2);
    }

    // 2. Table output cannot stream (format_elem returns None for it); reject
    //    before touching any file or network.
    if !args.pretty && output_format == OutputFormat::Table {
        eprintln!(
            "watch: --format table is not supported for an unbounded stream; \
             use the default PSV, JSON, or --pretty"
        );
        std::process::exit(2);
    }

    // 3. Validate filters and parse fields before opening the record file, so
    //    an invalid invocation cannot truncate an existing recording.
    let client_filters = match args.filters.compile_client_filters() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("watch: invalid filter: {e}");
            std::process::exit(2);
        }
    };
    let fields = match parse_fields(&args.fields, false) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("watch: {e}");
            std::process::exit(2);
        }
    };

    // 4. Open the record writer before entering the async runtime: oneio wraps
    //    reqwest::blocking, whose internal tokio runtime must not be created
    //    or dropped from within an async context.
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
    if let Err(e) = rt.block_on(run_async(
        args,
        recorder,
        client_filters,
        fields,
        output_format,
    )) {
        eprintln!("watch: {e}");
        std::process::exit(1);
    }
}

fn install_crypto_provider() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let provider = rustls::crypto::ring::default_provider();
        if provider.install_default().is_err() {
            // Another provider was already installed process-wide; that one is used.
        }
    });
}

async fn run_async(
    args: WatchArgs,
    mut recorder: Option<MrtRecorder>,
    client_filters: Vec<bgpkit_parser::parser::filter::Filter>,
    fields: Vec<&'static str>,
    output_format: OutputFormat,
) -> Result<()> {
    let WatchArgs {
        host,
        no_reconnect,
        pretty,
        time_format,
        filters,
        ..
    } = args;

    let (mut subscribe, report) = filters.to_ris_subscribe(host.as_deref())?;
    // Request raw BGP bytes and a subscription acknowledgement: raw parsing
    // preserves all path attributes, and the ack distinguishes an accepted
    // subscription from a silently rejected one.
    subscribe = subscribe.include_raw(true).acknowledge(true);

    eprintln!("source: RIPE RIS Live ({RIS_LIVE_URL})");
    if let Some(h) = &report.host {
        eprintln!("  host: {h}");
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
        eprintln!(
            "  client-side filters: {} (server-side pushdown only reduces traffic)",
            client_filters.len()
        );
    }
    eprintln!("  live vantage is RIS collectors only, not global visibility");
    eprintln!("press Ctrl-C to stop");

    let out_format = if pretty {
        OutputFormat::JsonPretty
    } else {
        output_format
    };
    if let Some(h) = get_header(out_format, &fields) {
        println!("{h}");
    }

    let mut running = true;
    let mut stats = WatchStats::default();

    loop {
        let (ws_stream, _) = match connect_async(RIS_LIVE_URL).await {
            Ok(c) => c,
            Err(e) => {
                if running && !no_reconnect {
                    eprintln!("connection failed ({e}); retrying in {RECONNECT_BACKOFF:?}s");
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
        let mut subscribed_ok = false;

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
            match parse_live_frame(text)? {
                LiveFrame::SubscribeOk => {
                    if !subscribed_ok {
                        eprintln!("subscription acknowledged by server");
                        subscribed_ok = true;
                    }
                }
                LiveFrame::Error(err_text) => {
                    return Err(anyhow!("RIS Live rejected the subscription: {err_text}"));
                }
                LiveFrame::Other => {}
                LiveFrame::Data(live) => {
                    if !subscribed_ok {
                        // Data before an ack still means the subscription works.
                        subscribed_ok = true;
                    }
                    for elem in live.elems {
                        if !filters.matches(&elem, &client_filters) {
                            continue;
                        }
                        stats.elems += 1;
                        if let Some(rec) = recorder.as_mut() {
                            rec.process(&elem)?;
                            stats.recorded += 1;
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
                }
            }

            if !running {
                break;
            }
        }

        // Flush recorder before deciding on reconnect.
        if let Some(rec) = recorder.as_mut() {
            rec.flush()?;
        }

        eprintln!(
            "disconnected after {} messages; total elements: {}",
            stats.messages, stats.elems
        );

        if !running || no_reconnect {
            break;
        }
        eprintln!("reconnecting in {RECONNECT_BACKOFF:?}s");
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
/// would make an unbounded live stream grow without bound. The recorder
/// instead flushes the encoder to the output file periodically: each flush
/// writes a self-contained batch of BGP4MP messages, and the concatenation of
/// batches is itself a valid updates MRT stream. Write failures abort the
/// command: the recording would silently lose batches otherwise.
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

    fn process(&mut self, elem: &bgpkit_parser::BgpElem) -> Result<()> {
        self.encoder.process_elem(elem);
        self.count += 1;
        if self.count.is_multiple_of(Self::FLUSH_INTERVAL) {
            self.flush()?;
        }
        Ok(())
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
