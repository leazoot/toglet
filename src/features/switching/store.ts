/**
 * The switch flow.
 *
 * ```text
 * idle → checking → blocked          (a Codex session is still running)
 *                 → confirm → running → done | failed
 * ```
 *
 * Three rules this file exists to keep:
 *
 * * **The step count only ever comes from Rust.** `step` moves when an event says a step
 *   finished, never on a timer. A progress display that walks forward while the work is stuck is
 *   the specific lie this project refuses to tell.
 * * **Success is `switched === true` and nothing else.** Not "the call resolved" - the command
 *   resolves with a failure view too, and a switch that landed while Codex kept the old
 *   credentials is not a plain success either.
 * * **The active account is never updated here.** It comes back through `list_accounts`, which
 *   reads what Rust recorded after verification.
 */

import { create } from "zustand";

import { inspectClients, onSwitchStep, switchAccount } from "../../ipc";
import type { AccountView, ClientVerdict, IpcFailure, SwitchView } from "../../types/ipc";

export type SwitchPhase =
  "idle" | "checking" | "blocked" | "confirm" | "running" | "done" | "failed";

export interface SwitchState {
  readonly phase: SwitchPhase;
  /** The account being switched to. `null` only while idle. */
  readonly target: AccountView | null;
  /** Why the switch was blocked, when it was. */
  readonly verdict: ClientVerdict | null;
  /** How many steps Rust says have finished, 0 to 4. */
  readonly step: number;
  /** The outcome, once there is one. */
  readonly result: SwitchView | null;
  /** Set when the call itself did not get through, as opposed to a switch that failed. */
  readonly failure: IpcFailure | null;
  readonly detailsOpen: boolean;

  readonly begin: (target: AccountView) => Promise<void>;
  readonly confirm: (nowSeconds: number) => Promise<void>;
  readonly cancel: () => void;
  readonly toggleDetails: () => void;
}

const IDLE = {
  phase: "idle",
  target: null,
  verdict: null,
  step: 0,
  result: null,
  failure: null,
  detailsOpen: false,
} as const;

export const useSwitching = create<SwitchState>()((set, get) => ({
  ...IDLE,

  begin: async (target) => {
    set({ ...IDLE, phase: "checking", target });

    const clients = await inspectClients();
    if (!clients.ok) {
      // The probe itself did not answer. Offering to switch anyway would be offering an
      // action nobody checked - so this is a refusal, not a silent pass.
      set({ phase: "failed", failure: clients.failure });
      return;
    }

    // `desktop_only` means the client can be closed and reopened around the switch, which is
    // what the switch does. `blocked` and `unknown` both mean "do not touch the credentials".
    const blocked = clients.value === "blocked" || clients.value === "unknown";
    set({ phase: blocked ? "blocked" : "confirm", verdict: clients.value });
  },

  confirm: async (nowSeconds) => {
    const target = get().target;
    if (target === null) {
      return;
    }
    set({ phase: "running", step: 0, result: null, failure: null });

    // Subscribed before the switch starts: a step that finished between the call and the
    // subscription would otherwise never be shown.
    const stop = await onSwitchStep((step) => {
      set((state) => (state.phase === "running" ? { step } : state));
    });

    try {
      const outcome = await switchAccount(target.id, nowSeconds);
      if (!outcome.ok) {
        set({ phase: "failed", failure: outcome.failure });
        return;
      }
      // `switched` is the only thing that makes this a success. The command resolves with a
      // failure view as well, and that view is what carries the rollback report.
      set({
        phase: outcome.value.switched ? "done" : "failed",
        result: outcome.value,
        step: outcome.value.progress,
      });
    } finally {
      stop();
    }
  },

  cancel: () => {
    // A switch already under way is not cancellable - the credentials are being replaced. Only
    // the states before it, and the states after it, can be dismissed.
    if (get().phase === "running") {
      return;
    }
    set({ ...IDLE });
  },

  toggleDetails: () => {
    set((state) => ({ detailsOpen: !state.detailsOpen }));
  },
}));
