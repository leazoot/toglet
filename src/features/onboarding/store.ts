/**
 * Adding an account: idle → confirming → waiting → added | duplicate | failed.
 * `duplicate` is not a failure: the browser reused an open session, and `account/login/start`
 * cannot request the account chooser.
 */

import { create } from "zustand";

import { cancelLogin, finishLogin, startLogin } from "../../ipc";
import type { AccountView, IpcFailure } from "../../types/ipc";

export type AddPhase = "idle" | "confirming" | "waiting" | "added" | "duplicate" | "failed";

interface AddState {
  readonly phase: AddPhase;
  /** The account the sign-in produced, whether it was new or one already held. */
  readonly account: AccountView | null;
  readonly failure: IpcFailure | null;

  readonly open: () => void;
  readonly begin: (nowSeconds: number) => Promise<void>;
  readonly cancel: () => void;
  readonly dismiss: () => void;
}

const IDLE = { phase: "idle", account: null, failure: null } as const;

export const useAdding = create<AddState>()((set, get) => ({
  ...IDLE,

  open: () => {
    set({ ...IDLE, phase: "confirming" });
  },

  begin: async (nowSeconds) => {
    if (get().phase !== "confirming") {
      return;
    }
    set({ phase: "waiting", failure: null, account: null });

    const started = await startLogin();
    if (!started.ok) {
      set({ phase: "failed", failure: started.failure });
      return;
    }

    const done = await finishLogin(nowSeconds);
    if (!done.ok) {
      set({ phase: "failed", failure: done.failure });
      return;
    }
    set({
      phase: done.value.added ? "added" : "duplicate",
      account: done.value.account,
    });
  },

  cancel: () => {
    // Cancelling in Rust removes the temporary home and its app server.
    if (get().phase === "waiting") {
      void cancelLogin();
    }
    set({ ...IDLE });
  },

  dismiss: () => {
    set({ ...IDLE });
  },
}));
