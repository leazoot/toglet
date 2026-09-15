import { describe, expect, it } from "vitest";

import { t } from "../../i18n";
import type { AutoRunState, AutoRunView } from "../../types/ipc";
import { controlsFor, isExecuting, statusLine } from "./status";

const NOW = 1_800_000_000;

function plan(overrides: Partial<AutoRunView> = {}): AutoRunView {
  return {
    enabled: true,
    binding: null,
    participants: [],
    state: "armed",
    generation: 1,
    executingAccountId: null,
    waitReason: null,
    expectedAvailableAt: null,
    nextCheckAt: null,
    lastResult: null,
    resumeCount: 0,
    maxResumes: 8,
    deadline: null,
    updatedAt: "2026-09-11T00:00:00Z",
    ...overrides,
  };
}

const NAMES: Record<string, string> = { "acct-1": "Team", "acct-2": "Personal" };

function line(view: AutoRunView): string | null {
  const found = statusLine(
    view,
    (id) => NAMES[id] ?? null,
    (at) => `clock(${(at - NOW).toString()})`,
    t,
  );
  return found === null ? null : t(found.key, found.params);
}

function tone(view: AutoRunView): string | null {
  const found = statusLine(
    view,
    (id) => NAMES[id] ?? null,
    (at) => `clock(${(at - NOW).toString()})`,
    t,
  );
  return found === null ? null : found.tone;
}

describe("the status line", () => {
  it("has a sentence for each of the ten states and the two between", () => {
    const expected: Record<Exclude<AutoRunState, "disabled">, string> = {
      armed: "Watching the session for the quota to run out",
      selecting: "Choosing an account",
      waiting_quota: "Waiting for quota · expected time unknown",
      verifying: "Verifying quota",
      switching: "Switching account",
      resuming: "Resuming the task",
      running: "Running",
      waiting_network: "Waiting for the network",
      round_completed: "One round completed",
      needs_human: "Needs attention: see the log",
      paused: "Paused",
      stopped: "Stopped",
    };
    for (const [state, sentence] of Object.entries(expected) as [AutoRunState, string][]) {
      expect(line(plan({ state })), state).toBe(sentence);
    }
  });

  it("has no line for a plan that is off", () => {
    expect(line(plan({ enabled: false, state: "disabled" }))).toBeNull();
  });

  it("says the expected time is unknown rather than counting down from nothing", () => {
    // `null` means Rust does not know; no countdown may be guessed.
    expect(line(plan({ state: "waiting_quota", expectedAvailableAt: null }))).toBe(
      "Waiting for quota · expected time unknown",
    );
  });

  it("shows the expected time Rust wrote, and the account being waited for", () => {
    expect(
      line(
        plan({
          state: "waiting_quota",
          expectedAvailableAt: NOW + 7_200,
          lastResult: { kind: "waited", accountId: "acct-2", turnId: null, code: null, at: "" },
        }),
      ),
    ).toBe("Waiting for quota · expected clock(7200) · Personal");
  });

  it("names the account the scheduler is running as", () => {
    expect(line(plan({ state: "running", executingAccountId: "acct-1" }))).toBe("Running · Team");
    // An id the list does not hold is not invented into a name.
    expect(line(plan({ state: "running", executingAccountId: "acct-9" }))).toBe("Running");
  });

  it("spells out the reasons it has words for, and shows the rest as their code", () => {
    expect(line(plan({ state: "needs_human", waitReason: "waiting_on_human" }))).toBe(
      "Needs attention: the session is waiting for your answer",
    );
    expect(line(plan({ state: "stopped", waitReason: "max_resumes" }))).toBe(
      "Stopped · continuation limit reached",
    );
    expect(line(plan({ state: "paused", waitReason: "desktop_reopened" }))).toBe(
      "Paused · the Codex app was reopened",
    );
    expect(line(plan({ state: "needs_human", waitReason: "some_new_code" }))).toBe(
      "Needs attention: some_new_code",
    );
  });

  // A transiently refused switch or resume is retried; the line must not read as progress.
  it("says a switch or a continuation is waiting on something rather than progressing", () => {
    expect(line(plan({ state: "switching", waitReason: "client_running" }))).toBe(
      "Switch waiting · a Codex session is running · trying again",
    );
    expect(line(plan({ state: "resuming", waitReason: "client_running" }))).toBe(
      "Resume waiting · a Codex session is running · trying again",
    );
    expect(tone(plan({ state: "resuming", waitReason: "client_running" }))).toBe("warn");
    // Without a reason they are the plain steps they were.
    expect(tone(plan({ state: "resuming" }))).toBe("mute");
  });

  it("offers pause while something is in flight and continue while waiting on the user", () => {
    for (const state of [
      "armed",
      "selecting",
      "waiting_quota",
      "verifying",
      "switching",
      "resuming",
      "running",
      "waiting_network",
    ] as const) {
      expect(controlsFor(state), state).toEqual(["pause", "cancel"]);
    }
    for (const state of ["paused", "round_completed", "stopped", "needs_human"] as const) {
      expect(controlsFor(state), state).toEqual(["resume", "cancel"]);
    }
    expect(controlsFor("disabled")).toEqual([]);
  });
});

describe("the continuing tag", () => {
  it("marks the executing account only while the scheduler is acting as it", () => {
    for (const state of [
      "verifying",
      "switching",
      "resuming",
      "running",
      "waiting_network",
    ] as const) {
      expect(isExecuting(plan({ state, executingAccountId: "acct-1" }), "acct-1")).toBe(true);
    }
  });

  it("does not mark the last attempt's account while the scheduler waits for another", () => {
    // The row said "continuing" while the line beneath waited for the other account.
    for (const state of [
      "waiting_quota",
      "needs_human",
      "paused",
      "round_completed",
      "armed",
      "stopped",
    ] as const) {
      expect(isExecuting(plan({ state, executingAccountId: "acct-1" }), "acct-1")).toBe(false);
    }
    expect(isExecuting(plan({ state: "running", executingAccountId: "acct-1" }), "acct-2")).toBe(
      false,
    );
    expect(isExecuting(null, "acct-1")).toBe(false);
  });
});
