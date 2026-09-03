import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountView, QuotaView } from "../../types/ipc";
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
    onOpenSettings: () => undefined,
    onAddAccount: () => undefined,
    overlay: null,
    sheet: null,
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
    panel({ accounts: { state: "failed", failure: { command: "list_accounts" } }, quotas: {} });

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
        onOpenSettings={() => undefined}
        onAddAccount={() => undefined}
        overlay={null}
        sheet={null}
      />,
    );

    const names = screen.getAllByTestId("account-row").map((node) => node.textContent);
    expect(names[0]).toContain("Account 3");
    expect(names[2]).toContain("Account 1");
  });
});
