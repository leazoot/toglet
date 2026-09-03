/**
 * The quota reading for each account.
 *
 * A reading that fails does not clear the one before it: an old number that is marked as old is
 * more useful than no number, and clearing would be indistinguishable from "you have none".
 * Whether what is held is still fresh is decided by `isStale` against the current clock, not by
 * a flag set when it arrived.
 *
 * Accounts are read **one at a time**. Each reading starts a `codex app-server` in a throwaway
 * home, and Rust allows only one of those at once; asking for ten at once would not make them
 * arrive any sooner.
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
  /** Drops what is held for an account that no longer exists. */
  readonly forget: (accountId: string) => void;
}

/** How old a reading may be before opening the panel re-reads it: two minutes. */
export const EXPAND_REFRESH_AGE_SECONDS = 120;

/**
 * The accounts whose reading is missing, failed, or older than the allowance for opening the
 * panel. One still on its way is left alone - asking again would only queue a second reading
 * behind the first.
 */
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
      // Only an account with nothing held yet shows as loading. One that is being re-read keeps
      // its current numbers on screen: refreshing never puts the panel into a loading state.
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
        // The values stay; what changes is the claim about them. Marking the reading stale is a
        // statement of fact - this is no longer a fresh answer - and it is the difference between
        // showing a cached number and passing one off as current.
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

/** What is known about one account's quota. An account never asked about is still loading. */
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
