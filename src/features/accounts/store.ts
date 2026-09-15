/** The account list mirrored from Rust. `isActive` is never derived or updated optimistically. */

import { create } from "zustand";

import { listAccounts, removeAccount } from "../../ipc";
import type { AccountView, RollbackReport } from "../../types/ipc";
import type { Loadable } from "../../types/load";

/**
 * `orphaned`: gone from the list but the credential entry survived. `rollback` reports what
 * happened to Codex's sign-in when signing out the account in use; `null` if it was not touched.
 */
export type Removal =
  | { readonly phase: "removing"; readonly accountId: string; readonly signingOut: boolean }
  | { readonly phase: "failed"; readonly name: string; readonly rollback: RollbackReport | null }
  | { readonly phase: "orphaned"; readonly name: string };

interface AccountsState {
  readonly accounts: Loadable<readonly AccountView[]>;
  readonly removal: Removal | null;
  readonly load: () => Promise<void>;
  /** Resolves to whether the account is gone. Removing the account in use signs Codex out. */
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
    // Re-read either way: a removal that failed halfway is only visible in Rust's list.
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
