import { describe, expect, it } from "vitest";

import { t } from "../../i18n";
import type { AutoRunView } from "../../types/ipc";
import { notificationFor } from "./notifications";

function plan(overrides: Partial<AutoRunView> = {}): AutoRunView {
  return {
    enabled: true,
    binding: {
      executionEnvironment: "desktop",
      projectLabel: "toglet-demo",
      threadId: "thread-1",
      threadTitle: "Fix the parser in /Users/somebody/project",
      resumeInstruction: "Continue, and mail lea@example.com when done.",
      boundAt: "2026-09-11T00:00:00Z",
    },
    participants: [],
    state: "waiting_quota",
    generation: 3,
    executingAccountId: "acct-1",
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
const nameOf = (id: string): string | null => NAMES[id] ?? null;

function notification(previous: AutoRunView | null, next: AutoRunView): string | null {
  return notificationFor(previous, next, nameOf, t)?.body ?? null;
}

describe("automatic-continuation notifications", () => {
  it("announces the scheduler needing a person, once", () => {
    const needs = plan({ state: "needs_human", waitReason: "waiting_on_human" });

    expect(notification(plan(), needs)).toBe(
      "Automatic continuation needs attention: the session is waiting for your answer.",
    );
    // The same state written again - a re-read, a generation bump - is not news.
    expect(notification(needs, { ...needs, generation: 4 })).toBeNull();
  });

  it("announces a stop with its reason, or without one when Rust gave none", () => {
    expect(notification(plan(), plan({ state: "stopped", waitReason: "deadline" }))).toBe(
      "Automatic continuation stopped: deadline reached.",
    );
    expect(notification(plan(), plan({ state: "stopped", waitReason: null }))).toBe(
      "Automatic continuation stopped.",
    );
  });

  it("announces a finished continuation, once", () => {
    const done = plan({ state: "round_completed" });
    expect(notificationFor(plan({ state: "running" }), done, nameOf, t)).toEqual({
      title: "Toglet",
      body: t("autorun.notify.roundCompleted"),
    });
    expect(notificationFor(done, done, nameOf, t)).toBeNull();
  });

  it("announces a switch by the account's display name, once per switch", () => {
    const switched = plan({
      state: "resuming",
      lastResult: { kind: "switched", accountId: "acct-2", turnId: null, code: null, at: "t1" },
    });

    expect(notification(plan(), switched)).toBe("Switched to Personal to continue the task.");
    expect(notification(switched, { ...switched, state: "running" })).toBeNull();
    // A later switch is a new result.
    const again = plan({
      lastResult: { kind: "switched", accountId: "acct-1", turnId: null, code: null, at: "t2" },
    });
    expect(notification(switched, again)).toBe("Switched to Team to continue the task.");
  });

  it("does not invent a name for an account the list does not hold", () => {
    const switched = plan({
      lastResult: { kind: "switched", accountId: "acct-9", turnId: null, code: null, at: "t1" },
    });

    expect(notification(plan(), switched)).toBe("Switched accounts to continue the task.");
  });

  // Each restart re-reads the plan from disk; with nothing to compare, every state would look new.
  it("says nothing about the first plan of a run, however alarming it is", () => {
    for (const state of ["stopped", "needs_human", "round_completed"] as const) {
      expect(notification(null, plan({ state, waitReason: "deadline" })), state).toBeNull();
    }
    expect(
      notification(
        null,
        plan({
          lastResult: { kind: "switched", accountId: "acct-2", turnId: null, code: null, at: "t1" },
        }),
      ),
    ).toBeNull();
  });

  it("says nothing for the ordinary changes of state", () => {
    expect(notification(plan(), plan({ state: "verifying" }))).toBeNull();
    expect(notification(plan(), plan({ state: "running" }))).toBeNull();
    expect(notification(plan(), plan({ state: "resuming" }))).toBeNull();
    expect(notification(plan(), plan({ state: "paused", waitReason: "manual_switch" }))).toBeNull();
  });

  it("never carries an address, a path, a session title or the instruction", () => {
    // The plan holds all four; none may reach a notification shown on lock screens and phones.
    const bodies = [
      notification(plan(), plan({ state: "needs_human", waitReason: "thread_unavailable" })),
      notification(plan(), plan({ state: "stopped", waitReason: "max_resumes" })),
      notification(
        plan(),
        plan({
          lastResult: {
            kind: "switched",
            accountId: "acct-2",
            turnId: "turn",
            code: null,
            at: "t",
          },
        }),
      ),
    ];
    for (const body of bodies) {
      expect(body).not.toBeNull();
      expect(body).not.toMatch(/@/);
      expect(body).not.toMatch(/\//);
      expect(body).not.toContain("Fix the parser");
      expect(body).not.toContain("Continue, and mail");
    }
  });
});
