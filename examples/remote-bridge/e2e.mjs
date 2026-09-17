#!/usr/bin/env node
// Drives the whole path once, on this machine, with no browser and no phone.
//
//   the page  --signed command-->  bridge  <--poll--  Toglet
//
// Both ends are simulated, but the bytes are real: the fixture is signed the way the page signs,
// and `src-tauri/tests/remote_wire.rs` checks the same file with Toglet's own verifier, so the
// two implementations cannot disagree about the signed byte string unnoticed.
//
//   node e2e.mjs            run the round trip and rewrite the fixtures
//   node e2e.mjs --check    run the round trip only (fixtures must already match)

import { createCipheriv, createHash, createHmac } from "node:crypto";
import { spawn } from "node:child_process";
import { writeFileSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURES = join(HERE, "..", "..", "src-tauri", "tests", "fixtures");

const SECRET = "a-shared-secret-for-the-fixtures";
const SESSION = "a83b4c1d9e0f2a3b4c5d6e7f80912233";
const DEVICE = "6f1c00112233445566778899aabbccdd";
const ISSUED_AT = 1_757_664_000;
// Let the operating system pick a free port, so a bridge already running is not in the way.
const PORT = 0;

const mac = (message) => createHmac("sha256", SECRET).update(message).digest("hex");

// The excerpt travels sealed, not merely signed: HMAC proves who sent a receipt but hides
// nothing in it, and the bridge hands the last receipt to whoever asks. The sealing key is the
// shared secret under its own domain separator, so the bytes that sign a receipt are never the
// bytes that open one. Mirrors src-tauri/src/remote/crypt.rs.
const EXCERPT_DOMAIN = "toglet-remote/2 excerpt";

/** Sealed with a fixed nonce, so the fixture is reproducible and a diff means a real change. */
function sealExcerpt(plaintext, nonceHex) {
  const key = createHash("sha256")
    .update(EXCERPT_DOMAIN + SECRET, "utf8")
    .digest();
  const cipher = createCipheriv("aes-256-gcm", key, Buffer.from(nonceHex, "hex"));
  const body = Buffer.concat([cipher.update(plaintext, "utf8"), cipher.final()]);
  // Rust's `aes-gcm` returns the tag appended to the ciphertext; Node hands them over apart.
  return {
    ciphertext: Buffer.concat([body, cipher.getAuthTag()]).toString("hex"),
    nonce: nonceHex,
  };
}

// The key the bridge checks a `/status` read against. Derived one-way from the shared secret, so
// a bridge that is taken over can authenticate readers but still cannot sign a command: that MAC
// uses the raw secret, which never reaches it. Deriving it here from SECRET is also what proves
// the page and the bridge agree on the derivation.
const STATUS_DOMAIN = "toglet-remote/2 status";
const STATUS_KEY = createHash("sha256")
  .update(STATUS_DOMAIN + SECRET, "utf8")
  .digest();

/** What the page sends on a `/status` read. `at` lets a test offer a stale timestamp. */
function statusHeaders(at = Math.floor(Date.now() / 1000)) {
  const signed = ["toglet-remote/2", "status", String(at)].join("\n");
  return {
    "x-toglet-ts": String(at),
    "x-toglet-mac": createHmac("sha256", STATUS_KEY).update(signed).digest("hex"),
  };
}

const EXCERPT_TEXT = "解析器有两种改法，我建议先补一个分支。";
const SEALED_EXCERPT = sealExcerpt(EXCERPT_TEXT, "0f1e2d3c4b5a69788796a5b4");

function command(action, counter, observedState, nonce, issuedAt, text = "") {
  const signed = [
    "toglet-remote/2",
    "command",
    action,
    SESSION,
    observedState,
    String(counter),
    nonce,
    String(issuedAt),
    // Always present, empty for the argument-free actions. Signing it is what stops the bridge
    // rewriting the one thing on this path worth rewriting.
    text,
  ].join("\n");
  const envelope = {
    v: 2,
    kind: "command",
    action,
    sessionId: SESSION,
    observedState,
    counter,
    nonce,
    issuedAt,
    mac: mac(signed),
  };
  // Absent and empty sign identically, so only `send` carries the field at all.
  if (text !== "") envelope.text = text;
  return envelope;
}

function receipt(
  state,
  waitReason,
  expectedAvailableAt,
  cursor,
  nextPollSeconds,
  resumeCount,
  lastCommand,
  excerpt = null,
) {
  const signed = [
    "toglet-remote/2",
    "receipt",
    DEVICE,
    SESSION,
    String(ISSUED_AT),
    state,
    waitReason ?? "",
    expectedAvailableAt === null ? "" : String(expectedAvailableAt),
    String(cursor),
    String(nextPollSeconds),
    String(resumeCount),
    // The sealed excerpt. A sealed value is never empty (GCM always emits a tag), so absent and
    // present cannot alias.
    excerpt === null ? "" : excerpt.ciphertext,
    excerpt === null ? "" : excerpt.nonce,
    lastCommand === null ? "" : String(lastCommand.counter),
    lastCommand === null ? "" : lastCommand.action,
    lastCommand === null ? "" : lastCommand.result,
  ].join("\n");
  return {
    v: 2,
    kind: "receipt",
    deviceId: DEVICE,
    sessionId: SESSION,
    issuedAt: ISSUED_AT,
    state,
    waitReason,
    expectedAvailableAt,
    cursor,
    nextPollSeconds,
    resumeCount,
    excerptCiphertext: excerpt === null ? null : excerpt.ciphertext,
    excerptNonce: excerpt === null ? null : excerpt.nonce,
    lastCommand,
    mac: mac(signed),
  };
}

// A fixed nonce, so the fixture is byte-for-byte reproducible and a diff means a real change.
const GENUINE = command("resume", 43, "needs_human", "9d2e0011223344556677889900aabbcc", ISSUED_AT);

const fixtures = {
  secret: SECRET,
  sessionId: SESSION,
  deviceId: DEVICE,
  issuedAt: ISSUED_AT,
  genuine: GENUINE,
  // One byte of the signature changed. Toglet must refuse it.
  forged: { ...GENUINE, mac: GENUINE.mac.replace(/^./, (c) => (c === "0" ? "1" : "0")) },
  receipt: receipt("needs_human", "waiting_on_human", null, 42, 20, 3, {
    counter: 42,
    action: "resume",
    result: "applied",
  }),
  // The one action that carries a parameter; its text is inside the signature.
  send: command(
    "send",
    44,
    "needs_human",
    "1a2b0011223344556677889900aabbcc",
    ISSUED_AT,
    "go with your recommendation",
  ),
  receiptWaiting: receipt(
    "waiting_quota",
    "five_hour_exhausted",
    ISSUED_AT + 5040,
    42,
    20,
    3,
    null,
  ),
  // Sealed here, opened by Toglet in `remote_wire.rs`. Without this the two AES-GCM sides could
  // disagree about the derived key, the nonce or where the tag sits, and the only symptom would
  // be a phone that says "cannot decrypt" forever.
  sealedExcerpt: { plaintext: EXCERPT_TEXT, ...SEALED_EXCERPT },
  // The same sealed halves inside a receipt's signed bytes, so a bridge cannot swap them.
  receiptExcerpt: receipt("needs_human", "waiting_on_human", null, 42, 20, 3, null, SEALED_EXCERPT),
};

/** Retries `attempt` until it answers with something, or gives up after two seconds. */
async function until(attempt) {
  for (let tries = 0; tries < 40; tries += 1) {
    const value = await attempt();
    if (value !== null) return value;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return { receipt: null };
}

async function roundTrip() {
  const bridge = spawn(process.execPath, [join(HERE, "bridge.mjs")], {
    env: {
      ...process.env,
      PORT: String(PORT),
      HOST: "127.0.0.1",
      STATUS_KEY: STATUS_KEY.toString("hex"),
    },
    stdio: ["ignore", "pipe", "inherit"],
  });
  // The spawn is outside the `try`, so the `finally` alone does not cover it. The bridge must not
  // outlive the script on a throw or Ctrl-C.
  const stop = () => {
    try {
      bridge.kill();
    } catch {
      // Already gone.
    }
  };
  process.once("exit", stop);
  process.once("SIGINT", () => {
    stop();
    process.exit(130);
  });

  // With PORT=0, the bridge's startup line is the only way to learn its port.
  const announced = await new Promise((resolve) => bridge.stdout.once("data", resolve));
  const port = /:(\d+)/.exec(String(announced))?.[1];
  if (port === undefined) throw new Error("the bridge did not say which port it took");
  const base = `http://127.0.0.1:${port}/`;
  const say = (ok, text) => console.log(`${ok ? "  ok  " : "  FAIL"} ${text}`);
  let failures = 0;
  const check = (ok, text) => {
    say(ok, text);
    if (!ok) failures += 1;
  };

  try {
    // 1. Toglet polls with nothing waiting; the bridge parks it until the command below arrives.
    const poll = fetch(`${base}toglet`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(fixtures.receipt),
    }).then((response) => response.json());

    // 2. The page reads the status. Retried, because the parked poll's body may not be read yet.
    const status = await until(async () => {
      const body = await (await fetch(`${base}status`, { headers: statusHeaders() })).json();
      return body.receipt === null ? null : body;
    });
    check(status.receipt?.state === "needs_human", "the page sees the state Toglet reported");
    check(status.receipt?.mac === fixtures.receipt.mac, "the receipt is relayed byte for byte");

    // 2b. The three ways a read must be refused. Without these the endpoint could be wide open
    //     and every check above would still pass.
    const unsigned = await fetch(`${base}status`);
    check(unsigned.status === 401, "an unsigned status read is refused");

    const forged = statusHeaders();
    forged["x-toglet-mac"] = forged["x-toglet-mac"].replace(/^./, (c) => (c === "0" ? "1" : "0"));
    const badMac = await fetch(`${base}status`, { headers: forged });
    check(badMac.status === 401, "a status read signed with the wrong key is refused");

    const old = Math.floor(Date.now() / 1000) - 16 * 60;
    const stale = await fetch(`${base}status`, { headers: statusHeaders(old) });
    check(stale.status === 401, "a correctly signed but stale status read is refused");

    // 3. The page delivers a signed command, which wakes the parked poll.
    const queued = await fetch(`${base}command`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(GENUINE),
    });
    check(queued.status === 202, "the bridge accepts a command");

    const collected = await poll;
    check(collected.command?.mac === GENUINE.mac, "the parked poll is woken and hands it over");
    check(collected.command?.action === "resume", "the command arrives unchanged");

    // 4. Nothing is left behind: a second poll gets nothing, after a short hold.
    const started = Date.now();
    const second = await fetch(`${base}toglet`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(fixtures.receipt),
    }).then((response) => response.json());
    check(second.command === null, "a collected command is not handed out twice");
    check(Date.now() - started > 20_000, "an empty poll is held open rather than answered at once");
  } finally {
    bridge.kill();
  }
  return failures;
}

const path = join(FIXTURES, "remote_wire.json");
const rendered = `${JSON.stringify(fixtures, null, 2)}\n`;

if (process.argv.includes("--check")) {
  const onDisk = readFileSync(path, "utf8");
  if (onDisk !== rendered) {
    console.error("the fixtures on disk differ from what this script produces");
    process.exit(1);
  }
  console.log("  ok   the fixtures match");
} else {
  writeFileSync(path, rendered);
  console.log(`  ok   wrote ${path}`);
}

process.exit((await roundTrip()) === 0 ? 0 : 1);
