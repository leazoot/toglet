/**
 * Quota readings per account. A failed re-read marks the held value stale, never clears it.
 * Read one account at a time: each reading starts a `codex app-server`, and Rust runs one at once.
 */

import { create } from "zustand";

import { refreshQuota } from "../../ipc";
import type { QuotaView } from "../../types/ipc";
import type { Loadable } from "../../types/load";

interface QuotaState {
  readonly quotas: Readonly<Record<string, Loadable<QuotaView>>>;
  /** True while a batch is running. Drives the refresh indicator, never a full-panel spinner. */
  readonly refreshing: boolean;
  readonly load: (accountIds: readonly string[], nowSeconds: number) => Promise<void>;
  readonly forget: (accountId: string) => void;
}

const EXPAND_REFRESH_AGE_SECONDS = 120;

/** Accounts to re-read on expand. One still loading is skipped so it is not queued twice. */
export function dueForRefresh(
  quotas: Readonly<Record<string, Loadable<QuotaView>>>,
  accountIds: readonly string[],
  nowSeconds: number,
): string[] {
  return accountIds.filter((accountId) => {
    const held = quotas[accountId];
    if (held === undefined || held.state === "failed") {
      return true;
    }
    if (held.state === "loading") {
      return false;
    }
    return nowSeconds - held.value.fetchedAt > EXPAND_REFRESH_AGE_SECONDS;
  });
}

export const useQuota = create<QuotaState>()((set, get) => ({
  quotas: {},
  refreshing: false,
  load: async (accountIds, nowSeconds) => {
    if (accountIds.length === 0) {
      return;
    }
    set({ refreshing: true });

    for (const accountId of accountIds) {
      // A re-read keeps the current numbers on screen; only an account with nothing held loads.
      if (get().quotas[accountId] === undefined) {
        setQuota(set, accountId, { state: "loading" });
      }

      const result = await refreshQuota(accountId, nowSeconds);
      if (result.ok) {
        setQuota(set, accountId, { state: "ready", value: result.value });
        continue;
      }

      const held = get().quotas[accountId];
      if (held?.state === "ready") {
        // Keep the cached values but never pass them off as current.
        setQuota(set, accountId, { state: "ready", value: { ...held.value, stale: true } });
      } else {
        setQuota(set, accountId, { state: "failed", failure: result.failure });
      }
    }

    set({ refreshing: false });
  },
  forget: (accountId) => {
    set((state) => ({
      quotas: Object.fromEntries(Object.entries(state.quotas).filter(([id]) => id !== accountId)),
    }));
  },
}));

/** An account never asked about reads as loading. */
export function quotaOf(
  quotas: Readonly<Record<string, Loadable<QuotaView>>>,
  accountId: string | null,
): Loadable<QuotaView> {
  if (accountId === null) {
    return { state: "loading" };
  }
  return quotas[accountId] ?? { state: "loading" };
}

function setQuota(
  set: (partial: (state: QuotaState) => Partial<QuotaState>) => void,
  accountId: string,
  value: Loadable<QuotaView>,
): void {
  set((state) => ({ quotas: { ...state.quotas, [accountId]: value } }));
}
