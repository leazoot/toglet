#!/usr/bin/env node
// A reference bridge for Toglet's remote control. Zero dependencies: `node bridge.mjs`.
//
//   POST /toglet   Toglet's poll. Carries a signed status receipt; answers with at most one
//                  queued command, holding the request up to 25 seconds when there is none.
//   GET  /status   The page reads what Toglet last said.
//   POST /command  The page queues a signed command for Toglet to collect.
//
// It holds no secret: it cannot sign a command and does not verify receipts. Everything is in
// memory, so a restart drops uncollected commands.

import { createServer } from "node:http";

const PORT = Number(process.env.PORT ?? 8787);
const HOST = process.env.HOST ?? "127.0.0.1";

/** Long enough to be a long poll, short enough to stay inside any proxy's idle timeout. */
const HOLD_MS = 25_000;

/** Matches the fifteen minutes Toglet accepts an envelope for. Older commands are dropped. */
const COMMAND_TTL_MS = 15 * 60 * 1000;

/** One bridge serves one person; a bounded queue cannot be flooded. */
const MAX_QUEUED = 8;

/** The last receipt Toglet sent, relayed to the page verbatim. */
let lastReceipt = null;
let lastReceiptAt = 0;

/** Commands waiting to be collected, oldest first. */
const queue = [];

/** Polls parked with nothing to give, waiting for a command to arrive. */
const waiting = new Set();

function prune() {
  const cutoff = Date.now() - COMMAND_TTL_MS;
  while (queue.length > 0 && queue[0].at < cutoff) {
    queue.shift();
  }
}

function handOut(response) {
  prune();
  const next = queue.shift();
  send(response, 200, { v: 1, command: next ? next.command : null });
}

function wake() {
  for (const parked of waiting) {
    clearTimeout(parked.timer);
    waiting.delete(parked);
    handOut(parked.response);
    // One command, one poll. The rest stay parked.
    if (queue.length === 0) break;
  }
}

function send(response, status, body) {
  const text = JSON.stringify(body);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(text),
    // The page may be served from another origin.
    "access-control-allow-origin": "*",
    "access-control-allow-headers": "content-type",
  });
  response.end(text);
}

async function readBody(request, limit = 16 * 1024) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > limit) throw new Error("too large");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

const server = createServer((request, response) => {
  const url = new URL(request.url ?? "/", `http://${request.headers.host ?? "localhost"}`);

  if (request.method === "OPTIONS") {
    send(response, 204, {});
    return;
  }

  // ---- Toglet: hand in a receipt, take away a command ------------------------------------
  if (request.method === "POST" && url.pathname === "/toglet") {
    readBody(request)
      .then((body) => {
        try {
          lastReceipt = JSON.parse(body);
          lastReceiptAt = Date.now();
        } catch {
          // An unparseable receipt is still a poll; refusing it would stall command delivery.
        }
        prune();
        if (queue.length > 0) {
          handOut(response);
          return;
        }
        // Nothing to give: hold the request so a command that arrives meanwhile goes out at once.
        const parked = { response, timer: null };
        parked.timer = setTimeout(() => {
          waiting.delete(parked);
          handOut(response);
        }, HOLD_MS);
        waiting.add(parked);
        request.on("close", () => {
          clearTimeout(parked.timer);
          waiting.delete(parked);
        });
      })
      .catch(() => send(response, 413, { error: "too large" }));
    return;
  }

  // ---- the page: what did Toglet last say? ------------------------------------------------
  if (request.method === "GET" && url.pathname === "/status") {
    send(response, 200, {
      v: 1,
      receipt: lastReceipt,
      // Lets the page stop showing an old state as current.
      ageSeconds: lastReceipt === null ? null : Math.round((Date.now() - lastReceiptAt) / 1000),
      queued: queue.length,
    });
    return;
  }

  // ---- the page: here is a command ---------------------------------------------------------
  if (request.method === "POST" && url.pathname === "/command") {
    readBody(request)
      .then((body) => {
        let command;
        try {
          command = JSON.parse(body);
        } catch {
          send(response, 400, { error: "not json" });
          return;
        }
        prune();
        if (queue.length >= MAX_QUEUED) {
          send(response, 429, { error: "too many waiting" });
          return;
        }
        queue.push({ command, at: Date.now() });
        wake();
        send(response, 202, { v: 1, queued: queue.length });
      })
      .catch(() => send(response, 413, { error: "too large" }));
    return;
  }

  // List the valid paths so a mistyped address is easy to diagnose. They are not secret.
  send(response, 404, {
    error: "no such path",
    paths: { toglet: "POST /toglet", status: "GET /status", command: "POST /command" },
  });
});

server.listen(PORT, HOST, () => {
  // The port actually bound, which differs from PORT when PORT is 0.
  const { port } = server.address();
  process.stdout.write(`bridge listening on http://${HOST}:${port}\n`);
  process.stdout.write("  Toglet posts to /toglet, the page uses /status and /command\n");
  process.stdout.write("  in memory only; put https in front of it before using it for real\n");
});
