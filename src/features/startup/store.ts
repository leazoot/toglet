/**
 * What was true when Toglet opened: whether this machine can run Codex at all, and whether a
 * switch had been left half-finished.
 *
 * Both are read once at start-up and describe the machine rather than any one account, which
 * is why they live together and not in `features/accounts`.
 */

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
    // Independent reads: neither should wait on the other, and one failing must not hide the
    // other's answer.
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
