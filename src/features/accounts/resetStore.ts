/**
 * Redeeming a reset credit: idle → confirm → running → done | failed.
 *
 * Spending a credit cannot be undone, so nothing is sent before `confirm`, and only `succeeded`
 * counts as success - the other outcomes left the credits alone and are shown as refusals.
 */

import { create } from "zustand";

import { consumeResetCredit } from "../../ipc";
import type { AccountView, IpcFailure, ResetCreditOutcomeView } from "../../types/ipc";
import { useQuota } from "../quotas/store";

export type ResetPhase = "idle" | "confirm" | "running" | "done" | "failed";

export interface ResetState {
  readonly phase: ResetPhase;
  /** The account whose credit is being spent. `null` only while idle. */
  readonly target: AccountView | null;
  /** How many credits were held when the confirmation opened, for the sentence. */
  readonly held: number;
  readonly result: ResetCreditOutcomeView | null;
  readonly failure: IpcFailure | null;

  readonly begin: (target: AccountView, held: number) => void;
  readonly confirm: (nowSeconds: number) => Promise<void>;
  readonly dismiss: () => void;
}

const IDLE = {
  phase: "idle",
  target: null,
  held: 0,
  result: null,
  failure: null,
} as const;

export const useReset = create<ResetState>()((set, get) => ({
  ...IDLE,

  begin: (target, held) => {
    set({ ...IDLE, phase: "confirm", target, held });
  },

  confirm: async (nowSeconds) => {
    const target = get().target;
    if (target === null) {
      return;
    }
    set({ phase: "running", result: null, failure: null });

    const outcome = await consumeResetCredit(target.id, nowSeconds);
    if (!outcome.ok) {
      set({ phase: "failed", failure: outcome.failure });
      return;
    }

    set({ phase: "done", result: outcome.value });
    if (outcome.value.succeeded) {
      // The windows changed, so the held numbers are stale. Re-read rather than guess them.
      void useQuota.getState().load([target.id], nowSeconds);
    }
  },

  dismiss: () => {
    // Not dismissable while the credit is being spent: the answer decides what was used.
    if (get().phase === "running") {
      return;
    }
    set({ ...IDLE });
  },
}));
