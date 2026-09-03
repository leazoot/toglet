/**
 * Adding an account: step 1 / step 2.
 *
 * ```text
 * idle → confirming → waiting → added | duplicate | failed
 * ```
 *
 * No naming step. The design's step 1 asked for a name before the browser opened; the account
 * is named after itself, and the account's own name turns out to be in the credential the
 * sign-in produces. The step that remains is the warning about the browser, which has to be read
 * before the browser opens.
 *
 * `duplicate` is a state of its own rather than a failure. The sign-in worked; the browser
 * reused a ChatGPT session that was already open, and `account/login/start` has no way to ask
 * for the account chooser. Calling that a failure would blame the user for something the
 * protocol does not let anyone ask for.
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
    // Tearing the sign-in down is what removes the throwaway home and the app server behind it,
    // so it is asked for even when the interface is already moving on.
    if (get().phase === "waiting") {
      void cancelLogin();
    }
    set({ ...IDLE });
  },

  dismiss: () => {
    set({ ...IDLE });
  },
}));
