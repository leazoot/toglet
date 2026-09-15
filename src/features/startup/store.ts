/** Read once at start-up: whether Codex can run here, and whether a switch was left unfinished. */

import { create } from "zustand";

import { detectEnvironment, startupRecovery } from "../../ipc";
import type { EnvironmentReport, RecoveryOutcome } from "../../types/ipc";
import type { Loadable } from "../../types/load";

interface StartupState {
  readonly environment: Loadable<EnvironmentReport>;
  /** `null` inside `ready` means there was nothing to recover - the ordinary answer. */
  readonly recovery: Loadable<RecoveryOutcome | null>;
  readonly load: () => Promise<void>;
}

export const useStartup = create<StartupState>()((set) => ({
  environment: { state: "loading" },
  recovery: { state: "loading" },
  load: async () => {
    set({ environment: { state: "loading" }, recovery: { state: "loading" } });
    // Independent reads: one failing must not hide the other's answer.
    const [environment, recovery] = await Promise.all([detectEnvironment(), startupRecovery()]);
    set({
      environment: environment.ok
        ? { state: "ready", value: environment.value }
        : { state: "failed", failure: environment.failure },
      recovery: recovery.ok
        ? { state: "ready", value: recovery.value }
        : { state: "failed", failure: recovery.failure },
    });
  },
}));
