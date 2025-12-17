/**
 * Monocle WebSocket client example (JavaScript)
 *
 * Calls all currently-registered WebSocket methods in the Monocle server.
 *
 * Usage:
 *   MONOCLE_WS_URL=ws://127.0.0.1:3000/ws node monocle/examples/ws_client_all.js
 *
 * Dependencies:
 *   npm i ws
 *
 * Notes:
 * - This example assumes the server uses the request/response envelope:
 *     { id: string, method: string, params: any }
 * - Responses:
 *     { id: string, op_id?: string, type: "result"|"progress"|"stream"|"error", data: any }
 * - For current non-streaming methods: `op_id` should be absent.
 */

"use strict";

const WebSocket = require("ws");

const WS_URL = process.env.MONOCLE_WS_URL || "ws://127.0.0.1:3000/ws";

class MonocleClient {
  constructor(url) {
    this.url = url;
    this.ws = null;
    this.nextId = 1;

    /** @type {Map<string, {resolve: Function, reject: Function}>} */
    this.pending = new Map();
  }

  async connect() {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return;

    this.ws = new WebSocket(this.url);

    this.ws.on("message", (buf) => {
      const text = typeof buf === "string" ? buf : buf.toString("utf8");

      /** @type {{id?: string, op_id?: string, type?: string, data?: any}} */
      let msg;
      try {
        msg = JSON.parse(text);
      } catch (e) {
        console.error("Non-JSON message:", text);
        return;
      }

      if (!msg || typeof msg.id !== "string" || !msg.id) {
        console.error("Malformed response (missing id):", msg);
        return;
      }

      const pending = this.pending.get(msg.id);
      if (!pending) {
        // Could be a progress/stream message for a streaming op (not used in this demo).
        console.warn("Unmatched message:", msg);
        return;
      }

      // Non-streaming calls should only ever see one terminal response.
      if (msg.type === "result") {
        pending.resolve(msg);
        this.pending.delete(msg.id);
      } else if (msg.type === "error") {
        const err = new Error(
          msg?.data?.message ? String(msg.data.message) : "Server error"
        );
        err.code = msg?.data?.code;
        err.details = msg?.data?.details;
        err.envelope = msg;
        pending.reject(err);
        this.pending.delete(msg.id);
      } else {
        // progress/stream for streaming ops; for this demo we just log.
        console.log("event:", msg.type, msg);
      }
    });

    this.ws.on("error", (err) => {
      // Fail all pending requests
      for (const [id, pending] of this.pending.entries()) {
        pending.reject(err);
        this.pending.delete(id);
      }
    });

    await new Promise((resolve, reject) => {
      this.ws.once("open", resolve);
      this.ws.once("error", reject);
    });
  }

  /**
   * Sends a JSON request and waits for a terminal response (result/error).
   * @param {string} method
   * @param {any} params
   * @returns {Promise<any>} ResponseEnvelope
   */
  call(method, params) {
    const id = String(this.nextId++);
    const req = { id, method, params };

    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify(req));
    });
  }

  close() {
    if (!this.ws) return;
    try {
      this.ws.close();
    } catch {
      // ignore
    }
  }
}

function pretty(obj) {
  return JSON.stringify(obj, null, 2);
}

async function main() {
  const client = new MonocleClient(WS_URL);
  await client.connect();
  console.log("connected:", WS_URL);

  // 1) system.info
  console.log("\n1) system.info");
  console.log(pretty(await client.call("system.info", {})));

  // 2) time.parse
  console.log("\n2) time.parse");
  console.log(
    pretty(
      await client.call("time.parse", {
        times: ["1700000000", "2024-01-01T00:00:00Z"],
        format: null,
      })
    )
  );

  // 3) country.lookup
  console.log("\n3) country.lookup");
  console.log(pretty(await client.call("country.lookup", { query: "US" })));

  // 4) ip.lookup
  console.log("\n4) ip.lookup");
  console.log(
    pretty(await client.call("ip.lookup", { ip: "1.1.1.1", simple: false }))
  );

  // 5) ip.public
  console.log("\n5) ip.public");
  console.log(pretty(await client.call("ip.public", {})));

  // 6) rpki.validate
  console.log("\n6) rpki.validate");
  console.log(
    pretty(
      await client.call("rpki.validate", { prefix: "1.1.1.0/24", asn: 13335 })
    )
  );

  // 7) rpki.roas (DB-first; date unsupported in current DB-first mode)
  console.log("\n7) rpki.roas");
  console.log(
    pretty(
      await client.call("rpki.roas", {
        asn: 13335,
        prefix: null,
        date: null,
        source: "cloudflare",
      })
    )
  );

  // 8) rpki.aspas (DB-first; date unsupported in current DB-first mode)
  console.log("\n8) rpki.aspas");
  console.log(
    pretty(
      await client.call("rpki.aspas", {
        customer_asn: 13335,
        provider_asn: null,
        date: null,
        source: "cloudflare",
      })
    )
  );

  // 9) as2org.search
  console.log("\n9) as2org.search");
  console.log(
    pretty(
      await client.call("as2org.search", {
        query: "cloudflare",
        asn_only: false,
        name_only: true,
        country_only: false,
        full_country: false,
        full_table: false,
      })
    )
  );

  // 10) as2org.bootstrap
  console.log("\n10) as2org.bootstrap");
  console.log(pretty(await client.call("as2org.bootstrap", { force: false })));

  // 11) as2rel.search
  console.log("\n11) as2rel.search");
  console.log(
    pretty(
      await client.call("as2rel.search", {
        asns: [13335],
        sort_by_asn: false,
        show_name: true,
      })
    )
  );

  // 12) as2rel.relationship
  console.log("\n12) as2rel.relationship");
  console.log(
    pretty(await client.call("as2rel.relationship", { asn1: 13335, asn2: 174 }))
  );

  // 13) as2rel.update (DB-first WS policy: expected to be rejected; use database.refresh)
  console.log("\n13) as2rel.update (expected to error)");
  try {
    console.log(pretty(await client.call("as2rel.update", { url: null })));
  } catch (e) {
    console.log(
      pretty({
        error: true,
        code: e.code || "UNKNOWN",
        message: e.message,
        details: e.details ?? null,
      })
    );
  }

  // 14) pfx2as.lookup (cache-only; populate via database.refresh source=pfx2as-cache)
  console.log("\n14) pfx2as.lookup");
  console.log(
    pretty(
      await client.call("pfx2as.lookup", { prefix: "1.1.1.0/24", mode: "longest" })
    )
  );

  // 15) database.status
  console.log("\n15) database.status");
  console.log(pretty(await client.call("database.status", {})));

  // 16) database.refresh
  console.log("\n16) database.refresh");
  console.log(
    pretty(
      await client.call("database.refresh", { source: "pfx2as-cache", force: false })
    )
  );

  client.close();
}

main().catch((e) => {
  console.error(e);
  process.exitCode = 1;
});
