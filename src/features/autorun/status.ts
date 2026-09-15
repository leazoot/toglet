/**
 * The status line and its controls, read only from fields Rust wrote. "Expected 03:00" is just
 * `expectedAvailableAt` formatted; when it is unknown the line says so. No local countdown.
 */

import type { MessageKey, MessageParams } from "../../i18n";
import type { AutoRunState, AutoRunView } from "../../types/ipc";
import type { AutoRunControl } from "./store";

/** The colour of the dot beside the line; never the only carrier of meaning. */
export type StatusTone = "ok" | "warn" | "bad" | "mute";

export interface StatusLine {
  readonly tone: StatusTone;
  readonly key: MessageKey;
  readonly params: MessageParams;
}

/** Turns an account id into the name to show, or `null` when the list does not hold it. */
export type NameOf = (accountId: string) => string | null;

export type ClockOf = (atSeconds: number) => string;

/** Reason codes with a sentence of their own. Any other code is shown verbatim, not guessed at. */
const REASON_KEYS: Readonly<Record<string, MessageKey>> = {
  waiting_on_human: "autorun.reason.waitingOnHuman",
  no_account_available: "autorun.reason.noAccountAvailable",
  query_failures: "autorun.reason.queryFailures",
  manual_switch: "autorun.reason.manualSwitch",
  desktop_reopened: "autorun.reason.desktopReopened",
  max_resumes: "autorun.reason.maxResumes",
  deadline: "autorun.reason.deadline",
  app_restarted: "autorun.reason.appRestarted",
  client_running: "autorun.reason.clientRunning",
  client_shutdown_timeout: "autorun.reason.clientShutdownTimeout",
  switch_verification_mismatch: "autorun.reason.identityMismatch",
  thread_unavailable: "autorun.reason.threadUnavailable",
  reauth_required: "autorun.reason.reauthRequired",
  unauthorized: "autorun.reason.unauthorized",
  usage_limit_exceeded: "autorun.reason.usageLimit",
  network: "autorun.reason.network",
};

export function reasonKey(code: string): MessageKey | null {
  return REASON_KEYS[code] ?? null;
}

/** The line for a plan that is on, or `null` when disabled. `t` is the caller's translator. */
export function statusLine(
  plan: AutoRunView,
  nameOf: NameOf,
  clockOf: ClockOf,
  t: (key: MessageKey, params?: MessageParams) => string,
): StatusLine | null {
  const reason =
    plan.waitReason === null
      ? null
      : (() => {
          const key = reasonKey(plan.waitReason);
          return key === null ? plan.waitReason : t(key);
        })();

  switch (plan.state) {
    case "disabled":
      return null;
    case "armed":
      return { tone: "mute", key: "autorun.status.armed", params: {} };
    case "selecting":
      return { tone: "mute", key: "autorun.status.selecting", params: {} };
    case "waiting_quota": {
      const when =
        plan.expectedAvailableAt === null
          ? t("autorun.expectedUnknown")
          : t("autorun.expectedAt", { time: clockOf(plan.expectedAvailableAt) });
      // Only the last result names the account being waited for; nothing is guessed without it.
      const waitedFor =
        plan.lastResult?.kind === "waited" && plan.lastResult.accountId !== null
          ? nameOf(plan.lastResult.accountId)
          : null;
      return waitedFor === null
        ? { tone: "mute", key: "autorun.status.waiting", params: { when } }
        : { tone: "mute", key: "autorun.status.waitingFor", params: { when, name: waitedFor } };
    }
    case "verifying":
      return { tone: "mute", key: "autorun.status.verifying", params: {} };
    // A reason while switching or resuming means a transient refusal being retried; say so
    // rather than implying progress.
    case "switching":
      return reason === null
        ? { tone: "mute", key: "autorun.status.switching", params: {} }
        : { tone: "warn", key: "autorun.status.switchingBlocked", params: { reason } };
    case "resuming":
      return reason === null
        ? { tone: "mute", key: "autorun.status.resuming", params: {} }
        : { tone: "warn", key: "autorun.status.resumingBlocked", params: { reason } };
    case "running": {
      const name = plan.executingAccountId === null ? null : nameOf(plan.executingAccountId);
      return name === null
        ? { tone: "ok", key: "autorun.status.running", params: {} }
        : { tone: "ok", key: "autorun.status.runningAs", params: { name } };
    }
    case "waiting_network":
      return { tone: "warn", key: "autorun.status.waitingNetwork", params: {} };
    case "round_completed":
      return { tone: "ok", key: "autorun.status.roundCompleted", params: {} };
    case "needs_human":
      return {
        tone: "warn",
        key: "autorun.status.needsHuman",
        params: { reason: reason ?? t("autorun.reason.unknown") },
      };
    case "paused":
      return reason === null
        ? { tone: "mute", key: "autorun.status.paused", params: {} }
        : { tone: "mute", key: "autorun.status.pausedBecause", params: { reason } };
    case "stopped":
      return reason === null
        ? { tone: "mute", key: "autorun.status.stopped", params: {} }
        : { tone: "mute", key: "autorun.status.stoppedBecause", params: { reason } };
  }
}

/** States in which the scheduler has something in flight and can be paused. */
const ACTIVE: ReadonlySet<AutoRunState> = new Set<AutoRunState>([
  "armed",
  "selecting",
  "waiting_quota",
  "verifying",
  "switching",
  "resuming",
  "running",
  "waiting_network",
]);

/** Pause for a plan with something in flight, resume otherwise; cancel whenever it is on. */
export function controlsFor(state: AutoRunState): readonly AutoRunControl[] {
  if (state === "disabled") {
    return [];
  }
  return ACTIVE.has(state) ? ["pause", "cancel"] : ["resume", "cancel"];
}

/**
 * States in which `executingAccountId` is the account acted as right now. Elsewhere it is the
 * last attempt's account, and tagging it "continuing" would contradict the status line.
 */
const EXECUTING: ReadonlySet<AutoRunState> = new Set<AutoRunState>([
  "verifying",
  "switching",
  "resuming",
  "running",
  "waiting_network",
]);

/** Whether the row for `accountId` should carry the "continuing" tag. */
export function isExecuting(plan: AutoRunView | null, accountId: string): boolean {
  return (
    plan !== null &&
    plan.enabled &&
    plan.executingAccountId === accountId &&
    EXECUTING.has(plan.state)
  );
}
