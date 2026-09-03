/**
 * The account list, mirrored from Rust.
 *
 * Rust owns the truth, including which account is active: `isActive` is copied from the
 * backend and never derived here from "a switch succeeded, so it must be this one". Nothing in
 * this store is optimistic.
 */

import { create } from "zustand";

import { listAccounts, removeAccount } from "../../ipc";
import type { AccountView, RollbackReport } from "../../types/ipc";
import type { Loadable } from "../../types/load";

/**
 * A removal under way or just finished. `orphaned` is the honest half-result: the account is
 * gone from the list, but the credential store kept its entry. `rollback` on a failure says
 * what happened to Codex's sign-in when the account in use was being signed out; `null` when
 * nothing of Codex's was touched.
 */
export type Removal =
  | { readonly phase: "removing"; readonly accountId: string; readonly signingOut: boolean }
  | { readonly phase: "failed"; readonly name: string; readonly rollback: RollbackReport | null }
  | { readonly phase: "orphaned"; readonly name: string };

interface AccountsState {
  readonly accounts: Loadable<readonly AccountView[]>;
  readonly removal: Removal | null;
  readonly load: () => Promise<void>;
  /**
   * Resolves to whether the account is gone. The list is re-read either way. The account in
   * use is removed by signing Codex out - the explicit choice asked for is the confirmation
   * the sheet collects before calling this.
   */
  readonly remove: (account: AccountView, nowSeconds: number) => Promise<boolean>;
  readonly dismissRemoval: () => void;
}

export const useAccounts = create<AccountsState>()((set, get) => ({
  accounts: { state: "loading" },
  removal: null,
  remove: async (account, nowSeconds) => {
    set({
      removal: { phase: "removing", accountId: account.id, signingOut: account.isActive },
    });
    const result = await removeAccount(account.id, account.isActive, nowSeconds);
    // Re-read whichever way it went: what Rust lists is the truth about what is left, and a
    // removal that failed halfway is only visible by asking.
    await get().load();
    const removed = result.ok && result.value.removed;
    set({
      removal: !result.ok
        ? { phase: "failed", name: account.displayName, rollback: null }
        : !result.value.removed
          ? { phase: "failed", name: account.displayName, rollback: result.value.rollback }
          : result.value.credentialDeleted
            ? null
            : { phase: "orphaned", name: account.displayName },
    });
    return removed;
  },
  dismissRemoval: () => {
    set({ removal: null });
  },
  load: async () => {
    set({ accounts: { state: "loading" } });
    const result = await listAccounts();
    set({
      accounts: result.ok
        ? { state: "ready", value: result.value }
        : { state: "failed", failure: result.failure },
    });
  },
}));
