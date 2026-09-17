// Toglet Remote. One screen, and one rule: never show a button that would not work.
//
// A task waiting on a Codex approval still cannot be *continued* from here - `resume` would
// re-read the thread and land back on "needs you" - so the continue button does not exist in
// that state. What does work there is answering: a sentence interrupts the turn holding the
// question and starts a fresh one with those words. Hence the composer, and hence "send" being
// the primary action on exactly the screen where "continue" is absent.
//
// Wire format: src-tauri/src/remote/envelope.rs.

// ---------------------------------------------------------------- language

// The phone's language, not the desktop's. Toglet sends codes and never sentences, which is what
// lets this page speak whatever the person holding it speaks.
const ZH = (navigator.language || "en").toLowerCase().startsWith("zh");

const COPY = {
  en: {
    brand: "Toglet",

    connected: "connected",
    syncing: "syncing",
    offline: "offline",
    unlinked: "not connected",

    pairTitle: "Connect Toglet",
    pairIntro: "Connect to the Toglet Bridge on your computer.",
    bridge: "Bridge address",
    secret: "Shared secret",
    writeOnly: "write-only",
    pairFine: "The secret is not shown once saved; leave it blank to keep the current one.",
    pair: "Connect",
    forget: "Disconnect",

    running: "Running",
    runningWhy: (rounds) =>
      rounds > 0 ? `Continuation ${rounds} in progress` : "The task is running",

    waiting: "Waiting for quota",
    waitingWhy: "Quota is temporarily exhausted; it will continue once quota resets",
    waitingAt: (time) => `Continues around ${time}`,
    waitingUnknown: "Waiting for quota to reset",

    left: "left",
    stopped: "Stopped",

    needsYou: "Waiting for you",
    needsYouWhy: "Codex is waiting for your input",
    askLead: "Answer from here",
    askTail:
      "Your reply starts a fresh turn with those words; the question Codex is holding is dropped.",

    paused: "Paused",
    pausedWhy: "Paused from your phone",

    unavailable: "State unavailable",
    unbound: "No task bound",

    unreachableWhy: "Cannot reach the Toglet Bridge",
    unauthorizedWhy: "The bridge did not accept this key; its STATUS_KEY may be out of date",
    notFoundWhy: "No bridge at this address",
    noStatusKeyWhy: "The bridge has no STATUS_KEY installed yet",
    garbledWhy: "The bridge's reply could not be read; a proxy may be in the way",
    refusedWhy: (code) => `The bridge did not return a state (${code})`,
    neverCameWhy: "The bridge is connected, but no state has arrived from Toglet",
    staleWhy: "No recent state from the computer",
    unboundWhy: "No task is bound on the computer",

    lastState: "LAST STATE · EXPIRED",
    noStateNoAction: "The task state is unavailable, so the actions are disabled.",
    checkAddress:
      "Check that the bridge address ends in /toglet and that remote control is turned on.",

    nothingToContinue: "The task is running, so there is nothing to continue.",
    quotaNothing: "It continues by itself once quota resets.",

    continue: "Continue",
    pause: "Pause",
    cancel: "Hold to cancel",
    cancelTask: "Hold to cancel task",
    releasing: "Release to cancel",
    cancelStep1: "Cancel task",
    cancelStep2: "Confirm cancel",
    retry: "Retry",

    trace: "CONTINUE REQUEST · SENDING",
    traceSend: "REPLY · SENDING",

    composerHint: "Write a reply\u2026",
    send: "Send",
    phrases: "Quick phrases",
    phrasesTag: "this phone",
    phrasesFine: "One per line. Stored on this phone only - they never reach the bridge.",
    counter: (n, max) => `${n} / ${max}`,

    excerptHead: "CODEX LAST SAID",
    excerptNone: "No content preview",
    // Deliberately does not name a cause: a null excerpt means the switch is off, or the agent
    // has not spoken this turn, or sealing failed - and the receipt cannot tell them apart.
    excerptNoneWhy:
      "Either the preview is turned off on the computer, or nothing new has been said this turn.",
    excerptSealed: "Content preview could not be decrypted",
    excerptSealedWhy: "The shared secret on this phone does not match the one on the computer.",
    delivered: "Sent",
    collected: "Received",
    applied: "Executed",
    asleep: "The computer may be asleep; it will carry this out once it wakes.",

    syncedAgo: (text) => `synced ${text}`,
    lastSync: (time) => `Last synced: ${time}`,
    neverSynced: "not synced yet",

    secondsAgo: (n) => `${n}s ago`,
    minutesAgo: (n) => `${n} min ago`,
    hoursAgo: (n) => `${n} h ago`,

    reasons: {
      completed: "The round completed",
      user_stopped: "The task was stopped manually",
      interrupted: "The task was interrupted",
      network: "The network connection failed",
      manual_switch: "The account was switched manually",
      desktop_reopened: "Codex was restarted",
      query_failures: "Quota status could not be read",
      status_unknown: "The end state of the task could not be identified",
      failed_unknown_reason: "The task failed",
      waiting_on_human: "Waiting for your input",
      reauth_required: "An account needs to sign in again",
      unauthorized: "The sign-in is no longer valid",
      identity_mismatch: "The signed-in account is not the expected one",
      thread_unavailable: "The session could not be resumed",
      max_resumes: "The continuation limit was reached",
      deadline: "The deadline was reached",
      no_account_available: "No account is available",
    },

    outcomes: {
      applied: "Executed",

      remote_bad_mac: "Rejected: signature verification failed",
      remote_replayed: "Rejected: already processed",
      remote_nonce_reused: "Rejected: already processed",
      remote_expired: "Rejected: the request expired",
      remote_session_mismatch: "Rejected: task mismatch",
      remote_state_changed: "Rejected: the task state changed",
      remote_rate_limited: "Too many requests, try again later",
      remote_version: "Rejected: incompatible version",
      remote_malformed: "Rejected: malformed request",
      remote_unknown_action: "Rejected: unsupported action",
      remote_unavailable: "Not carried out: the task is unavailable",
    },
  },
  zh: {
    brand: "Toglet",

    connected: "已连接",
    syncing: "同步中",
    offline: "离线",
    unlinked: "未连接",

    pairTitle: "连接 Toglet",
    pairIntro: "连接电脑上的 Toglet Bridge。",
    bridge: "Bridge 地址",
    secret: "共享密钥",
    writeOnly: "仅写入",
    pairFine: "密钥保存后不会显示；留空则保留当前密钥。",
    pair: "连接",
    forget: "断开连接",

    running: "运行中",
    runningWhy: (rounds) => (rounds > 0 ? `正在进行第 ${rounds} 轮续跑` : "任务正在运行"),

    waiting: "等待额度",
    waitingWhy: "额度暂时不足，将在恢复后自动继续",
    waitingAt: (time) => `预计 ${time} 继续`,
    waitingUnknown: "等待额度恢复",

    left: "剩余",
    stopped: "已停止",

    needsYou: "等待你的操作",
    needsYouWhy: "Codex 正在等待你的输入",
    askLead: "在这里回答",
    askTail: "你的回复会用这句话重新开始一轮；Codex 正等着的那个问题会被放弃。",

    paused: "已暂停",
    pausedWhy: "已从手机暂停",

    unavailable: "状态不可用",
    unbound: "未绑定任务",

    unreachableWhy: "无法连接到 Toglet Bridge",
    unauthorizedWhy: "Bridge 不接受这把密钥，它上面的 STATUS_KEY 可能是旧的",
    notFoundWhy: "这个地址上没有 Bridge",
    noStatusKeyWhy: "Bridge 上还没有装 STATUS_KEY",
    garbledWhy: "读不出 Bridge 的回复，可能有代理挡在中间",
    refusedWhy: (code) => `Bridge 没有给出状态（${code}）`,
    neverCameWhy: "Bridge 已连接，但尚未收到 Toglet 状态",
    staleWhy: "暂未收到电脑的最新状态",
    unboundWhy: "电脑上尚未绑定任务",

    lastState: "上次状态 · 已过期",
    noStateNoAction: "暂时无法获取任务状态，操作已禁用。",
    checkAddress: "请检查 Bridge 地址是否以 /toglet 结尾，并确认已开启远程控制。",

    nothingToContinue: "任务正在运行，无需继续。",
    quotaNothing: "额度恢复后会自动继续。",

    continue: "继续",
    pause: "暂停",
    cancel: "长按取消",
    cancelTask: "长按取消任务",
    releasing: "松开取消",
    cancelStep1: "取消任务",
    cancelStep2: "确认取消",
    retry: "重试",

    trace: "继续请求 · 发送中",
    traceSend: "回复 · 发送中",

    composerHint: "写一句话\u2026",
    send: "发送",
    phrases: "快捷短语",
    phrasesTag: "本机",
    phrasesFine: "每行一条。只存在这台手机上，不会经过 Bridge。",
    counter: (n, max) => `${n} / ${max}`,

    excerptHead: "CODEX 最后说",
    excerptNone: "暂无内容预览",
    // 刻意不指明原因：回执里的 null 同时表示开关关着、agent 这一轮没说话、密封失败三件事。
    excerptNoneWhy: "可能是电脑上没开启预览，也可能是这一轮还没有新消息。",
    excerptSealed: "内容预览无法解密",
    excerptSealedWhy: "这台手机上的共享密钥与电脑上的不一致。",
    delivered: "已发送",
    collected: "已接收",
    applied: "已执行",
    asleep: "电脑可能处于休眠状态，唤醒后将继续处理。",

    syncedAgo: (text) => `${text}同步`,
    lastSync: (time) => `上次同步：${time}`,
    neverSynced: "尚未同步",

    secondsAgo: (n) => `${n} 秒前`,
    minutesAgo: (n) => `${n} 分钟前`,
    hoursAgo: (n) => `${n} 小时前`,

    reasons: {
      completed: "本轮已完成",
      user_stopped: "任务被手动停止",
      interrupted: "任务被中断",
      network: "网络连接异常",
      manual_switch: "账户已手动切换",
      desktop_reopened: "Codex 已重新启动",
      query_failures: "额度状态获取失败",
      status_unknown: "无法识别任务结束状态",
      failed_unknown_reason: "任务执行失败",
      waiting_on_human: "等待你的输入",
      reauth_required: "账户需要重新登录",
      unauthorized: "登录状态已失效",
      identity_mismatch: "当前登录账户与预期不一致",
      thread_unavailable: "无法恢复当前会话",
      max_resumes: "已达到最大续跑次数",
      deadline: "已达到任务截止时间",
      no_account_available: "暂无可用账户",
    },

    outcomes: {
      applied: "已执行",

      remote_bad_mac: "请求被拒绝：签名验证失败",
      remote_replayed: "请求被拒绝：请求已处理",
      remote_nonce_reused: "请求被拒绝：请求已处理",
      remote_expired: "请求被拒绝：请求已过期",
      remote_session_mismatch: "请求被拒绝：任务不匹配",
      remote_state_changed: "请求被拒绝：任务状态已变化",
      remote_rate_limited: "请求过于频繁，请稍后再试",
      remote_version: "请求被拒绝：版本不兼容",
      remote_malformed: "请求被拒绝：格式错误",
      remote_unknown_action: "请求被拒绝：不支持此操作",
      remote_unavailable: "操作未执行：任务当前不可用",
    },
  },
};
const T = ZH ? COPY.zh : COPY.en;

const REDUCED = window.matchMedia("(prefers-reduced-motion: reduce)");
const SVGNS = "http://www.w3.org/2000/svg";

// ---------------------------------------------------------------- the matrix
//
// Which states a continue can actually fix. Anything not listed gets no continue button - not a
// disabled one.

const CONTINUABLE_REASONS = new Set([
  "completed",
  "user_stopped",
  "interrupted",
  "network",
  "manual_switch",
  "desktop_reopened",
  "query_failures",
  "status_unknown",
  "failed_unknown_reason",
]);

const BUSY_STATES = new Set([
  "running",
  "resuming",
  "switching",
  "verifying",
  "selecting",
  "armed",
  "waiting_network",
]);

/**
 * Maps a receipt to what the screen shows. `why` separates the four causes of "unknown", so the
 * page does not blame the network when the bridge answered but Toglet never posted.
 */
function shape(receipt, stale, reachable, failure = null) {
  // A refusal carries its own reason; only a request that never got an answer is "unreachable".
  if (reachable === false)
    return { kind: "unknown", why: failure?.why ?? "unreachable", code: failure?.code ?? null };
  if (receipt === null) return { kind: "unknown", why: "neverCame" };
  if (stale) return { kind: "unknown", why: "stale" };

  const { state, waitReason } = receipt;
  if (state === "paused") return { kind: "paused", canContinue: true };
  if (state === "waiting_quota") return { kind: "waiting" };
  if (state === "round_completed")
    return { kind: "stopped", canContinue: true, reason: "completed" };
  if (BUSY_STATES.has(state)) return { kind: "running" };
  if (state === "needs_human") {
    return CONTINUABLE_REASONS.has(waitReason)
      ? { kind: "stopped", canContinue: true, reason: waitReason }
      : { kind: "needsYou", canContinue: false, reason: waitReason };
  }
  // `disabled` and `stopped`: nothing is bound, so nothing here can act on it.
  return { kind: "unknown", why: "unbound" };
}

// ---------------------------------------------------------------- pairing (this phone only)

const STORE = "toglet.remote.pairing";

function pairing() {
  try {
    return JSON.parse(localStorage.getItem(STORE) ?? "null");
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- signing

async function sign(secret, message) {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(message));
  return [...new Uint8Array(signature)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

function randomHex(bytes) {
  const buffer = new Uint8Array(bytes);
  crypto.getRandomValues(buffer);
  return [...buffer].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * The exact byte string Toglet signs. Fields joined by newlines in a fixed order - not
 * canonicalised JSON, which differs between implementations in key order and escaping.
 */
async function envelope(action, receipt, counter, secret, text = "") {
  const nonce = randomHex(16);
  const issuedAt = Math.floor(Date.now() / 1000);
  const body = {
    v: 2,
    kind: "command",
    action,
    sessionId: receipt.sessionId,
    observedState: receipt.state,
    counter,
    nonce,
    issuedAt,
  };
  const signed = [
    "toglet-remote/2",
    "command",
    action,
    body.sessionId,
    body.observedState,
    String(counter),
    nonce,
    String(issuedAt),
    // Last, and always signed - empty for the argument-free actions. Signing it is what stops
    // the bridge rewriting the one thing on this path worth rewriting.
    text,
  ].join("\n");
  const mac = await sign(secret, signed);
  // Absent and empty sign identically, so only `send` carries the field at all.
  if (text !== "") body.text = text;
  return { ...body, mac };
}

// ---------------------------------------------------------------- the excerpt
//
// HMAC proves who sent a receipt but hides nothing inside it, and the bridge is untrusted: its
// status endpoint hands the last receipt to whoever asks. So the one piece of session content
// allowed out travels sealed, under a key derived from the shared secret but separated from the
// one that signs it. This mirrors src-tauri/src/remote/crypt.rs; the two must agree byte for byte.

/** Domain separator. The bytes that verify a receipt are never the bytes that open one. */
const EXCERPT_DOMAIN = "toglet-remote/2 excerpt";

/**
 * The key the bridge checks a `/status` read against, derived one-way from the shared secret.
 *
 * The bridge needs to authenticate readers - a receipt can carry a sealed excerpt, so handing it
 * to whoever asks is a leak - but it must not become able to forge a command. A separate derived
 * key gives it exactly the first and none of the second: SHA-256 does not run backwards, and the
 * command MAC uses the raw secret this machine never sees.
 */
const STATUS_DOMAIN = "toglet-remote/2 status";

async function statusHeaders(secret) {
  const issuedAt = Math.floor(Date.now() / 1000);
  const material = new TextEncoder().encode(STATUS_DOMAIN + secret);
  const raw = await crypto.subtle.digest("SHA-256", material);
  const key = await crypto.subtle.importKey("raw", raw, { name: "HMAC", hash: "SHA-256" }, false, [
    "sign",
  ]);
  // Its own kind in the signed bytes, so a status MAC can never be replayed as a command.
  const signed = ["toglet-remote/2", "status", String(issuedAt)].join("\n");
  const signature = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(signed));
  const mac = [...new Uint8Array(signature)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  return { "x-toglet-ts": String(issuedAt), "x-toglet-mac": mac };
}

/** Lower- or upper-case hex of even length; `null` for anything else. */
function fromHex(hex) {
  if (typeof hex !== "string" || hex.length === 0 || hex.length % 2 !== 0) return null;
  if (!/^[0-9a-fA-F]+$/.test(hex)) return null;
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/**
 * The excerpt in clear, or `null` when it does not authenticate. Never throws and never guesses:
 * GCM checks the tag before returning anything, so a tampered value yields nothing at all rather
 * than plausible-looking rubbish.
 */
async function openExcerpt(secret, ciphertextHex, nonceHex) {
  const ciphertext = fromHex(ciphertextHex);
  const nonce = fromHex(nonceHex);
  if (ciphertext === null || nonce === null || nonce.length !== 12) return null;
  try {
    // UTF-8 of a concatenation is the concatenation of the encodings, so this is byte-identical
    // to Rust hashing the separator and the secret in two updates.
    const material = new TextEncoder().encode(EXCERPT_DOMAIN + secret);
    const digest = await crypto.subtle.digest("SHA-256", material);
    const key = await crypto.subtle.importKey("raw", digest, { name: "AES-GCM" }, false, [
      "decrypt",
    ]);
    const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: nonce }, key, ciphertext);
    return new TextDecoder().decode(plain);
  } catch {
    // Failing to authenticate is expected behaviour on an untrusted path, not a fault.
    return null;
  }
}

/**
 * What the excerpt block should say. A `null` field is NOT reported as "preview is off": it also
 * means the agent has not spoken this turn, and the receipt cannot tell those apart (BACKLOG-051).
 * Claiming a cause the data does not carry is the same lie as an empty string posing as silence.
 */
async function readExcerpt(secret, receipt) {
  const ciphertext = receipt?.excerptCiphertext ?? null;
  const nonce = receipt?.excerptNonce ?? null;
  if (ciphertext === null || nonce === null) return { kind: "none" };
  const text = await openExcerpt(secret, ciphertext, nonce);
  return text === null ? { kind: "sealed" } : { kind: "text", text };
}

// ---------------------------------------------------------------- quick phrases (this phone only)
//
// Stored here and nowhere else. They never enter a receipt or a command envelope - a phrase is
// just ordinary text once it is sent - so adding them cost the wire format nothing.

const PHRASE_STORE = "toglet.remote.phrases";
const DEFAULT_PHRASES = ZH
  ? ["继续", "按你的推荐来", "先跑一遍测试"]
  : ["Continue", "Go with your recommendation", "Run the tests first"];

function phrases() {
  try {
    const held = JSON.parse(localStorage.getItem(PHRASE_STORE) ?? "null");
    if (!Array.isArray(held)) return DEFAULT_PHRASES;
    const kept = held.filter((line) => typeof line === "string" && line.trim() !== "");
    return kept.length === 0 ? DEFAULT_PHRASES : kept;
  } catch {
    return DEFAULT_PHRASES;
  }
}

function savePhrases(text) {
  const lines = text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "");
  try {
    localStorage.setItem(PHRASE_STORE, JSON.stringify(lines));
  } catch {
    // A phone with storage blocked still works; it just falls back to the defaults.
  }
}

// ---------------------------------------------------------------- state

const app = document.getElementById("app");

let model = {
  receipt: null,
  ageSeconds: null,
  reachable: null,
  /** Why the last read was refused - `{ why, code }` - or `null` for none and for a dead network. */
  failure: null,
  /** `{ counter, action, at, collected, done }` while a command is on its way. */
  inFlight: null,
  /** Set for one render after the bridge comes back, to play the Live Core's scan. */
  reconnected: false,
  /** What is typed but not yet sent. Held here so a redraw cannot throw it away. */
  draft: "",
  /** `{ kind: "none" | "sealed" | "text", text? }` - decrypting is async, drawing is not. */
  excerpt: { kind: "none" },
};

async function refresh() {
  const paired = pairing();
  if (paired === null) return;
  const was = model.reachable;
  // Whether an answer came back at all, and what it said. Without these the failure below cannot
  // be told apart from a dead network, and each of them is a different repair: a stale key on the
  // bridge, a wrong address, a bridge with no key installed. Collapsing them into one sentence is
  // what made a working deployment look broken.
  let answered = false;
  let code = 0;
  try {
    const response = await fetch(new URL("status", paired.endpoint), {
      cache: "no-store",
      headers: await statusHeaders(paired.secret),
    });
    answered = true;
    code = response.status;
    if (!response.ok) throw new Error("refused");
    const body = await response.json();
    model.receipt = body.receipt ?? null;
    model.ageSeconds = body.ageSeconds;
    model.reachable = true;
    model.failure = null;
    model.reconnected = was === false;
    model.excerpt = await readExcerpt(paired.secret, model.receipt);

    // The third node lights only when Toglet says so - never before.
    const last = model.receipt?.lastCommand ?? null;
    if (model.inFlight !== null && last !== null && last.counter >= model.inFlight.counter) {
      model.inFlight = { ...model.inFlight, collected: true, done: last.result };
      setTimeout(() => {
        if (model.inFlight?.done) {
          model.inFlight = null;
          render();
        }
      }, 2600);
    } else if (model.inFlight !== null && (model.receipt?.cursor ?? -1) >= model.inFlight.counter) {
      model.inFlight = { ...model.inFlight, collected: true };
    }
  } catch {
    model.reachable = false;
    model.failure = answered ? refusal(code) : null;
    model.reconnected = false;
  }
  render();
}

/**
 * Why a `/status` read was refused, from its status code. `401` is the one worth spelling out:
 * the bridge holds a STATUS_KEY derived from a different secret, and the usual cause is a secret
 * regenerated on the computer without redeploying the bridge.
 */
function refusal(code) {
  if (code === 401) return { why: "unauthorized", code };
  if (code === 404) return { why: "notFound", code };
  if (code === 503) return { why: "noStatusKey", code };
  // An answer that arrived and could not be read: most often a proxy serving its own page.
  if (code >= 200 && code < 300) return { why: "garbled", code };
  return { why: "refused", code };
}

async function deliver(action, text = "") {
  const paired = pairing();
  if (paired === null || model.receipt === null) return;
  const counter = (model.receipt.cursor ?? 0) + 1;
  const body = await envelope(action, model.receipt, counter, paired.secret, text);
  model.inFlight = { counter, action, at: new Date(), collected: false, done: null };
  render();
  try {
    await fetch(new URL("command", paired.endpoint), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
  } catch {
    model.inFlight = null;
    model.reachable = false;
    // The request itself never completed, so there is no refusal to report.
    model.failure = null;
  }
  render();
  void refresh();
}

// ---------------------------------------------------------------- drawing

function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function svgNode(tag, attributes) {
  const node = document.createElementNS(SVGNS, tag);
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, String(value));
  return node;
}

/** Outline icon, stroke 1.75. One helper keeps a single icon style. */
function icon(size, paths, stroke = "var(--t3)", width = 1.75) {
  const node = svgNode("svg", {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke,
    "stroke-width": width,
    "stroke-linecap": "round",
    "stroke-linejoin": "round",
  });
  for (const d of paths) node.append(svgNode("path", { d }));
  return node;
}

const BOLT = ["M13 3 5.5 13H11l-1 8 8.5-10.5H13z"];
const DIAMOND = ["M12 3.2 20 12l-8 8.8L4 12z"];
const BROKEN_LINK = [
  "M9 7 5.5 10.5a4.6 4.6 0 0 0 6.5 6.5L15 13.5",
  "M15 17 18.5 13.5a4.6 4.6 0 0 0-6.5-6.5L9 10.5",
];
const CHEVRON = ["M7 7l5 5-5 5"];

/** The tone each state paints its Signal, Core mark and header mark in. */
const TONE = {
  running: "var(--mint)",
  waiting: "var(--amber)",
  stopped: "var(--coral)",
  needsYou: "var(--amber)",
  paused: null,
  unknown: null,
  unpaired: null,
};

/**
 * The Live Core: orbit r76 (circumference 477.5), core r36 (r48 under a countdown), and a gap in
 * the track so it never reads as a progress ring.
 */
function liveCore(view) {
  const kind = view.kind;
  const wrap = element("div", model.inFlight === null ? "core" : "core small");
  const tone = TONE[kind];
  const still = REDUCED.matches;

  const glow = {
    running: ["rgba(var(--mint-rgb),.12)", 290, "tgGlow 3.2s ease-in-out infinite"],
    waiting: ["rgba(var(--amber-rgb),.10)", 290, "tgGlow 4.6s ease-in-out infinite"],
    stopped: ["rgba(var(--coral-rgb),.07)", 250, null],
    needsYou: ["rgba(var(--amber-rgb),.08)", 260, null],
  }[kind];
  if (glow !== undefined && model.inFlight === null) {
    const layer = element("div", "glow");
    layer.style.setProperty("--glow-size", `${glow[1]}px`);
    layer.style.background = `radial-gradient(circle, ${glow[0]}, transparent 66%)`;
    if (glow[2] !== null && !still) layer.style.animation = glow[2];
    wrap.append(layer);
  }

  const svg = svgNode("svg", { viewBox: "0 0 200 200", role: "img" });
  svg.setAttribute("aria-label", spoken(view));

  const faint = kind === "unknown" || kind === "unpaired" || kind === "paused";
  svg.append(
    svgNode("circle", {
      cx: 100,
      cy: 100,
      r: 76,
      fill: "none",
      stroke: faint ? "var(--orbit-faint)" : "var(--orbit)",
      "stroke-width": 1.25,
      "stroke-linecap": "round",
      // Dashed while there is nothing live to show; otherwise a track with one gap in it.
      "stroke-dasharray": kind === "unpaired" ? "5 9" : kind === "unknown" ? "3 10" : "404 73.5",
      transform: "rotate(-96 100 100)",
    }),
  );

  if (tone !== null) {
    const waiting = kind === "waiting";
    const signal = svgNode("circle", {
      cx: 100,
      cy: 100,
      r: 76,
      fill: "none",
      stroke: tone,
      "stroke-width": waiting || kind === "needsYou" ? 2.4 : model.inFlight ? 3.2 : 2.75,
      "stroke-linecap": "round",
      // Waiting breathes the whole arc; needs-you splits it in two; the rest is a 42px segment.
      "stroke-dasharray": waiting ? "404 73.5" : kind === "needsYou" ? "150 88.7" : "42 435.5",
      transform: `rotate(${waiting ? -96 : kind === "needsYou" ? -158 : kind === "stopped" ? -38 : -90} 100 100)`,
    });
    if (kind === "needsYou") signal.setAttribute("opacity", ".85");
    if (still || kind === "stopped" || kind === "needsYou") {
      svg.append(signal);
    } else {
      const group = svgNode("g", {});
      group.style.transformOrigin = "100px 100px";
      group.style.animation = waiting
        ? "tgSlowPulse 3.4s ease-in-out infinite"
        : "tgOrbit 2.4s linear infinite";
      group.append(signal);
      svg.append(group);
    }
  }

  const inner = svgNode("circle", {
    cx: 100,
    cy: 100,
    r: kind === "waiting" ? 48 : 36,
    fill:
      kind === "unpaired" || kind === "unknown"
        ? "none"
        : kind === "running"
          ? "rgba(var(--mint-rgb),.035)"
          : kind === "waiting"
            ? "rgba(var(--amber-rgb),.03)"
            : "var(--core-fill)",
    stroke: faint ? "var(--orbit-faint)" : "var(--core-line)",
    "stroke-width": 1,
  });
  if (kind === "unpaired") inner.setAttribute("stroke-dasharray", "4 7");
  if (kind === "unknown") inner.setAttribute("stroke-dasharray", "3 8");
  if (kind === "running" && !still) {
    const group = svgNode("g", {});
    group.style.transformOrigin = "100px 100px";
    group.style.animation = "tgBreathe 2.8s ease-in-out infinite";
    group.append(inner);
    svg.append(group);
  } else {
    svg.append(inner);
  }
  wrap.append(svg);

  wrap.append(coreMark(kind));
  if (model.reconnected && !still) wrap.append(element("div", "scan"));
  return wrap;
}

/** The centre symbol, so the state is readable without reading the words. */
function coreMark(kind) {
  if (kind !== "waiting") ticking.countdown = null;
  if (kind === "waiting") {
    const centre = element("div", "centre stack");
    // Never derived from a percentage and never guessed: unknown is an em dash. The text is
    // written by paintCountdown, which is also what the one-second tick calls.
    ticking.countdown = element("span", "countdown");
    ticking.countdownLabel = element("span", "countdown-label");
    centre.append(ticking.countdown, ticking.countdownLabel);
    paintCountdown();
    return centre;
  }
  const centre = element("div", "centre");
  if (kind === "running") centre.append(element("span", "live-dot"));
  else if (kind === "stopped") centre.append(element("span", "stop-bar"));
  else if (kind === "unknown") centre.append(element("span", "dead-bar"));
  else if (kind === "unpaired") centre.append(icon(22, BROKEN_LINK));
  else if (kind === "paused") {
    const mark = element("div", "pause-mark");
    mark.append(element("i"), element("i"));
    centre.append(mark);
  } else if (kind === "needsYou") {
    const caret = element("div", "caret");
    caret.append(icon(15, CHEVRON, "var(--amber)", 2.2), element("i"));
    centre.append(caret);
  }
  return centre;
}

// ---------------------------------------------------------------- words and time

function clock(date) {
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

function stamp(date) {
  return `${clock(date)}:${String(date.getSeconds()).padStart(2, "0")}`;
}

function ago(seconds) {
  if (seconds < 60) return T.secondsAgo(Math.max(0, Math.round(seconds)));
  if (seconds < 3600) return T.minutesAgo(Math.round(seconds / 60));
  return T.hoursAgo(Math.round(seconds / 3600));
}

function countdown(seconds) {
  if (seconds <= 0) return "00:00";
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return hours > 0
    ? `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}`
    : `${String(minutes).padStart(2, "0")}:${String(Math.floor(seconds % 60)).padStart(2, "0")}`;
}

/** How old a receipt may be before it stops being the current state, in seconds. */
function staleAfter() {
  const cadence = model.receipt?.nextPollSeconds;
  // Toglet's poll interval depends on its state and is sent as `nextPollSeconds`, so the cut-off
  // is one missed poll plus a grace period.
  if (typeof cadence !== "number" || cadence <= 0) return 120;
  return Math.max(120, cadence + 90);
}

function words(view) {
  switch (view.kind) {
    case "running":
      return [T.running, T.runningWhy(model.receipt?.resumeCount ?? 0)];
    case "waiting":
      return [T.waiting, T.waitingWhy];
    case "paused":
      return [T.paused, T.pausedWhy];
    case "needsYou":
      return [T.needsYou, T.reasons[view.reason] ?? T.needsYouWhy];
    case "stopped":
      return [T.stopped, T.reasons[view.reason] ?? ""];
    case "unpaired":
      return [T.pairTitle, T.pairIntro];
    default:
      return view.why === "unbound" ? [T.unbound, T.unboundWhy] : [T.unavailable, unknownWhy(view)];
  }
}

const UNKNOWN_WHY = {
  unreachable: T.unreachableWhy,
  unauthorized: T.unauthorizedWhy,
  notFound: T.notFoundWhy,
  noStatusKey: T.noStatusKeyWhy,
  garbled: T.garbledWhy,
  refused: T.refusedWhy,
  neverCame: T.neverCameWhy,
  stale: T.staleWhy,
};

/** The line under "state unavailable". One reason takes the status code, so it is a function. */
function unknownWhy(view) {
  const line = UNKNOWN_WHY[view.why] ?? T.unreachableWhy;
  return typeof line === "function" ? line(view.code) : line;
}

/** What a screen reader hears for the Live Core. */
function spoken(view) {
  const [title, why] = words(view);
  return why ? `${title} · ${why}` : title;
}

/**
 * The third line under the status: the recovery time when there is one, otherwise what the last
 * command did. The receipt carries no time the current state began, so none is shown.
 */
function metaLine(view) {
  if (view.kind === "waiting") {
    const at = model.receipt?.expectedAvailableAt ?? null;
    return at === null ? "" : T.waitingAt(clock(new Date(at * 1000)));
  }
  const last = model.receipt?.lastCommand ?? null;
  if (last === null || model.inFlight !== null) return "";
  return T.outcomes[last.result] ?? last.result;
}

// ---------------------------------------------------------------- header

function header(paired, kind) {
  const bar = element("div", "header");

  const tone = TONE[kind];
  const mark = svgNode("svg", { width: 17, height: 17, viewBox: "0 0 20 20" });
  mark.style.display = "block";
  mark.append(
    svgNode("circle", {
      cx: 10,
      cy: 10,
      r: 7.6,
      fill: "none",
      stroke: kind === "unknown" ? "var(--orbit-faint)" : "var(--orbit)",
      "stroke-width": 1.5,
      "stroke-dasharray": "41 6.8",
      "stroke-linecap": "round",
      transform: "rotate(-96 10 10)",
    }),
  );
  if (tone !== null) {
    mark.append(
      svgNode("circle", {
        cx: 10,
        cy: 10,
        r: 7.6,
        fill: "none",
        stroke: tone,
        "stroke-width": 2,
        "stroke-dasharray": "9 38.8",
        "stroke-linecap": "round",
        transform: "rotate(-64 10 10)",
      }),
    );
  }
  mark.append(
    svgNode("circle", {
      cx: 10,
      cy: 10,
      r: 2.4,
      fill: tone ?? (kind === "unknown" ? "var(--muted)" : "var(--offline)"),
    }),
  );

  const brand = element(
    "div",
    `brand${kind === "paused" ? " quiet" : kind === "unknown" ? " dead" : ""}`,
  );
  brand.append(mark, element("span", "", T.brand));
  bar.append(brand);

  const linkClass = linkState(paired);
  const link = element("div", `link ${linkClass}`);
  link.append(element("i"), element("span", "", LINK_WORDS[linkClass]));
  bar.append(link);
  return bar;
}

const LINK_WORDS = {
  idle: T.unlinked,
  down: T.offline,
  busy: T.syncing,
  live: T.connected,
};

/** The connection chip's state. Named so the header's rebuild key can be written from it. */
function linkState(paired) {
  if (paired === null) return "idle";
  if (model.reachable === false) return "down";
  if (model.inFlight !== null || model.reachable !== true) return "busy";
  return "live";
}

// ---------------------------------------------------------------- command trace

/**
 * Three nodes: sent, received, executed. The third lights only when Toglet has said what it did,
 * so nothing on screen reads as success before then.
 */
function trace() {
  const box = element("div", "trace");
  box.append(element("div", "rule"));

  const head = element("div", "trace-head");
  head.append(element("span", "", model.inFlight.action === "send" ? T.traceSend : T.trace));
  head.append(element("time", "", stamp(model.inFlight.at)));
  box.append(head);

  const collected = model.inFlight.collected;
  const done = model.inFlight.done !== null;

  const rail = element("div", "rail");
  const first = element("div", `seg ${collected ? "done" : "active"}`);
  if (!collected) first.append(element("i"));
  const second = element("div", `seg ${done ? "done" : collected ? "active" : ""}`);
  if (collected && !done) second.append(element("i"));
  rail.append(
    element("span", "node done"),
    first,
    element("span", `node${collected ? " done" : ""}`),
    second,
    element("span", `node${done ? " done" : ""}`),
  );
  box.append(rail);

  const labels = element("div", "rail-labels");
  labels.append(
    element("span", "done", T.delivered),
    element("span", collected ? "done" : "", T.collected),
    element(
      "span",
      done ? "done" : "",
      done ? (T.outcomes[model.inFlight.done] ?? model.inFlight.done) : T.applied,
    ),
  );
  box.append(labels);

  // Not collected yet: the computer may be asleep, so say that rather than show a spinner.
  if (!collected) box.append(element("p", "why", T.asleep));

  // The travelling dot has to know how far to travel, and the rail is fluid.
  requestAnimationFrame(() => {
    const span = first.getBoundingClientRect().width;
    if (span > 0) box.style.setProperty("--rail-span", `${span}px`);
  });
  return box;
}

// ---------------------------------------------------------------- cancel

/**
 * Hold to cancel. Nothing is sent until the 600ms sweep completes; letting go early voids it.
 * With reduced motion on, it becomes two taps, so a hold is never the only route.
 */
function cancelButton(wide) {
  const button = element("button", `secondary danger${wide ? " wide" : ""}`);
  const label = element("span", "", wide ? T.cancelTask : T.cancel);

  if (REDUCED.matches) {
    let armed = false;
    let timer = null;
    label.textContent = T.cancelStep1;
    button.append(label);
    button.addEventListener("click", () => {
      if (armed) {
        clearTimeout(timer);
        void deliver("cancel");
        return;
      }
      armed = true;
      button.classList.add("arming");
      label.textContent = T.cancelStep2;
      timer = setTimeout(() => {
        armed = false;
        button.classList.remove("arming");
        label.textContent = T.cancelStep1;
      }, 4000);
    });
    return button;
  }

  const fill = element("span", "fill");
  button.append(fill, label);

  let started = 0;
  let frame = null;

  const stop = (fired) => {
    cancelAnimationFrame(frame);
    frame = null;
    started = 0;
    fill.style.width = "0";
    button.classList.remove("arming");
    app.classList.remove("holding");
    label.textContent = wide ? T.cancelTask : T.cancel;
    if (fired) void deliver("cancel");
  };

  const step = () => {
    const progress = Math.min(1, (performance.now() - started) / 600);
    fill.style.width = `${progress * 100}%`;
    if (progress >= 1) {
      stop(true);
      return;
    }
    frame = requestAnimationFrame(step);
  };

  button.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    button.setPointerCapture(event.pointerId);
    started = performance.now();
    button.classList.add("arming");
    app.classList.add("holding");
    label.textContent = T.releasing;
    frame = requestAnimationFrame(step);
  });
  for (const name of ["pointerup", "pointercancel", "pointerleave"]) {
    button.addEventListener(name, () => {
      if (frame !== null) stop(false);
    });
  }
  return button;
}

// ---------------------------------------------------------------- composer
//
// Where a sentence is actually carried out, mirroring `machine.rs::apply_steer`: waiting on a
// person, paused, or a round that has ended. A running turn cannot be steered and a quota wait
// would fail on arrival, so those screens have no composer at all - not a disabled one.

const COMPOSER_KINDS = new Set(["needsYou", "stopped", "paused"]);

/** The protocol's own limit, from src-tauri/src/remote/envelope.rs. */
const MAX_TEXT_CHARS = 2000;

/** Only shown near the ceiling; a counter on an empty box is noise. */
const COUNTER_FROM = 1600;

function excerptBlock() {
  const surface = element("div", "surface excerpt");
  const head = element("div", "surface-head");
  head.append(element("span", "", T.excerptHead));
  surface.append(head);

  const state = model.excerpt ?? { kind: "none" };
  if (state.kind === "text") {
    surface.append(element("p", "excerpt-body", state.text));
    return surface;
  }
  const sealed = state.kind === "sealed";
  surface.append(
    element("p", "excerpt-empty", sealed ? T.excerptSealed : T.excerptNone),
    element("p", "excerpt-why", sealed ? T.excerptSealedWhy : T.excerptNoneWhy),
  );
  return surface;
}

/** Quick phrases send on the press; they are ordinary text once sent. */
function chipRow() {
  const row = element("div", "chips");
  for (const phrase of phrases()) {
    const chip = element("button", "chip", phrase);
    chip.addEventListener("click", () => {
      model.draft = "";
      void deliver("send", phrase);
    });
    row.append(chip);
  }
  return row;
}

function grow(box) {
  box.style.height = "auto";
  box.style.height = `${Math.min(box.scrollHeight, 96)}px`;
}

function paintCounter(node) {
  const length = model.draft.length;
  node.hidden = length < COUNTER_FROM;
  node.textContent = T.counter(length, MAX_TEXT_CHARS);
  node.classList.toggle("full", length >= MAX_TEXT_CHARS);
}

function composer() {
  const wrap = element("div", "composer");
  const box = document.createElement("textarea");
  box.className = "composer-input";
  box.placeholder = T.composerHint;
  box.rows = 1;
  box.maxLength = MAX_TEXT_CHARS;
  box.spellcheck = false;
  // Restored rather than reset: the excerpt can change under a half-typed reply.
  box.value = model.draft;

  const count = element("span", "counter");
  box.addEventListener("input", () => {
    const had = model.draft.trim() !== "";
    model.draft = box.value.slice(0, MAX_TEXT_CHARS);
    grow(box);
    paintCounter(count);
    // Only when the send button's existence changes, which is the one thing a redraw settles.
    if (had !== (model.draft.trim() !== "")) render();
  });

  wrap.append(box, count);
  paintCounter(count);
  requestAnimationFrame(() => grow(box));
  return wrap;
}

// ---------------------------------------------------------------- dock

/** Class and words for the sync line. It says something different on every poll. */
function syncWords() {
  const seconds = model.ageSeconds;
  const reachable = model.reachable === true;
  const className = `sync${reachable ? (model.inFlight === null ? " live" : " busy") : ""}`;
  if (seconds === null || seconds === undefined || model.receipt === null) {
    return [className, T.neverSynced];
  }
  return [
    className,
    reachable
      ? T.syncedAgo(ago(seconds))
      : T.lastSync(clock(new Date(Date.now() - seconds * 1000))),
  ];
}

function syncLine() {
  const line = element("div", "sync");
  const text = element("span");
  ticking.sync = line;
  ticking.syncText = text;
  line.append(element("i"), text);
  paintSync();
  return line;
}

/** Which buttons exist at all is decided here, and nothing is ever disabled. */
function dock(view) {
  const bar = element("div", view.kind === "needsYou" || model.inFlight ? "dock spaced" : "dock");

  if (view.kind === "unknown") {
    const retry = element("button", "neutral", T.retry);
    retry.addEventListener("click", () => void refresh());
    bar.append(retry);
    // Unreachable is the one screen where the address may be wrong, so it offers unpairing.
    if (model.reachable === false) {
      const unpair = element("button", "quiet", T.forget);
      unpair.addEventListener("click", () => {
        localStorage.removeItem(STORE);
        model = {
          receipt: null,
          ageSeconds: null,
          reachable: null,
          inFlight: null,
          reconnected: false,
          draft: "",
          excerpt: { kind: "none" },
        };
        render();
      });
      bar.append(unpair);
    }
    bar.append(syncLine());
    return bar;
  }

  if (model.inFlight === null) {
    if (view.canContinue === true) {
      const go = element("button", "primary", T.continue);
      go.addEventListener("click", () => void deliver("resume"));
      bar.append(go);
    } else if (view.kind === "running" || view.kind === "waiting") {
      // Says why the main slot is empty instead of showing a button that would do nothing.
      bar.append(
        element("p", "hint", view.kind === "waiting" ? T.quotaNothing : T.nothingToContinue),
      );
    }
  }

  // A sentence, on exactly the screens where Toglet would carry one out. On "needs you" it is
  // the primary action, because "continue" is absent there; elsewhere "continue" is primary and
  // this sits below it, so the screen never has two solid Mint buttons.
  const draft = model.draft.trim();
  if (model.inFlight === null && COMPOSER_KINDS.has(view.kind) && draft !== "") {
    const send = element("button", view.kind === "needsYou" ? "primary" : "secondary wide", T.send);
    send.addEventListener("click", () => {
      model.draft = "";
      void deliver("send", draft);
    });
    bar.append(send);
  }

  if (view.kind === "paused") {
    bar.append(cancelButton(true));
  } else {
    const row = element("div", "row");
    const pause = element("button", "secondary", T.pause);
    pause.addEventListener("click", () => void deliver("pause"));
    row.append(pause, cancelButton(false));
    bar.append(row);
  }

  bar.append(syncLine());
  return bar;
}

// ---------------------------------------------------------------- pairing

function field(label, type, placeholder, tag) {
  const row = element("label", "field");
  row.append(icon(19, type === "url" ? BOLT : DIAMOND));
  const body = element("div", "field-body");
  body.append(element("span", "", label));
  const input = document.createElement("input");
  input.type = type;
  input.placeholder = placeholder;
  input.className = type === "url" ? "mono" : "";
  // Empty means "keep what is stored", so a password manager must not fill it.
  input.autocomplete = type === "password" ? "new-password" : "off";
  input.inputMode = type === "url" ? "url" : "text";
  input.autocapitalize = "off";
  input.spellcheck = false;
  body.append(input);
  row.append(body);
  if (tag !== undefined) row.append(element("span", "tag", tag));
  return { row, input };
}

function pairStage() {
  const stage = element("div", "stage top");
  stage.append(liveCore({ kind: "unpaired" }));
  const title = element("h1", "status pair", T.pairTitle);
  title.style.setProperty("--core-gap", "22px");
  stage.append(title);
  const intro = element("p", "reason", T.pairIntro);
  intro.style.marginTop = "11px";
  stage.append(intro);

  const held = pairing();
  const fields = element("div", "fields");
  const endpoint = field(T.bridge, "url", "https://…");
  const secret = field(T.secret, "password", "", T.writeOnly);
  if (held !== null) endpoint.input.value = held.endpoint;
  const lines = phrasesField();
  fields.append(endpoint.row, secret.row, lines.row);
  stage.append(fields);
  stage.append(element("p", "fine", T.pairFine), element("p", "fine", T.phrasesFine));
  return { stage, endpoint: endpoint.input, secret: secret.input, phrases: lines.input };
}

/** The quick phrases, edited where the rest of this phone's settings live. */
function phrasesField() {
  const row = element("label", "field tall");
  row.append(icon(19, CHEVRON));
  const body = element("div", "field-body");
  body.append(element("span", "", T.phrases));
  const box = document.createElement("textarea");
  box.className = "lines";
  box.rows = 3;
  box.spellcheck = false;
  box.autocapitalize = "off";
  box.value = phrases().join("\n");
  body.append(box);
  row.append(body, element("span", "tag", T.phrasesTag));
  return { row, input: box };
}

function pairDock(inputs) {
  const bar = element("div", "dock");
  const save = element("button", "primary", T.pair);
  save.addEventListener("click", () => {
    const held = pairing();
    const address = inputs.endpoint.value.trim();
    // Trimmed like the address above: a secret pasted with a trailing newline would
    // otherwise differ from the one Toglet stored, which trims, and every MAC would fail.
    const typed = inputs.secret.value.trim();
    const key = typed === "" ? (held?.secret ?? "") : typed;
    if (address === "" || key.length < 16) return;
    savePhrases(inputs.phrases.value);
    localStorage.setItem(
      STORE,
      JSON.stringify({ endpoint: address.endsWith("/") ? address : `${address}/`, secret: key }),
    );
    render();
    void refresh();
  });
  bar.append(save, syncLine());
  return bar;
}

// ---------------------------------------------------------------- render
//
// Four regions, each replaced only when its key - its structure, not its text - changes. A new
// element restarts its CSS animations, so rebuilding on every poll would reset the orbit and the
// pulse. Text that ticks (the countdown, "synced 12s ago") is written into nodes that stay put.

/** The live nodes of each region, and the key each was built from. */
const parts = { header: null, stage: null, context: null, dock: null, home: null };
const keys = {};
/** Nodes whose text changes on its own clock, kept so they can be written to in place. */
const ticking = {
  countdown: null,
  countdownLabel: null,
  sync: null,
  syncText: null,
  expired: null,
};

function place(name, key, build) {
  if (keys[name] === key && parts[name] !== null) return parts[name];
  const made = build();
  if (parts[name] === null) app.append(made);
  else app.replaceChild(made, parts[name]);
  // A rebuilt dock has thrown away the button a hold was running on, so the hold is over.
  if (name === "dock") app.classList.remove("holding");
  parts[name] = made;
  keys[name] = key;
  return made;
}

function render() {
  const paired = pairing();

  if (paired === null) {
    app.classList.remove("offline");
    place("header", "unpaired", () => header(null, "unpaired"));
    // Rebuilding this would throw away whatever is half-typed, so it is built once and kept.
    place("stage", "pair", () => {
      const inputs = pairStage();
      parts.pairInputs = inputs;
      return inputs.stage;
    });
    place("context", "none", () => element("div", "context"));
    place("dock", "pair", () => pairDock(parts.pairInputs));
    place("home", "home", home);
    paintSync();
    return;
  }

  // A receipt that is no longer current must not be shown as the current state.
  const stale =
    model.reachable === false || (model.ageSeconds !== null && model.ageSeconds > staleAfter());
  const view = shape(model.receipt, stale, model.reachable, model.failure);
  app.classList.toggle("offline", view.kind === "unknown" && model.reachable === false);

  const flight = model.inFlight;
  const link = linkState(paired);
  place("header", `${view.kind}|${link}`, () => header(paired, view.kind));

  // The Live Core lives in here, so this key is what decides whether an animation survives.
  place("stage", `${view.kind}|${flight === null ? 0 : 1}|${model.reconnected ? 1 : 0}`, () =>
    buildStage(view),
  );
  paintStage(view);

  place("context", contextKey(view), () => buildContext(view));
  place("dock", dockKey(view), () => dock(view));
  place("home", "home", home);
  paintSync();
  model.reconnected = false;
}

function buildStage(view) {
  const stage = element("div", "stage");
  stage.append(liveCore(view));
  const heading = element(
    "h1",
    `status${view.kind === "paused" ? " soft" : ""}${view.kind === "unknown" ? " dead" : ""}${model.inFlight ? " small" : ""}`,
  );
  heading.setAttribute("aria-live", "polite");
  if (model.inFlight !== null) heading.style.setProperty("--core-gap", "24px");
  const reason = element("p", `reason${view.kind === "unknown" ? " dead" : ""}`);
  const meta = element("p", "meta");
  stage.append(heading, reason, meta);
  parts.heading = heading;
  parts.reason = reason;
  parts.meta = meta;
  return stage;
}

/** Words only. Anything structural belongs in the stage key above. */
function paintStage(view) {
  const [title, why] = words(view);
  parts.heading.textContent = title;
  parts.reason.textContent = why ?? "";
  parts.reason.hidden = !why;
  const meta = metaLine(view);
  parts.meta.textContent = meta;
  parts.meta.hidden = meta === "";
  paintCountdown();
}

/** The one thing that changes every second, written straight into the node that shows it. */
function paintCountdown() {
  if (ticking.countdown === null) return;
  const at = model.receipt?.expectedAvailableAt ?? null;
  const left = at === null ? null : at - Math.floor(Date.now() / 1000);
  ticking.countdown.textContent = left === null ? "—" : countdown(left);
  ticking.countdownLabel.textContent = left === null ? T.waitingUnknown : T.left;
}

function paintSync() {
  if (ticking.sync === null) return;
  const [className, text] = syncWords();
  ticking.sync.className = className;
  ticking.syncText.textContent = text;
  if (ticking.expired !== null) ticking.expired.textContent = ago(model.ageSeconds ?? 0);
}

function contextKey(view) {
  if (model.inFlight !== null) {
    return `trace|${model.inFlight.counter}|${model.inFlight.collected ? 1 : 0}|${model.inFlight.done ?? ""}`;
  }
  if (COMPOSER_KINDS.has(view.kind)) return `ask|${view.kind}|${model.excerpt.kind}`;
  if (view.kind === "unknown") return `unknown|${view.why}|${model.receipt === null ? 0 : 1}`;
  return "none";
}

function buildContext(view) {
  const context = element("div", "context");
  ticking.expired = null;
  if (model.inFlight !== null) {
    context.append(trace());
    return context;
  }
  if (COMPOSER_KINDS.has(view.kind)) {
    context.append(element("div", "rule"), excerptBlock());
    if (view.kind === "needsYou") {
      const ask = element("div", "ask");
      ask.append(element("p", "lead", T.askLead), element("p", "tail", T.askTail));
      context.append(ask);
    }
    context.append(chipRow(), composer());
    return context;
  }
  if (view.kind === "unknown") {
    if (model.receipt !== null) {
      const surface = element("div", "surface");
      const head = element("div", "surface-head");
      const since = element("time", "", ago(model.ageSeconds ?? 0));
      ticking.expired = since;
      head.append(element("span", "", T.lastState), since);
      const body = element("div", "surface-body");
      const [was, wasWhy] = words(shape(model.receipt, false, true));
      body.append(element("b", "", was));
      if (wasWhy) body.append(element("span", "", wasWhy));
      surface.append(head, body);
      context.append(surface);
    }
    context.append(
      // A wrong address earns the same advice as a bridge that answers but never hears from
      // Toglet: both are fixed by checking what was typed, not by waiting.
      element(
        "p",
        "aside",
        view.why === "neverCame" || view.why === "notFound" ? T.checkAddress : T.noStateNoAction,
      ),
    );
  }
  return context;
}

/** Which buttons exist - not what they say. Rebuilding mid-hold would cancel the hold. */
function dockKey(view) {
  if (view.kind === "unknown") return `unknown|${model.reachable === false ? 1 : 0}`;
  const main =
    model.inFlight !== null
      ? "flight"
      : view.canContinue === true
        ? "continue"
        : view.kind === "waiting" || view.kind === "running"
          ? "hint"
          : "bare";
  // Whether there is something to send, not what it says: the button appears on the first
  // character and vanishes on the last, and is never present-but-disabled.
  const sending = COMPOSER_KINDS.has(view.kind) && model.draft.trim() !== "" ? "send" : "-";
  return `${view.kind}|${main}|${sending}`;
}

function home() {
  const strip = element("div", "home");
  strip.append(element("i"));
  return strip;
}

// ---------------------------------------------------------------- start

render();
void refresh();
setInterval(() => void refresh(), 5000);
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) void refresh();
});
// The countdown ticks from the timestamp Toglet sent. It writes one text node; a full render
// here would restart the pulse behind it every second.
setInterval(() => {
  if (!document.hidden) paintCountdown();
}, 1000);
REDUCED.addEventListener("change", render);

if ("serviceWorker" in navigator) {
  navigator.serviceWorker.register("sw.js").catch(() => {
    // No service worker means no home-screen install. The page still works.
  });
}
