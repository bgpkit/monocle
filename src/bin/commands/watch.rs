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

    /// Drink from the full unfiltered stream (~5k msgs/s). Required only
    /// when no filter is given at all; filters without a server-side scope
    /// (host/prefix/peer) still receive the full feed but are applied
    /// client-side.
    #[clap(long)]
    pub firehose: bool,

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

    // Table output cannot stream (format_elem returns None for it); reject
    // before touching any file or network.
    if !args.pretty && output_format == OutputFormat::Table {
        eprintln!(
            "watch: --format table is not supported for an unbounded stream; \
             use the default PSV, JSON, or --pretty"
        );
        std::process::exit(2);
    }

    // Validate filters and parse fields before opening the record file, so
    // an invalid invocation cannot truncate an existing recording.
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

    // Firehose guard: measured against the live feed, the full stream is
    // ~5.2k messages/s and a steady ~28 MB RSS, so client-side filtering is
    // viable. Any filter dimension is therefore allowed without --firehose;
    // only a completely bare invocation (which most users hit by accident)
    // still requires the explicit opt-in. Server-side scope (--host/--prefix/
    // --peer-ip) remains worthwhile to cut bandwidth and RIPE-side load.
    let plan = match args.filters.to_subscription_plan(args.host.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("watch: {e}");
            std::process::exit(2);
        }
    };
    if !args.firehose && args.filters.is_empty() && args.host.is_none() {
        eprintln!(
            "watch: no filter given; pass at least one filter (e.g. --origin-asn, --prefix, \
             --peer-asn, or --host to also cut server-side traffic) or pass --firehose to \
             drink the full stream (~5k msgs/s)"
        );
        std::process::exit(2);
    }

    // Open the record writer before entering the async runtime: oneio wraps
    // reqwest::blocking, whose internal tokio runtime must not be created
    // or dropped from within an async context.
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
        plan,
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
    plan: monocle::lens::watch::SubscriptionPlan,
    mut recorder: Option<MrtRecorder>,
    client_filters: Vec<bgpkit_parser::parser::filter::Filter>,
    fields: Vec<&'static str>,
    output_format: OutputFormat,
) -> Result<()> {
    let WatchArgs {
        no_reconnect,
        pretty,
        time_format,
        filters,
        ..
    } = args;

    // Request raw BGP bytes and a subscription acknowledgement: raw parsing
    // preserves all path attributes, and the ack distinguishes an accepted
    // subscription from a silently rejected one.
    let subscriptions: Vec<_> = plan
        .subscriptions
        .into_iter()
        .map(|s| s.include_raw(true).acknowledge(true))
        .collect();

    eprintln!("source: RIPE RIS Live ({RIS_LIVE_URL})");
    if let Some(h) = &plan.report.host {
        eprintln!("  host: {h}");
    }
    if !plan.report.prefixes.is_empty() {
        eprintln!(
            "  server-side prefixes: {}",
            plan.report.prefixes.join(", ")
        );
    }
    if !plan.report.peers.is_empty() {
        eprintln!("  server-side peers: {}", plan.report.peers.join(", "));
    }
    if let Some(req) = &plan.report.require {
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

    // Match parse: --pretty only upgrades compact JSON to pretty JSON.
    let out_format = if pretty && output_format == OutputFormat::Json {
        OutputFormat::JsonPretty
    } else {
        output_format
    };
    // Write the header through Write with BrokenPipe handling: println!
    // panics on a broken pipe (e.g. `watch --format markdown | head`).
    {
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        if let Some(h) = get_header(out_format, &fields) {
            if let Err(e) = writeln!(lock, "{h}") {
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    // The reader is already gone (e.g. `| head` exited before
                    // any element): nothing was streamed or recorded, just
                    // finalize (flush) the empty recorder and stop.
                    if let Some(rec) = recorder.as_mut() {
                        rec.finish()?;
                    }
                    return Ok(());
                }
                return Err(anyhow!("stdout write failed: {e}"));
            }
        }
        let _ = lock.flush();
    }

    let mut running = true;
    let mut stats = WatchStats::default();
    // One signal future for the whole session: once created, Tokio keeps
    // SIGINT registered process-wide, so Ctrl-C must be awaited during
    // connect, subscribe send, and backoff too, not only while reading.
    let mut sig = std::pin::pin!(tokio::signal::ctrl_c());

    // Errors inside the session set `fatal` and break out to the recorder
    // finalization below, so accepted elements are never lost from the
    // recorder buffer on an error path.
    let mut fatal: Option<anyhow::Error> = None;

    'session: loop {
        let (ws_stream, _) = tokio::select! {
            c = connect_async(RIS_LIVE_URL) => match c {
                Ok(c) => c,
                Err(e) => {
                    if running && !no_reconnect {
                        eprintln!("connection failed ({e}); retrying in {RECONNECT_BACKOFF:?}s");
                        tokio::select! {
                            _ = &mut sig => running = false,
                            _ = tokio::time::sleep(RECONNECT_BACKOFF) => {}
                        }
                        if running {
                            continue 'session;
                        }
                        break;
                    }
                    fatal = Some(anyhow!("connection failed: {e}"));
                    break;
                }
            },
            _ = &mut sig => break,
        };
        if !running {
            break;
        }

        eprintln!("connected");
        let (mut write, mut read) = ws_stream.split();

        let mut subscribed_ok = false;
        for sub in &subscriptions {
            let sub_msg = sub.to_json_string();
            let send = write.send(Message::Text(sub_msg.clone().into()));
            tokio::select! {
                res = send => {
                    if let Err(e) = res {
                        if running && !no_reconnect {
                            eprintln!("subscribe send failed ({e}); reconnecting");
                            tokio::select! {
                                _ = &mut sig => running = false,
                                _ = tokio::time::sleep(RECONNECT_BACKOFF) => {}
                            }
                            if running {
                                // Restart the whole connection, not just the
                                // subscription loop: the socket is unusable.
                                continue 'session;
                            }
                            break;
                        }
                        fatal = Some(anyhow!("subscribe send failed: {e}"));
                        break;
                    }
                    eprintln!("subscribed: {sub_msg}");
                }
                _ = &mut sig => {
                    running = false;
                    break;
                }
            }
        }
        if !running || fatal.is_some() {
            break;
        }

        let mut stdout = std::io::stdout();

        while running {
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
            let frame = match parse_live_frame(text) {
                Ok(f) => f,
                Err(e) => {
                    fatal = Some(e);
                    break;
                }
            };
            match frame {
                LiveFrame::SubscribeOk => {
                    if !subscribed_ok {
                        eprintln!("subscription acknowledged by server");
                        subscribed_ok = true;
                    }
                }
                LiveFrame::Error(err_text) => {
                    fatal = Some(anyhow!("RIS Live rejected the subscription: {err_text}"));
                    break;
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
                            if let Err(e) = rec.process(&elem) {
                                fatal = Some(e);
                                break;
                            }
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
                                if e.kind() == std::io::ErrorKind::BrokenPipe {
                                    // e.g. `| head`: stop cleanly.
                                    running = false;
                                } else {
                                    fatal = Some(anyhow!("stdout write failed: {e}"));
                                }
                                break;
                            }
                        }
                    }
                    if fatal.is_some() {
                        break;
                    }
                }
            }
        }

        // Flush recorder before deciding on reconnect.
        if let Some(rec) = recorder.as_mut() {
            if let Err(e) = rec.flush() {
                fatal = Some(e);
            }
        }

        eprintln!(
            "disconnected after {} messages; total elements: {}",
            stats.messages, stats.elems
        );

        if fatal.is_some() || !running || no_reconnect {
            break;
        }
        eprintln!("reconnecting in {RECONNECT_BACKOFF:?}s");
        tokio::select! {
            _ = &mut sig => running = false,
            _ = tokio::time::sleep(RECONNECT_BACKOFF) => {}
        }
    }

    // Finalize the recording on every exit path (success, Ctrl-C, fatal
    // error) so buffered elements are never lost; the finalization error,
    // if any, does not mask the original fatal error.
    let result = match fatal {
        Some(e) => Err(e),
        None => Ok(()),
    };
    if let Some(rec) = recorder.as_mut() {
        match rec.finish() {
            Ok(()) => {
                eprintln!("recorded {} elements to {:?}", stats.recorded, rec.path);
            }
            Err(e) if result.is_ok() => return Err(e),
            Err(e) => eprintln!("record finalization also failed: {e}"),
        }
    }
    result
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
