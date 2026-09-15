import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() =>
  vi.fn<(command: string, args?: Record<string, unknown>) => Promise<unknown>>(),
);
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { t } from "../../i18n";
import type { AccountView, AutoRunView, ThreadListView } from "../../types/ipc";
import { AutoRunSection } from "../settings/AutoRunSection";
import { AutoRunSheet } from "../settings/AutoRunSheet";
import { AutoRunConfirm } from "./AutoRunConfirm";
import {
  DEFAULT_MAX_RESUMES,
  INSTRUCTION_MAX_CHARS,
  choiceAvailable,
  deadlineAt,
  draftFrom,
  problemOf,
  sameAsPlan,
  toRequest,
} from "./draft";
import type { BindDraft } from "./draft";
import { useAutoRun } from "./store";

const TEAM: AccountView = {
  id: "acct-1",
  displayName: "Team",
  maskedEmail: "lea***@gmail.com",
  planType: "plus",
  status: "active",
  isActive: true,
};
const PERSONAL: AccountView = {
  id: "acct-2",
  displayName: "Personal",
  maskedEmail: "per***@gmail.com",
  planType: "plus",
  status: "ready",
  isActive: false,
};
const ACCOUNTS = { state: "ready", value: [TEAM, PERSONAL] } as const;

/** A Wednesday, 14:00 local time. */
const NOW = Math.floor(new Date(2026, 8, 9, 14, 0, 0).getTime() / 1000);

function disabledPlan(overrides: Partial<AutoRunView> = {}): AutoRunView {
  return {
    enabled: false,
    binding: null,
    participants: [],
    state: "disabled",
    generation: 0,
    executingAccountId: null,
    waitReason: null,
    expectedAvailableAt: null,
    nextCheckAt: null,
    lastResult: null,
    resumeCount: 0,
    maxResumes: null,
    deadline: null,
    updatedAt: "2026-09-09T06:00:00Z",
    ...overrides,
  };
}

function boundPlan(overrides: Partial<AutoRunView> = {}): AutoRunView {
  return disabledPlan({
    binding: {
      executionEnvironment: "desktop",
      projectLabel: "toglet-demo",
      threadId: "thread-1",
      threadTitle: "Fix the parser",
      resumeInstruction: "Carry on.",
      boundAt: "2026-09-09T06:00:00Z",
    },
    // Stored out of order on purpose: the order field is the priority, not the position.
    participants: [
      { accountId: "acct-2", order: 1 },
      { accountId: "acct-1", order: 0 },
    ],
    maxResumes: 4,
    ...overrides,
  });
}

function draft(overrides: Partial<BindDraft> = {}): BindDraft {
  return {
    thread: {
      threadId: "thread-1",
      title: "Fix the parser",
      preview: null,
      projectLabel: "toglet-demo",
    },
    participants: ["acct-1", "acct-2"],
    instruction: "Carry on.",
    deadline: "none",
    keptDeadline: null,
    maxResumes: 8,
    ...overrides,
  };
}

const LISTING: ThreadListView = {
  threads: [
    {
      threadId: "thread-old",
      title: "Old work",
      preview: null,
      projectLabel: "toglet-demo",
      updatedAt: NOW - 3 * 86_400,
    },
    {
      threadId: "thread-1",
      title: "Fix the parser",
      preview: null,
      projectLabel: "toglet-demo",
      updatedAt: NOW - 600,
    },
    // Codex named none of these; the first message is what tells them apart.
    {
      threadId: "thread-2",
      title: null,
      preview: "write the release notes",
      projectLabel: "notes",
      updatedAt: NOW - 7_200,
    },
    {
      threadId: "thread-3",
      title: null,
      preview: null,
      projectLabel: "notes",
      updatedAt: NOW - 9_000,
    },
  ],
  truncated: false,
  runtimeVersion: "0.153.4",
};

describe("the draft", () => {
  it("starts from the default instruction, the default cap and no deadline", () => {
    const fresh = draftFrom(null, "Default words.", NOW);

    expect(fresh.thread).toBeNull();
    expect(fresh.participants).toEqual([]);
    expect(fresh.instruction).toBe("Default words.");
    expect(fresh.maxResumes).toBe(DEFAULT_MAX_RESUMES);
    expect(fresh.deadline).toBe("none");
  });

  it("takes a bound plan's accounts in priority order, not storage order", () => {
    const derived = draftFrom(boundPlan(), "Default words.", NOW);

    expect(derived.participants).toEqual(["acct-1", "acct-2"]);
    expect(derived.instruction).toBe("Carry on.");
    expect(derived.maxResumes).toBe(4);
    expect(derived.thread?.projectLabel).toBe("toglet-demo");
  });

  it("keeps a stored deadline that lies ahead and drops one that has passed", () => {
    // Rust refuses a deadline in the past.
    const ahead = draftFrom(boundPlan({ deadline: NOW + 3_600 }), "", NOW);
    expect(ahead.deadline).toBe("kept");
    expect(ahead.keptDeadline).toBe(NOW + 3_600);

    const behind = draftFrom(boundPlan({ deadline: NOW - 60 }), "", NOW);
    expect(behind.deadline).toBe("none");
    expect(behind.keptDeadline).toBeNull();
  });

  it("names what still stops a confirmation, in the order the sheet asks for it", () => {
    expect(problemOf(draft({ thread: null }))).toBe("no_thread");
    expect(problemOf(draft({ participants: [] }))).toBe("no_participants");
    expect(problemOf(draft({ instruction: "   " }))).toBe("no_instruction");
    expect(problemOf(draft({ instruction: "字".repeat(INSTRUCTION_MAX_CHARS + 1) }))).toBe(
      "instruction_long",
    );
    expect(problemOf(draft({ instruction: "字".repeat(INSTRUCTION_MAX_CHARS) }))).toBeNull();
    expect(problemOf(draft())).toBeNull();
  });

  it("turns a preset into the local moment it names", () => {
    const today = new Date(2026, 8, 9, 23, 0, 0).getTime() / 1000;
    const tomorrow = new Date(2026, 8, 10, 9, 0, 0).getTime() / 1000;

    expect(deadlineAt(draft({ deadline: "today_23" }), NOW)).toBe(today);
    expect(deadlineAt(draft({ deadline: "tomorrow_09" }), NOW)).toBe(tomorrow);
    expect(deadlineAt(draft({ deadline: "in_24h" }), NOW)).toBe(NOW + 86_400);
    expect(deadlineAt(draft({ deadline: "none" }), NOW)).toBeNull();
    expect(deadlineAt(draft({ deadline: "kept", keptDeadline: NOW + 5 }), NOW)).toBe(NOW + 5);
  });

  it("withdraws today's evening once it has passed", () => {
    const lateEvening = Math.floor(new Date(2026, 8, 9, 23, 30, 0).getTime() / 1000);

    expect(choiceAvailable("today_23", NOW)).toBe(true);
    expect(choiceAvailable("today_23", lateEvening)).toBe(false);
    expect(choiceAvailable("tomorrow_09", lateEvening)).toBe(true);
    expect(deadlineAt(draft({ deadline: "today_23" }), lateEvening)).toBeNull();
  });

  it("sends the request the way Rust declares it, with the instruction trimmed", () => {
    expect(
      toRequest(draft({ instruction: "  Carry on.  ", maxResumes: null }), "thread-1", NOW),
    ).toStrictEqual({
      threadId: "thread-1",
      resumeInstruction: "Carry on.",
      participants: ["acct-1", "acct-2"],
      maxResumes: null,
      deadline: null,
    });
  });

  it("knows when confirming would change nothing about the stored plan", () => {
    const plan = boundPlan({ maxResumes: 8 });
    expect(sameAsPlan(draft(), plan)).toBe(true);
    expect(sameAsPlan(draft({ instruction: "Carry on, differently." }), plan)).toBe(false);
    expect(sameAsPlan(draft({ participants: ["acct-2", "acct-1"] }), plan)).toBe(false);
    expect(sameAsPlan(draft({ deadline: "in_24h" }), plan)).toBe(false);
    expect(sameAsPlan(draft(), disabledPlan())).toBe(false);
  });
});

describe("the automatic-continuation store", () => {
  beforeEach(() => {
    invoke.mockReset();
    useAutoRun.setState({
      plan: { state: "loading" },
      threads: null,
      draft: null,
      proposal: null,
      committing: false,
      controlling: false,
      failure: null,
    });
  });

  function answering(bind: unknown = null, enable: unknown = null): void {
    invoke.mockImplementation((command: string) => {
      switch (command) {
        case "bind_autorun":
          return bind instanceof Error ? Promise.reject(bind) : Promise.resolve(bind);
        case "set_autorun_enabled":
          return enable instanceof Error ? Promise.reject(enable) : Promise.resolve(enable);
        case "read_autorun":
          return Promise.resolve(boundPlan({ enabled: true, state: "armed" }));
        default:
          return Promise.reject(new Error(`unexpected ${command}`));
      }
    });
  }

  it("binds and then turns the plan on, in that order, naming the arguments as Rust does", async () => {
    answering();
    useAutoRun.getState().replace(disabledPlan());
    useAutoRun.getState().propose(draft({ deadline: "in_24h" }));

    const on = await useAutoRun.getState().commit(NOW);

    expect(on).toBe(true);
    const commands = invoke.mock.calls.map(([name]) => name);
    expect(commands.indexOf("bind_autorun")).toBeLessThan(commands.indexOf("set_autorun_enabled"));
    expect(invoke).toHaveBeenCalledWith("bind_autorun", {
      request: {
        threadId: "thread-1",
        resumeInstruction: "Carry on.",
        participants: ["acct-1", "acct-2"],
        maxResumes: 8,
        deadline: NOW + 86_400,
      },
    });
    expect(invoke).toHaveBeenCalledWith("set_autorun_enabled", { enabled: true });
    // The plan held is Rust's answer, and the draft is dropped.
    const held = useAutoRun.getState();
    expect(held.proposal).toBeNull();
    expect(held.draft).toBeNull();
    expect(held.plan.state === "ready" && held.plan.value.enabled).toBe(true);
  });

  it("does not re-bind a plan that is being turned back on unchanged", async () => {
    // Binding needs the session in the last listing, which a plan bound in an earlier run lacks.
    answering();
    useAutoRun.getState().replace(boundPlan({ maxResumes: 8 }));
    useAutoRun.getState().propose(draft());

    await useAutoRun.getState().commit(NOW);

    expect(invoke.mock.calls.map(([name]) => name)).not.toContain("bind_autorun");
    expect(invoke).toHaveBeenCalledWith("set_autorun_enabled", { enabled: true });
  });

  it("stops at a refused binding and never asks for the plan to be turned on", async () => {
    // Rust's structured error, as the bridge delivers it: an object carrying the code.
    const refused = Object.assign(new Error("refused"), {
      code: "thread_unavailable",
      retryable: false,
    });
    invoke.mockImplementation((command: string) =>
      command === "bind_autorun" ? Promise.reject(refused) : Promise.resolve(null),
    );
    useAutoRun.getState().replace(disabledPlan());
    useAutoRun.getState().propose(draft());

    const on = await useAutoRun.getState().commit(NOW);

    expect(on).toBe(false);
    expect(invoke.mock.calls.map(([name]) => name)).not.toContain("set_autorun_enabled");
    const held = useAutoRun.getState();
    expect(held.failure).toStrictEqual({
      step: "bind",
      failure: { command: "bind_autorun", error: { code: "thread_unavailable", retryable: false } },
    });
    // The proposal stays, so the dialog can say what to do.
    expect(held.proposal).not.toBeNull();
    expect(held.committing).toBe(false);
  });

  it("mirrors the plan an event carries without deriving anything", () => {
    const plan = boundPlan({
      enabled: true,
      state: "waiting_quota",
      waitReason: "usage_limit_exceeded",
    });

    useAutoRun.getState().replace(plan);

    const held = useAutoRun.getState().plan;
    expect(held.state === "ready" ? held.value : null).toStrictEqual(plan);
  });

  it("pauses, continues and cancels with the matching command, then re-reads the plan", async () => {
    for (const [action, command] of [
      ["pause", "pause_autorun"],
      ["resume", "resume_autorun"],
      ["cancel", "cancel_autorun"],
    ] as const) {
      invoke.mockReset();
      invoke.mockImplementation((name: string) =>
        Promise.resolve(name === "read_autorun" ? disabledPlan() : null),
      );

      await useAutoRun.getState().control(action);

      expect(invoke).toHaveBeenCalledWith(command, undefined);
      expect(invoke.mock.calls.map(([name]) => name)).toContain("read_autorun");
      expect(useAutoRun.getState().controlling).toBe(false);
    }
  });

  it("records which control did not get through", async () => {
    invoke.mockImplementation((name: string) =>
      name === "pause_autorun"
        ? Promise.reject(
            Object.assign(new Error("refused"), { code: "internal", retryable: false }),
          )
        : Promise.resolve(null),
    );

    await useAutoRun.getState().control("pause");

    expect(useAutoRun.getState().failure).toStrictEqual({
      step: "pause",
      failure: { command: "pause_autorun", error: { code: "internal", retryable: false } },
    });
  });

  it("turns the plan off with the one command that does it", async () => {
    answering();
    await useAutoRun.getState().disable();

    expect(invoke).toHaveBeenCalledWith("set_autorun_enabled", { enabled: false });
    expect(useAutoRun.getState().failure).toBeNull();
  });
});

describe("the settings group", () => {
  afterEach(cleanup);

  function group(plan: AutoRunView, overrides: Partial<Parameters<typeof AutoRunSection>[0]> = {}) {
    const onEdit = vi.fn();
    const onEnable = vi.fn();
    const onDisable = vi.fn();
    const onChooseSession = vi.fn();
    const onChosen = vi.fn();
    render(
      <AutoRunSection
        plan={{ state: "ready", value: plan }}
        accounts={ACCOUNTS}
        threads={null}
        draft={null}
        busy={false}
        failure={null}
        nowSeconds={NOW}
        onEdit={onEdit}
        choosing={false}
        onChooseSession={onChooseSession}
        onChosen={onChosen}
        onEnable={onEnable}
        onDisable={onDisable}
        {...overrides}
      />,
    );
    return { onEdit, onEnable, onDisable, onChooseSession, onChosen };
  }

  function unfold(): void {
    fireEvent.click(screen.getByRole("button", { name: /More options/ }));
  }

  it("keeps the switch off, and says why, until a session is chosen", () => {
    group(disabledPlan());

    const toggle = screen.getByRole("switch", { name: "Enabled" });
    expect(toggle.hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("Choose a session first")).toBeDefined();
    expect(screen.getByText("Not chosen")).toBeDefined();
  });

  it("asks for the confirmation with the draft when the switch is turned on", () => {
    const { onEnable } = group(disabledPlan(), { draft: draft() });

    const toggle = screen.getByRole("switch", { name: "Enabled" });
    expect(toggle.hasAttribute("disabled")).toBe(false);
    fireEvent.click(toggle);

    expect(onEnable).toHaveBeenCalledWith(draft());
  });

  it("turns a running plan off directly and locks the fields while it is on", () => {
    const { onDisable } = group(boundPlan({ enabled: true, state: "armed" }));

    expect(screen.getByText("Turn it off to change these.")).toBeDefined();
    unfold();
    expect(screen.getByRole("textbox").hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("checkbox", { name: "Include Team" }).hasAttribute("disabled")).toBe(
      true,
    );

    fireEvent.click(screen.getByRole("switch", { name: "Enabled" }));
    expect(onDisable).toHaveBeenCalled();
  });

  it("asks the sheet for the chooser page from the session button", () => {
    const { onChooseSession } = group(disabledPlan());

    fireEvent.click(screen.getByRole("button", { name: "Choose a session" }));

    expect(onChooseSession).toHaveBeenCalledTimes(1);
  });

  it("lists the sessions by folder, newest first, named by title or first message", () => {
    const { onEdit, onChosen } = group(disabledPlan(), {
      threads: { state: "ready", value: LISTING },
      choosing: true,
    });

    const options = screen.getAllByRole("option").map((option) => option.textContent);
    expect(options).toEqual([
      "Fix the parser10m ago",
      `Old work${weekdayAt(NOW - 3 * 86_400)}`,
      "write the release notes2h 0m ago",
      "Untitled session2h 30m ago",
    ]);
    // Which Codex version made the list, since missing sessions cannot say so themselves.
    expect(screen.getByText("Codex 0.153.4")).toBeDefined();

    fireEvent.click(screen.getByRole("option", { name: /release notes/ }));

    expect(onEdit).toHaveBeenCalledWith(
      expect.objectContaining({
        thread: {
          threadId: "thread-2",
          title: null,
          preview: "write the release notes",
          projectLabel: "notes",
        },
      }),
    );
    expect(onChosen).toHaveBeenCalledTimes(1);
  });

  it("names the chosen session by its first message, from the draft or the last listing", () => {
    group(disabledPlan(), {
      draft: draft({
        thread: { threadId: "thread-2", title: null, preview: null, projectLabel: "notes" },
      }),
      threads: { state: "ready", value: LISTING },
    });

    expect(screen.getByRole("button", { name: "Choose a session" }).textContent).toBe(
      "notes · write the release notes",
    );
  });

  it("says which Codex saw nothing when the list is empty", () => {
    group(disabledPlan(), {
      threads: {
        state: "ready",
        value: { threads: [], truncated: false, runtimeVersion: "0.111.0" },
      },
      choosing: true,
    });

    expect(
      screen.getByText(/This version of Codex \(0\.111\.0\) can see no sessions/),
    ).toBeDefined();
  });

  it("folds the instruction and the limits away under a line that sums them up", () => {
    group(disabledPlan(), { draft: draft({ instruction: t("autorun.defaultInstruction") }) });

    expect(screen.queryByRole("textbox")).toBeNull();
    const fold = screen.getByRole("button", { name: /More options/ });
    expect(fold.getAttribute("aria-expanded")).toBe("false");
    expect(fold.textContent).toContain("Default instruction · No deadline · Up to 8");

    unfold();

    expect(fold.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("textbox")).toBeDefined();
    expect(screen.getByRole("radio", { name: "In 24h" })).toBeDefined();
  });

  it("sums up a custom instruction, a deadline and no cap", () => {
    group(disabledPlan(), {
      draft: draft({ instruction: "Carry on.", deadline: "in_24h", maxResumes: null }),
    });

    expect(screen.getByRole("button", { name: /More options/ }).textContent).toContain(
      `Custom instruction · Stop by ${weekdayAt(NOW + 86_400)} · No limit`,
    );
  });

  it("unfolds on its own when the instruction is what stops the switch", () => {
    group(disabledPlan(), { draft: draft({ instruction: "   " }) });

    expect(screen.getByText("Write the instruction first")).toBeDefined();
    expect(screen.getByRole("textbox")).toBeDefined();
  });

  it("orders the ticked accounts and moves them up and down", () => {
    const { onEdit } = group(disabledPlan(), { draft: draft({ participants: ["acct-1"] }) });

    // Ticking adds to the end of the order.
    fireEvent.click(screen.getByRole("checkbox", { name: "Include Personal" }));
    expect(onEdit).toHaveBeenLastCalledWith(
      expect.objectContaining({ participants: ["acct-1", "acct-2"] }),
    );
    // The only participant cannot move.
    expect(screen.getByRole("button", { name: "Move Team up" }).hasAttribute("disabled")).toBe(
      true,
    );
    expect(screen.getByRole("button", { name: "Move Team down" }).hasAttribute("disabled")).toBe(
      true,
    );
    // An account that is not ticked cannot be ordered either.
    expect(screen.getByRole("button", { name: "Move Personal up" }).hasAttribute("disabled")).toBe(
      true,
    );
  });

  it("moves a second account ahead of the first", () => {
    const { onEdit } = group(disabledPlan(), { draft: draft() });

    fireEvent.click(screen.getByRole("button", { name: "Move Personal up" }));

    expect(onEdit).toHaveBeenLastCalledWith(
      expect.objectContaining({ participants: ["acct-2", "acct-1"] }),
    );
    // Unticking removes and keeps the rest in order.
    fireEvent.click(screen.getByRole("checkbox", { name: "Include Team" }));
    expect(onEdit).toHaveBeenLastCalledWith(expect.objectContaining({ participants: ["acct-2"] }));
  });

  it("counts the instruction the way Rust does and marks one that is too long", () => {
    group(disabledPlan(), {
      draft: draft({ instruction: "字".repeat(INSTRUCTION_MAX_CHARS + 1) }),
    });

    expect(
      screen.getByText(`${String(INSTRUCTION_MAX_CHARS + 1)} / ${String(INSTRUCTION_MAX_CHARS)}`),
    ).toBeDefined();
    expect(screen.getByText("Shorten the instruction")).toBeDefined();
    expect(screen.getByRole("switch", { name: "Enabled" }).hasAttribute("disabled")).toBe(true);
  });

  it("offers the four deadline presets and the default cap, and shows a kept deadline as itself", () => {
    const { onEdit } = group(disabledPlan(), {
      draft: draft({ deadline: "kept", keptDeadline: NOW + 3_600 }),
    });
    unfold();

    for (const name of [
      "None",
      "Today 23:00",
      "Tomorrow 09:00",
      "In 24h",
      "No limit",
      "4",
      "8",
      "16",
    ]) {
      expect(screen.getByRole("radio", { name })).toBeDefined();
    }
    expect(screen.getByRole("radio", { name: "8" }).getAttribute("aria-checked")).toBe("true");
    expect(
      screen.getByRole("radio", { name: weekdayAt(NOW + 3_600) }).getAttribute("aria-checked"),
    ).toBe("true");

    fireEvent.click(screen.getByRole("radio", { name: "No limit" }));
    expect(onEdit).toHaveBeenLastCalledWith(expect.objectContaining({ maxResumes: null }));
    fireEvent.click(screen.getByRole("radio", { name: "In 24h" }));
    expect(onEdit).toHaveBeenLastCalledWith(expect.objectContaining({ deadline: "in_24h" }));
  });

  it("leaves the platform caveat to the confirmation", () => {
    group(disabledPlan());

    expect(screen.queryByText(t("autorun.confirmCaveat"))).toBeNull();
  });

  it("flips to the chooser page and back, reading the list on the way in", () => {
    const onListThreads = vi.fn();
    const onEdit = vi.fn();
    render(
      <AutoRunSheet
        plan={{ state: "ready", value: disabledPlan() }}
        accounts={ACCOUNTS}
        threads={{ state: "ready", value: LISTING }}
        draft={null}
        busy={false}
        failure={null}
        nowSeconds={NOW}
        onEdit={onEdit}
        onListThreads={onListThreads}
        onEnable={vi.fn()}
        onDisable={vi.fn()}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByRole("dialog", { name: "Automatic continuation" })).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Choose a session" }));

    expect(onListThreads).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("dialog", { name: "Choose a session" })).toBeDefined();
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.getAllByRole("option")).toHaveLength(4);

    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(screen.getByRole("switch", { name: "Enabled" })).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Choose a session" }));
    fireEvent.click(screen.getByRole("option", { name: /Old work/ }));
    expect(onEdit).toHaveBeenCalledWith(
      expect.objectContaining({
        thread: {
          threadId: "thread-old",
          title: "Old work",
          preview: null,
          projectLabel: "toglet-demo",
        },
      }),
    );
    expect(screen.getByRole("dialog", { name: "Automatic continuation" })).toBeDefined();
  });

  it("says when the plan could not be turned off, and that it is still on", () => {
    group(boundPlan({ enabled: true, state: "armed" }), {
      failure: {
        step: "disable",
        failure: { command: "set_autorun_enabled", error: { code: "internal", retryable: false } },
      },
    });

    expect(screen.getByRole("alert").textContent).toMatch(
      /could not be turned off \(internal\)\. It is still on/,
    );
  });
});

describe("the confirmation", () => {
  afterEach(cleanup);

  function confirm(overrides: Partial<Parameters<typeof AutoRunConfirm>[0]> = {}) {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <AutoRunConfirm
        draft={draft({ deadline: "tomorrow_09", instruction: "x".repeat(100) })}
        accounts={ACCOUNTS}
        nowSeconds={NOW}
        committing={false}
        failure={null}
        onConfirm={onConfirm}
        onCancel={onCancel}
        {...overrides}
      />,
    );
    return { onConfirm, onCancel };
  }

  it("names the session, says what will happen, and sums the options up in small print", () => {
    const { onConfirm } = confirm();

    const dialog = screen.getByRole("dialog", { name: "Start waiting?" });
    expect(dialog.textContent).toContain("toglet-demo · Fix the parser");
    expect(dialog.textContent).toContain("switch through Team → Personal in that order");
    // The instruction itself is not repeated here; the sheet has it.
    expect(dialog.textContent).not.toContain("xxxx");
    expect(dialog.textContent).toContain(
      `Custom instruction · Stop by ${weekdayAt(NOW + 19 * 3_600)} · Up to 8`,
    );
    expect(dialog.textContent).toContain(t("autorun.confirmCaveat"));

    fireEvent.click(screen.getByRole("button", { name: "Start waiting" }));
    expect(onConfirm).toHaveBeenCalled();
  });

  it("says a session that dropped out of the listing has to be chosen again", () => {
    confirm({
      failure: {
        step: "bind",
        failure: {
          command: "bind_autorun",
          error: { code: "thread_unavailable", retryable: false },
        },
      },
    });

    expect(screen.getByRole("alert").textContent).toMatch(/Choose the session again/);
    expect(screen.getByRole("alert").textContent).toMatch(/Nothing was changed/);
  });

  it("holds both buttons while the plan is being turned on", () => {
    confirm({ committing: true });

    expect(screen.getByRole("button", { name: "Turning on…" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "Back" }).hasAttribute("disabled")).toBe(true);
  });
});

/** `Wed 11:00` for a moment, in the words the formatter uses. */
function weekdayAt(seconds: number): string {
  const at = new Date(seconds * 1000);
  const day = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][at.getDay()] ?? "";
  return `${day} ${at.getHours().toString().padStart(2, "0")}:${at.getMinutes().toString().padStart(2, "0")}`;
}
