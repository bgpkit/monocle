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
use std::future::Future;
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
    // main() resets SIGPIPE to SIG_DFL so finite commands terminate on broken
    // pipes; for watch that would kill the process at the kernel write and
    // bypass recorder finalization. Restore the ignored disposition so writes
    // return EPIPE, which the stream loop already handles cleanly.
    #[cfg(unix)]
    {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        }
    }

    // The tree enables two rustls CryptoProviders (aws-lc-rs via oneio, ring
    // via reqwest); pick ring once so TLS setup cannot fail ambiguously.
    install_crypto_provider();

    // Table output cannot stream (format_elem returns None for it); reject
    // before touching any file or network. --pretty only upgrades compact
    // JSON, so it does not make Table streamable either.
    if output_format == OutputFormat::Table {
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

    // Create the runtime first: if it fails, nothing has been opened or
    // truncated yet. Then open the record writer before entering the async
    // runtime, because oneio wraps reqwest::blocking, whose internal tokio
    // runtime must not be created or dropped from within an async context.
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create async runtime: {e}");
            std::process::exit(1);
        }
    };

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
                        eprintln!("connection failed ({e}); retrying in {RECONNECT_BACKOFF:?}");
                        if fut_select_sig_or_sleep(&mut sig).await {
                            break;
                        }
                        continue 'session;
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
                            // Restart the whole connection, not just the
                            // subscription loop: the socket is unusable.
                            if fut_select_sig_or_sleep(&mut sig).await {
                                running = false;
                                break;
                            }
                            continue 'session;
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
                        // Without reconnection an interrupted stream is a
                        // failure: keep it fatal so scripts see a nonzero exit.
                        if no_reconnect {
                            fatal = Some(anyhow!("stream error: {e}"));
                        }
                        break;
                    }
                    None => {
                        eprintln!("stream closed by server");
                        if no_reconnect {
                            fatal = Some(anyhow!("stream closed by server"));
                        }
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
        eprintln!("reconnecting in {RECONNECT_BACKOFF:?}");
        // A completed ctrl_c future must not be polled again; break the
        // session loop here instead of relying on the `running` check.
        if fut_select_sig_or_sleep(&mut sig).await {
            break;
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

/// Await Ctrl-C (returns true) or the reconnect backoff (returns false).
///
/// Wraps the one-shot `ctrl_c` future so that once it has completed it is
/// never polled again (re-polling a completed future panics); callers must
/// treat `true` as terminal.
async fn fut_select_sig_or_sleep<F: Future<Output = std::io::Result<()>> + Unpin>(
    sig: &mut F,
) -> bool {
    let slept = tokio::time::sleep(RECONNECT_BACKOFF);
    tokio::pin!(slept);
    tokio::select! {
        _ = sig => true,
        _ = &mut slept => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgpkit_parser::BgpkitParser;

    fn synthetic_elem(i: u32) -> bgpkit_parser::BgpElem {
        bgpkit_parser::BgpElem {
            peer_ip: "192.0.2.1".parse().unwrap(),
            peer_asn: bgpkit_parser::models::Asn::new_32bit(64512),
            prefix: "203.0.113.0/24".parse().unwrap(),
            timestamp: 1_700_000_000.0 + f64::from(i),
            elem_type: bgpkit_parser::models::ElemType::ANNOUNCE,
            ..Default::default()
        }
    }

    #[test]
    fn recorder_flushes_at_boundary_and_finalizes_partial_batch() {
        for count in [499u32, 500, 501] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(format!("rec-{count}.mrt"));
            let mut rec = MrtRecorder::new(path.clone()).unwrap();
            for i in 0..count {
                let mut elem = synthetic_elem(i);
                elem.timestamp = 1_700_000_000.0 + f64::from(i);
                rec.process(&elem).unwrap();
            }
            rec.finish().unwrap();

            // The finished file must replay to exactly the elements written.
            let parser = BgpkitParser::new(path.to_str().unwrap()).unwrap();
            let replayed = parser.into_iter().count();
            assert_eq!(replayed, count as usize, "count {count} round trip");
        }
    }

    #[test]
    fn recorder_finish_without_elements_writes_valid_empty_stream() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.mrt");
        let mut rec = MrtRecorder::new(path.clone()).unwrap();
        rec.finish().unwrap();
        // File exists and is valid (possibly zero-record) MRT.
        assert!(path.exists());
    }
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
