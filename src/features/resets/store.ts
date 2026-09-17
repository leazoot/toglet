/**
 * Reset alerts, mirrored from Rust: always the settings and reading Rust answered with. The
 * feed is read on the Rust side; this store never sees an address.
 */

import { create } from "zustand";

import { openResetsSite, readResets, saveResets } from "../../ipc";
import type { IpcFailure, ResetsDraft, ResetsView } from "../../types/ipc";
import type { Loadable } from "../../types/load";

interface ResetsState {
  readonly resets: Loadable<ResetsView>;
  /** True while a save is on its way to Rust. */
  readonly busy: boolean;
  /** The last command that did not get through. Shown, then dismissed by the next action. */
  readonly failure: IpcFailure | null;
  readonly load: () => Promise<void>;
  /** A state Rust pushed after a reading; replaces what is held, never merged. */
  readonly replace: (view: ResetsView) => void;
  readonly save: (draft: ResetsDraft) => Promise<boolean>;
  readonly openSite: () => Promise<void>;
}

export const useResets = create<ResetsState>()((set) => ({
  resets: { state: "loading" },
  busy: false,
  failure: null,
  load: async () => {
    const result = await readResets();
    set({
      resets: result.ok
        ? { state: "ready", value: result.value }
        : { state: "failed", failure: result.failure },
    });
  },
  replace: (view) => {
    set({ resets: { state: "ready", value: view } });
  },
  save: async (draft) => {
    set({ busy: true, failure: null });
    const result = await saveResets(draft);
    if (!result.ok) {
      set({ busy: false, failure: result.failure });
      return false;
    }
    set({ resets: { state: "ready", value: result.value }, busy: false });
    return true;
  },
  openSite: async () => {
    const result = await openResetsSite();
    if (!result.ok) {
      set({ failure: result.failure });
    }
  },
}));
