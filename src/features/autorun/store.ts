/**
 * Automatic continuation. The plan mirrors Rust (nothing derived here); the draft lives in the
 * store so closing the sheet keeps it; the proposal is the draft awaiting confirmation.
 */

import { create } from "zustand";

import {
  bindAutoRun,
  cancelAutoRun,
  listThreads,
  pauseAutoRun,
  readAutoRun,
  resumeAutoRun,
  setAutoRunEnabled,
} from "../../ipc";
import type { AutoRunView, IpcFailure, IpcResult, ThreadListView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { sameAsPlan, toRequest } from "./draft";
import type { BindDraft } from "./draft";

/** The three things the status line can ask of a plan that is on. */
export type AutoRunControl = "pause" | "resume" | "cancel";

/** A command that did not do what was asked, and which one it was. */
export interface AutoRunFailure {
  readonly step: "bind" | "enable" | "disable" | AutoRunControl;
  readonly failure: IpcFailure;
}

interface AutoRunStore {
  readonly plan: Loadable<AutoRunView>;
  /** `null` until the chooser is first opened; listing starts an app server. */
  readonly threads: Loadable<ThreadListView> | null;
  readonly draft: BindDraft | null;
  readonly proposal: BindDraft | null;
  /** True while the confirmation is being carried out. */
  readonly committing: boolean;
  /** True while a pause, continue or cancel is on its way. */
  readonly controlling: boolean;
  readonly failure: AutoRunFailure | null;

  readonly load: () => Promise<void>;
  readonly replace: (plan: AutoRunView) => void;
  readonly listThreads: () => Promise<void>;
  readonly edit: (draft: BindDraft) => void;
  readonly propose: (draft: BindDraft) => void;
  readonly dismiss: () => void;
  /** Binds if anything changed, then turns the plan on. Resolves to whether it is now on. */
  readonly commit: (nowSeconds: number) => Promise<boolean>;
  readonly disable: () => Promise<void>;
  /** Pauses, resumes or cancels; the resulting plan is re-read from Rust. */
  readonly control: (action: AutoRunControl) => Promise<void>;
  readonly dismissFailure: () => void;
}

const CONTROLS: Record<AutoRunControl, () => Promise<IpcResult<null>>> = {
  pause: pauseAutoRun,
  resume: resumeAutoRun,
  cancel: cancelAutoRun,
};

export const useAutoRun = create<AutoRunStore>()((set, get) => ({
  plan: { state: "loading" },
  threads: null,
  draft: null,
  proposal: null,
  committing: false,
  controlling: false,
  failure: null,

  load: async () => {
    const result = await readAutoRun();
    set({
      plan: result.ok
        ? { state: "ready", value: result.value }
        : { state: "failed", failure: result.failure },
    });
  },

  replace: (plan) => {
    set({ plan: { state: "ready", value: plan } });
  },

  listThreads: async () => {
    set({ threads: { state: "loading" } });
    const result = await listThreads();
    set({
      threads: result.ok
        ? { state: "ready", value: result.value }
        : { state: "failed", failure: result.failure },
    });
  },

  edit: (draft) => {
    set({ draft });
  },

  propose: (draft) => {
    set({ proposal: draft, failure: null });
  },

  dismiss: () => {
    // A confirmation being carried out is not dismissible: the plan is being written.
    if (get().committing) {
      return;
    }
    set({ proposal: null, failure: null });
  },

  commit: async (nowSeconds) => {
    const draft = get().proposal;
    const threadId = draft?.thread?.threadId;
    if (draft === null || threadId === undefined) {
      return false;
    }
    set({ committing: true, failure: null });

    const plan = get().plan;
    const unchanged = plan.state === "ready" && sameAsPlan(draft, plan.value);
    if (!unchanged) {
      const bound = await bindAutoRun(toRequest(draft, threadId, nowSeconds));
      if (!bound.ok) {
        set({ committing: false, failure: { step: "bind", failure: bound.failure } });
        return false;
      }
    }

    const enabled = await setAutoRunEnabled(true);
    if (!enabled.ok) {
      set({ committing: false, failure: { step: "enable", failure: enabled.failure } });
      return false;
    }
    // Re-read rather than rely on the state event arriving before this resolves.
    await get().load();
    // Drop the draft so the sheet derives it from the confirmed plan.
    set({ committing: false, proposal: null, draft: null });
    return true;
  },

  disable: async () => {
    set({ failure: null });
    const result = await setAutoRunEnabled(false);
    if (!result.ok) {
      set({ failure: { step: "disable", failure: result.failure } });
      return;
    }
    await get().load();
  },

  control: async (action) => {
    set({ controlling: true, failure: null });
    const result = await CONTROLS[action]();
    if (!result.ok) {
      set({ controlling: false, failure: { step: action, failure: result.failure } });
      return;
    }
    await get().load();
    set({ controlling: false });
  },

  dismissFailure: () => {
    set({ failure: null });
  },
}));
