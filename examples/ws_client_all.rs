//! WebSocket client example: call all currently-registered Monocle WS methods
//!
//! Run (from repo root):
//!   # Start the server (in another terminal):
//!   cargo run --features server --bin monocle -- server --address 127.0.0.1 --port 8080
//!
//!   # (optional) Health check:
//!   curl http://127.0.0.1:8080/health
//!
//!   # Run this client:
//!   MONOCLE_WS_URL=ws://127.0.0.1:8080/ws cargo run --example ws_client_all
//!
//! Notes:
//! - This example assumes the current server methods are non-streaming, so each request
//!   gets exactly one terminal response (`result` or `error`) and MUST NOT include `op_id`.
//! - Presets for future streaming methods `search.start` and `parse.start` are included
//!   as commented JSON payloads at the bottom, matching the requested constraints.
//!
//! Dependencies:
//! - Add to your Cargo.toml (client side / examples):
//!     anyhow = "1"
//!     futures-util = "0.3"
//!     serde = { version = "1", features = ["derive"] }
//!     serde_json = "1"
//!     tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
//!     tokio-tungstenite = "0.24"
//!     tungstenite = "0.24"

use anyhow::{anyhow, bail, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, tungstenite};

#[derive(Debug, Clone, Serialize)]
struct RequestEnvelope {
    id: String,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct ResponseEnvelope {
    id: String,
    #[serde(default)]
    op_id: Option<String>,
    #[serde(rename = "type")]
    response_type: String,
    data: Value,
}

async fn call_once(
    write: &mut (impl SinkExt<Message, Error = tungstenite::Error> + Unpin),
    read: &mut (impl StreamExt<Item = std::result::Result<Message, tungstenite::Error>> + Unpin),
    id: &str,
    method: &str,
    params: Value,
) -> Result<ResponseEnvelope> {
    let req = RequestEnvelope {
        id: id.to_string(),
        method: method.to_string(),
        params,
    };

    let text = serde_json::to_string(&req).context("serialize request")?;
    write
        .send(Message::Text(text))
        .await
        .with_context(|| format!("send request failed (id={id} method={method})"))?;

    // Read until we see a terminal response with matching id.
    while let Some(msg) = read.next().await {
        let msg = msg.context("read ws message")?;
        let text = match msg {
            Message::Text(s) => s,
            Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
            // Ignore pings/pongs/closes for the purposes of this demo.
            _ => continue,
        };

        let env: ResponseEnvelope =
            serde_json::from_str(&text).with_context(|| format!("parse response: {text}"))?;

        if env.id != id {
            // Could be a progress/stream message from another in-flight operation.
            // This example is sequential; we just log and continue.
            eprintln!(
                "rx (unmatched): id={} type={} op_id={:?}",
                env.id, env.response_type, env.op_id
            );
            continue;
        }

        // For current non-streaming methods, op_id must be absent.
        if env.op_id.is_some() {
            bail!(
                "protocol violation: non-streaming response included op_id: {:?}",
                env.op_id
            );
        }

        // Terminal types for current methods should be result/error.
        if env.response_type != "result" && env.response_type != "error" {
            bail!(
                "unexpected response type for non-streaming call: {}",
                env.response_type
            );
        }

        // Treat error as an error return with context.
        if env.response_type == "error" {
            return Err(anyhow!(
                "server returned error for {method} (id={id}): {}",
                env.data
            ));
        }

        return Ok(env);
    }

    bail!("connection closed before response for id={id} method={method}");
}

#[tokio::main]
async fn main() -> Result<()> {
    let url = env::var("MONOCLE_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:3000/ws".to_string());
    eprintln!("connecting: {url}");

    let (ws, _resp) = connect_async(&url).await.context("connect websocket")?;
    eprintln!("connected");

    let (mut write, mut read) = ws.split();

    // -------------------------------------------------------------------------
    // Call all currently registered WS methods (monocle/src/server/mod.rs create_router)
    // -------------------------------------------------------------------------

    // 1) system.info
    let r = call_once(
        &mut write,
        &mut read,
        "1",
        "system.info",
        serde_json::json!({}),
    )
    .await?;
    println!("\nsystem.info => {:#}", r.data);

    // 2) time.parse
    let r = call_once(
        &mut write,
        &mut read,
        "2",
        "time.parse",
        serde_json::json!({
            "times": ["1700000000", "2024-01-01T00:00:00Z"],
            "format": null
        }),
    )
    .await?;
    println!("\ntime.parse => {:#}", r.data);

    // 3) country.lookup
    let r = call_once(
        &mut write,
        &mut read,
        "3",
        "country.lookup",
        serde_json::json!({ "query": "US" }),
    )
    .await?;
    println!("\ncountry.lookup => {:#}", r.data);

    // 4) ip.lookup
    let r = call_once(
        &mut write,
        &mut read,
        "4",
        "ip.lookup",
        serde_json::json!({ "ip": "1.1.1.1", "simple": false }),
    )
    .await?;
    println!("\nip.lookup => {:#}", r.data);

    // 5) ip.public
    let r = call_once(
        &mut write,
        &mut read,
        "5",
        "ip.public",
        serde_json::json!({}),
    )
    .await?;
    println!("\nip.public => {:#}", r.data);

    // 6) rpki.validate
    let r = call_once(
        &mut write,
        &mut read,
        "6",
        "rpki.validate",
        serde_json::json!({ "prefix": "1.1.1.0/24", "asn": 13335 }),
    )
    .await?;
    println!("\nrpki.validate => {:#}", r.data);

    // 7) rpki.roas (DB-first; date is not supported in DB-first mode currently)
    let r = call_once(
        &mut write,
        &mut read,
        "7",
        "rpki.roas",
        serde_json::json!({
            "asn": 13335,
            "prefix": null,
            "date": null,
            "source": "cloudflare"
        }),
    )
    .await?;
    println!("\nrpki.roas => {:#}", r.data);

    // 8) rpki.aspas (DB-first; date is not supported in DB-first mode currently)
    let r = call_once(
        &mut write,
        &mut read,
        "8",
        "rpki.aspas",
        serde_json::json!({
            "customer_asn": 13335,
            "provider_asn": null,
            "date": null,
            "source": "cloudflare"
        }),
    )
    .await?;
    println!("\nrpki.aspas => {:#}", r.data);

    // 9) as2org.search
    let r = call_once(
        &mut write,
        &mut read,
        "9",
        "as2org.search",
        serde_json::json!({
            "query": "cloudflare",
            "asn_only": false,
            "name_only": true,
            "country_only": false,
            "full_country": false,
            "full_table": false
        }),
    )
    .await?;
    println!("\nas2org.search => {:#}", r.data);

    // 10) as2org.bootstrap
    let r = call_once(
        &mut write,
        &mut read,
        "10",
        "as2org.bootstrap",
        serde_json::json!({ "force": false }),
    )
    .await?;
    println!("\nas2org.bootstrap => {:#}", r.data);

    // 11) as2rel.search
    let r = call_once(
        &mut write,
        &mut read,
        "11",
        "as2rel.search",
        serde_json::json!({ "asns": [13335], "sort_by_asn": false, "show_name": true }),
    )
    .await?;
    println!("\nas2rel.search => {:#}", r.data);

    // 12) as2rel.relationship
    let r = call_once(
        &mut write,
        &mut read,
        "12",
        "as2rel.relationship",
        serde_json::json!({ "asn1": 13335, "asn2": 174 }),
    )
    .await?;
    println!("\nas2rel.relationship => {:#}", r.data);

    // 13) as2rel.update (DB-first WS policy: this endpoint is expected to be rejected)
    // If your server returns an error, that's expected; we print it and continue.
    match call_once(
        &mut write,
        &mut read,
        "13",
        "as2rel.update",
        serde_json::json!({ "url": null }),
    )
    .await
    {
        Ok(r) => println!("\nas2rel.update (unexpected success) => {:#}", r.data),
        Err(e) => println!("\nas2rel.update (expected failure) => {e}"),
    }

    // 14) pfx2as.lookup (cache-only DB-first policy)
    let r = call_once(
        &mut write,
        &mut read,
        "14",
        "pfx2as.lookup",
        serde_json::json!({ "prefix": "1.1.1.0/24", "mode": "longest" }),
    )
    .await?;
    println!("\npfx2as.lookup => {:#}", r.data);

    // 15) database.status
    let r = call_once(
        &mut write,
        &mut read,
        "15",
        "database.status",
        serde_json::json!({}),
    )
    .await?;
    println!("\ndatabase.status => {:#}", r.data);

    // 16) database.refresh
    let r = call_once(
        &mut write,
        &mut read,
        "16",
        "database.refresh",
        serde_json::json!({ "source": "pfx2as-cache", "force": false }),
    )
    .await?;
    println!("\ndatabase.refresh => {:#}", r.data);

    // Close cleanly.
    let _ = write.send(Message::Close(None)).await;

    // -------------------------------------------------------------------------
    // Requested presets for FUTURE streaming endpoints (disabled until implemented)
    // -------------------------------------------------------------------------
    //
    // SEARCH preset:
    // - time window: 2025-01-01T00:00:00Z for one hour
    // - collector: route-views3
    // - origin ASN: 13335
    //
    // let search_start_params = serde_json::json!({
    //   "filters": {
    //     "origin_asn": 13335,
    //     "start_ts": "2025-01-01T00:00:00Z",
    //     "end_ts": "2025-01-01T01:00:00Z"
    //   },
    //   "collector": "route-views3",
    //   "project": "routeviews",
    //   "dump_type": "updates",
    //   "batch_size": 500,
    //   "max_results": 5000
    // });
    // // When implemented, expect:
    // // - 0..N progress events (type=progress, includes op_id)
    // // - 0..N stream batches (type=stream, includes op_id)
    // // - exactly 1 terminal result/error (includes op_id)
    // // call_streaming("search.start", search_start_params).await?;
    //
    // PARSE preset:
    // - use a small MRT updates file (local path or URL)
    // - origin ASN filter: 13335
    //
    // Choose a real file path/URL for your environment; examples:
    // - local: "./examples/data/small-updates.mrt"
    // - remote: "https://.../some-small-updates.mrt.bz2"
    //
    // let parse_start_params = serde_json::json!({
    //   "file_path": "./examples/data/small-updates.mrt",
    //   "filters": { "origin_asn": 13335 },
    //   "batch_size": 500,
    //   "max_results": 5000
    // });
    // // call_streaming("parse.start", parse_start_params).await?;
    //
    // -------------------------------------------------------------------------

    Ok(())
}
