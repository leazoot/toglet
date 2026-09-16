import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountView, AutoRunView, QuotaView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { Panel } from "./Panel";
import type { PanelProps } from "./Panel";

const NOW = 1_800_000_000;

function account(index: number): AccountView {
  return {
    id: `acct-${index.toString()}`,
    displayName: `Account ${index.toString()}`,
    maskedEmail: "lea***@gmail.com",
    planType: "plus",
    status: index === 1 ? "active" : "ready",
    isActive: index === 1,
  };
}

function accounts(count: number): readonly AccountView[] {
  return Array.from({ length: count }, (_, index) => account(index + 1));
}

function quotas(list: readonly AccountView[]): Record<string, Loadable<QuotaView>> {
  return Object.fromEntries(
    list.map((one) => [
      one.id,
      {
        state: "ready",
        value: {
          accountId: one.id,
          windows: [{ kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null }],
          fetchedAt: NOW,
          source: "codex_app_server",
          stale: false,
          lastErrorCode: null,
          resetCredits: null,
        },
      } satisfies Loadable<QuotaView>,
    ]),
  );
}

function panel(overrides: Partial<PanelProps> = {}) {
  const list = accounts(3);
  const props: PanelProps = {
    accounts: { state: "ready", value: list },
    quotas: quotas(list),
    refreshing: false,
    status: { tone: "ok", key: "status.justNow" },
    nowSeconds: NOW,
    onRefresh: () => undefined,
    onSelect: () => undefined,
    onResetCredits: () => undefined,
    onOpenSettings: () => undefined,
    onAddAccount: () => undefined,
    onOpenAutoRun: () => undefined,
    overlay: null,
    sheet: null,
    autorun: null,
    autorunBusy: false,
    autorunFailure: null,
    onAutoRunControl: () => undefined,
    ...overrides,
  };
  return render(<Panel {...props} />);
}

describe("the panel", () => {
  afterEach(cleanup);

  it.each([1, 3, 5, 10])("renders %i accounts", (count) => {
    const list = accounts(count);
    panel({ accounts: { state: "ready", value: list }, quotas: quotas(list) });

    expect(screen.getAllByTestId("account-row")).toHaveLength(count);
  });

  it("counts the accounts in the toolbar", () => {
    panel();

    expect(screen.getByText("3 accounts")).toBeDefined();
  });

  it("does not say 1 accounts", () => {
    const list = accounts(1);
    panel({ accounts: { state: "ready", value: list }, quotas: quotas(list) });

    expect(screen.getByText("1 account")).toBeDefined();
  });

  it("shows the first-run panel when there is genuinely no account", () => {
    panel({ accounts: { state: "ready", value: [] }, quotas: {} });

    expect(screen.getByText("No accounts yet")).toBeDefined();
    expect(screen.queryByTestId("account-row")).toBeNull();
  });

  it("says the list could not be read instead of showing it as empty", () => {
    panel({
      accounts: { state: "failed", failure: { command: "list_accounts", error: null } },
      quotas: {},
    });

    expect(screen.getByText(/could not read its own state/)).toBeDefined();
    expect(screen.queryByText("No accounts yet")).toBeNull();
  });

  it("says the list is on its way rather than calling it empty", () => {
    panel({ accounts: { state: "loading" }, quotas: {} });

    expect(screen.getByText("Loading accounts…")).toBeDefined();
    expect(screen.queryByText("No accounts yet")).toBeNull();
  });

  it("keeps the rows on screen while a refresh runs", () => {
    // Refreshing turns the icon and the scan line on; it is not a loading state.
    panel({ refreshing: true });

    expect(screen.getAllByTestId("account-row")).toHaveLength(3);
  });

  it("does not let a refresh be asked for twice at once", () => {
    const onRefresh = vi.fn();
    panel({ refreshing: true, onRefresh });

    const button = screen.getByLabelText("Refresh quota");
    expect(button.hasAttribute("disabled")).toBe(true);
  });

  it("refreshes when the button is used", () => {
    const onRefresh = vi.fn();
    panel({ onRefresh });

    screen.getByLabelText("Refresh quota").click();

    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("puts the status in words, not only in the colour of a dot", () => {
    panel({ status: { tone: "warn", key: "status.cached" } });

    expect(screen.getByText(/Showing cached values/)).toBeDefined();
  });

  it("moves the focus down the list with the arrow keys", () => {
    // The first row is the active account and is not selectable, so the focusable rows are the
    // other two.
    panel();
    const rows = screen.getAllByRole("button", { name: /Switch to/ });
    rows[0]?.focus();

    fireEvent.keyDown(document.activeElement ?? document.body, { key: "ArrowDown" });

    expect(document.activeElement).toBe(rows[1]);
  });

  it("wraps rather than running off the end of a short list", () => {
    panel();
    const rows = screen.getAllByRole("button", { name: /Switch to/ });
    rows[0]?.focus();

    fireEvent.keyDown(document.activeElement ?? document.body, { key: "ArrowUp" });

    expect(document.activeElement).toBe(rows[rows.length - 1]);
  });

  it("opens the settings sheet from the toolbar", () => {
    const onOpenSettings = vi.fn();
    panel({ onOpenSettings });

    screen.getByLabelText("Settings").click();

    expect(onOpenSettings).toHaveBeenCalledTimes(1);
  });

  it("keys rows by the account's own id so a reorder cannot recycle the wrong row", () => {
    const list = accounts(3);
    const { rerender } = panel({ accounts: { state: "ready", value: list }, quotas: quotas(list) });

    const reordered = [...list].reverse();
    rerender(
      <Panel
        accounts={{ state: "ready", value: reordered }}
        quotas={quotas(list)}
        refreshing={false}
        status={{ tone: "ok", key: "status.justNow" }}
        nowSeconds={NOW}
        onRefresh={() => undefined}
        onSelect={() => undefined}
        onResetCredits={() => undefined}
        onOpenSettings={() => undefined}
        onAddAccount={() => undefined}
        onOpenAutoRun={() => undefined}
        overlay={null}
        sheet={null}
        autorun={null}
        autorunBusy={false}
        autorunFailure={null}
        onAutoRunControl={() => undefined}
      />,
    );

    const names = screen.getAllByTestId("account-row").map((node) => node.textContent);
    expect(names[0]).toContain("Account 3");
    expect(names[2]).toContain("Account 1");
  });
});

function plan(overrides: Partial<AutoRunView> = {}): AutoRunView {
  return {
    enabled: true,
    binding: {
      executionEnvironment: "desktop",
      projectLabel: "toglet-demo",
      threadId: "thread-1",
      threadTitle: "Fix the parser",
      resumeInstruction: "Carry on.",
      boundAt: "2026-09-11T00:00:00Z",
    },
    participants: [
      { accountId: "acct-2", order: 0 },
      { accountId: "acct-3", order: 1 },
    ],
    state: "waiting_quota",
    generation: 4,
    executingAccountId: "acct-2",
    waitReason: "five_hour_exhausted",
    expectedAvailableAt: NOW + 3 * 3_600,
    nextCheckAt: NOW + 3 * 3_600 + 90,
    lastResult: { kind: "waited", accountId: "acct-3", turnId: null, code: null, at: "" },
    resumeCount: 1,
    maxResumes: 8,
    deadline: null,
    updatedAt: "2026-09-11T00:00:00Z",
    ...overrides,
  };
}

describe("the toolbar", () => {
  afterEach(cleanup);

  it("opens automatic continuation from its own button, between refresh and add", () => {
    const onOpenAutoRun = vi.fn();
    panel({ onOpenAutoRun });

    const names = screen
      .getAllByRole("button")
      .map((button) => button.getAttribute("aria-label"))
      .filter((name) => name !== null);
    expect(names.slice(0, 4)).toEqual([
      "Refresh quota",
      "Automatic continuation",
      "Add account",
      "Settings",
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Automatic continuation" }));
    expect(onOpenAutoRun).toHaveBeenCalled();
  });
});

describe("the automatic-continuation line", () => {
  afterEach(cleanup);

  it("is absent while the plan is off, and while nothing is known about it", () => {
    panel();
    expect(screen.queryByTestId("autorun-line")).toBeNull();
    cleanup();

    panel({ autorun: plan({ enabled: false, state: "disabled" }) });
    expect(screen.queryByTestId("autorun-line")).toBeNull();
  });

  it("says who is being waited for and when, in Rust's words, under the quota line", () => {
    panel({ autorun: plan() });

    const line = screen.getByTestId("autorun-line");
    const at = new Date((NOW + 3 * 3_600) * 1000);
    const clock = `${at.getHours().toString().padStart(2, "0")}:${at.getMinutes().toString().padStart(2, "0")}`;
    expect(line.textContent).toContain(
      `Waiting for quota · expected ${["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][at.getDay()] ?? ""} ${clock} · Account 3`,
    );
    // The quota status is still there: two facts, two lines.
    expect(screen.getByText("Quota read just now.")).toBeDefined();
  });

  it("offers pause and cancel while something is in flight, continue and cancel otherwise", () => {
    const onAutoRunControl = vi.fn();
    panel({ autorun: plan(), onAutoRunControl });

    expect(screen.getByRole("button", { name: "Pause automatic continuation" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Continue automatic continuation" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Cancel automatic continuation" }));
    expect(onAutoRunControl).toHaveBeenCalledWith("cancel");
    cleanup();

    panel({ autorun: plan({ state: "paused", waitReason: null }), onAutoRunControl });
    expect(screen.queryByRole("button", { name: "Pause automatic continuation" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Continue automatic continuation" }));
    expect(onAutoRunControl).toHaveBeenCalledWith("resume");
  });

  it("holds the buttons while a control is on its way", () => {
    panel({ autorun: plan(), autorunBusy: true });

    expect(
      screen.getByRole("button", { name: "Pause automatic continuation" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("marks the participating rows and the executing one, and only while the plan is on", () => {
    panel({ autorun: plan({ state: "running" }) });

    expect(screen.getAllByTestId("participant-mark")).toHaveLength(2);
    expect(screen.getByText("Continuing")).toBeDefined();
    cleanup();

    // Waiting for another account's quota: a "continuing" tag on the last attempt's row would
    // contradict the line.
    panel({ autorun: plan() });
    expect(screen.getAllByTestId("participant-mark")).toHaveLength(2);
    expect(screen.queryByText("Continuing")).toBeNull();
    cleanup();

    panel({ autorun: plan({ enabled: false, state: "disabled" }) });
    expect(screen.queryByTestId("participant-mark")).toBeNull();
    expect(screen.queryByText("Continuing")).toBeNull();
  });

  it("says a control did not get through, and that the plan is as it was", () => {
    panel({
      autorun: plan(),
      autorunFailure: {
        step: "pause",
        failure: { command: "pause_autorun", error: { code: "internal", retryable: false } },
      },
    });

    expect(screen.getByRole("status").textContent).toMatch(
      /did not get through \(internal\)\. The plan is as it was/,
    );
  });
});
