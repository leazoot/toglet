/**
 * Switch flow: idle → checking → blocked | confirm → running → done | failed.
 * `step` only advances on Rust events, never a timer; success is `switched === true` only; the
 * active account is never set here, it comes back through `list_accounts` after verification.
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
  readonly verdict: ClientVerdict | null;
  /** How many steps Rust says have finished, 0 to 4. */
  readonly step: number;
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
      // An unanswered probe is a refusal, not a silent pass.
      set({ phase: "failed", failure: clients.failure });
      return;
    }

    // `desktop_only` can be closed and reopened around the switch; `blocked` and `unknown` cannot.
    const blocked = clients.value === "blocked" || clients.value === "unknown";
    set({ phase: blocked ? "blocked" : "confirm", verdict: clients.value });
  },

  confirm: async (nowSeconds) => {
    const target = get().target;
    if (target === null) {
      return;
    }
    set({ phase: "running", step: 0, result: null, failure: null });

    // Subscribe first so no step event is missed between the call and the subscription.
    const stop = await onSwitchStep((step) => {
      set((state) => (state.phase === "running" ? { step } : state));
    });

    try {
      const outcome = await switchAccount(target.id, nowSeconds);
      if (!outcome.ok) {
        set({ phase: "failed", failure: outcome.failure });
        return;
      }
      // The command also resolves with a failure view, which carries the rollback report.
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
    // Not cancellable while the credentials are being replaced.
    if (get().phase === "running") {
      return;
    }
    set({ ...IDLE });
  },

  toggleDetails: () => {
    set((state) => ({ detailsOpen: !state.detailsOpen }));
  },
}));
