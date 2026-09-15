/**
 * Phone remote settings, mirrored from Rust: always what Rust answered with, never what was
 * requested. The shared secret is never held here; no command reads it back.
 */

import { create } from "zustand";

import { forgetRemote, readRemote, saveRemote } from "../../ipc";
import type { IpcFailure, RemoteDraft, RemoteView } from "../../types/ipc";
import type { Loadable } from "../../types/load";

interface RemoteState {
  readonly remote: Loadable<RemoteView>;
  /** True while a save or a removal is on its way to Rust. */
  readonly busy: boolean;
  /** The last command that did not get through. Shown, then dismissed by the next action. */
  readonly failure: IpcFailure | null;
  readonly load: () => Promise<void>;
  readonly save: (draft: RemoteDraft) => Promise<boolean>;
  readonly forget: () => Promise<void>;
}

export const useRemote = create<RemoteState>()((set) => ({
  remote: { state: "loading" },
  busy: false,
  failure: null,
  load: async () => {
    const result = await readRemote();
    set({
      remote: result.ok
        ? { state: "ready", value: result.value }
        : { state: "failed", failure: result.failure },
    });
  },
  save: async (draft) => {
    set({ busy: true, failure: null });
    const result = await saveRemote(draft);
    if (!result.ok) {
      set({ busy: false, failure: result.failure });
      return false;
    }
    set({ remote: { state: "ready", value: result.value }, busy: false });
    return true;
  },
  forget: async () => {
    set({ busy: true, failure: null });
    const result = await forgetRemote();
    set(
      result.ok
        ? { remote: { state: "ready", value: result.value }, busy: false }
        : { busy: false, failure: result.failure },
    );
  },
}));
