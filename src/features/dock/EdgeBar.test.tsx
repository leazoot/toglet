import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountView, QuotaView, QuotaWindowView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { EdgeBar } from "./EdgeBar";
import type { EdgeBarProps } from "./EdgeBar";

const NOW = 1_800_000_000;

const ACCOUNT: AccountView = {
  id: "acct-1",
  // Not a name starting with `W`: that is the weekly ring's own label, and a test that cannot
  // tell the two apart proves nothing.
  displayName: "Team",
  maskedEmail: "lea***@gmail.com",
  planType: "plus",
  status: "active",
  isActive: true,
};

function quota(windows: readonly QuotaWindowView[], fetchedAt = NOW): Loadable<QuotaView> {
  return {
    state: "ready",
    value: {
      accountId: ACCOUNT.id,
      windows,
      fetchedAt,
      source: "codex_app_server",
      stale: false,
      lastErrorCode: null,
    },
  };
}

function bar(overrides: Partial<EdgeBarProps> = {}) {
  const props: EdgeBarProps = {
    side: "right",
    account: { state: "ready", value: ACCOUNT },
    hasAccounts: true,
    quota: quota([
      { kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: NOW + 51 * 60 },
      { kind: "weekly", usedPercent: 58, remainingPercent: 42, resetsAt: null },
    ]),
    notice: null,
    nowSeconds: NOW,
    ...overrides,
  };
  return render(<EdgeBar {...props} />);
}

/** The ring images carry the sentences; the visible percentages are decorative duplicates. */
function descriptions(): string[] {
  return screen.getAllByRole("img").map((node) => node.getAttribute("aria-label") ?? "");
}

describe("the collapsed bar", () => {
  afterEach(cleanup);

  it("says Codex is using none of these accounts rather than that there is no account", () => {
    // Two accounts in the list and none verified as current is not "no account has been added",
    // and the bar used to say exactly that.
    bar({ account: { state: "ready", value: null }, hasAccounts: true });

    expect(screen.getByTitle(/using none of these accounts/)).toBeDefined();
    expect(screen.queryByTitle(/No account has been added/)).toBeNull();
  });

  it("says no account has been added when none has", () => {
    bar({ account: { state: "ready", value: null }, hasAccounts: false });

    expect(screen.getByTitle(/No account has been added/)).toBeDefined();
  });

  it("offers to add an account when none has been added", () => {
    // The bar is the whole interface until the first account exists. A grey well and a hidden
    // sentence left a first run with nothing to press.
    const onAddAccount = vi.fn();
    bar({ account: { state: "ready", value: null }, hasAccounts: false, onAddAccount });

    fireEvent.click(screen.getByTestId("bar-add"));

    expect(onAddAccount).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /Add a Codex account/ })).toBeDefined();
    expect(screen.queryByText("5H")).toBeNull();
  });

  it("keeps the plain well, without the add button, while the list is still loading", () => {
    // Nothing is known yet. Offering to add would say "there are none", which may be false.
    bar({ account: { state: "loading" }, hasAccounts: false });

    expect(screen.queryByTestId("bar-add")).toBeNull();
  });

  it("offers to pick an account, not to add one, when accounts exist but none is current", () => {
    // That state's next step is to switch to one of the rows. The bar used to draw a plain well
    // there, which right after the first account was added looked broken.
    const onPickAccount = vi.fn();
    bar({ account: { state: "ready", value: null }, hasAccounts: true, onPickAccount });

    expect(screen.queryByTestId("bar-add")).toBeNull();
    fireEvent.click(screen.getByTestId("bar-pick"));

    expect(onPickAccount).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /Choose one to switch to/ })).toBeDefined();
    expect(screen.queryByText("5H")).toBeNull();
  });

  it("shows both quota windows with their percentages", () => {
    bar();

    expect(screen.getByText("68%")).toBeDefined();
    expect(screen.getByText("42%")).toBeDefined();
    expect(screen.getByText("5H")).toBeDefined();
    expect(screen.getByText("W")).toBeDefined();
  });

  it("draws the arc the design's formula gives", () => {
    const { container } = bar();

    const arcs = [...container.querySelectorAll("circle")].map((circle) =>
      circle.getAttribute("stroke-dasharray"),
    );
    expect(arcs).toContain("70.5 103.67");
  });

  it("shows a weekly window the server did not return as unknown, not as zero", () => {
    // `0%` would say the user has run out; nothing of the sort was reported.
    bar({
      quota: quota([{ kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null }]),
    });

    expect(screen.getByText("—")).toBeDefined();
    expect(screen.queryByText("0%")).toBeNull();
    expect(descriptions().some((text) => text.includes("was not returned"))).toBe(true);
  });

  it("tells a reading that failed apart from one the server did not return", () => {
    bar({ quota: { state: "failed", failure: { command: "refresh_quota" } } });

    const texts = descriptions();
    expect(texts.some((text) => text.includes("could not be read"))).toBe(true);
    expect(texts.some((text) => text.includes("was not returned"))).toBe(false);
  });

  it("says a reading is still on its way rather than calling it a failure", () => {
    bar({ quota: { state: "loading" } });

    expect(descriptions().some((text) => text.startsWith("Reading"))).toBe(true);
  });

  it("shows a genuine zero as zero", () => {
    bar({
      quota: quota([
        { kind: "five_hour", usedPercent: 100, remainingPercent: 0, resetsAt: null },
        { kind: "weekly", usedPercent: 100, remainingPercent: 0, resetsAt: null },
      ]),
    });

    expect(screen.getAllByText("0%")).toHaveLength(2);
    expect(screen.queryByText("—")).toBeNull();
  });

  it("marks a reading that has aged out as cached", () => {
    bar({
      quota: quota(
        [{ kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null }],
        NOW - 3600,
      ),
    });

    expect(descriptions().some((text) => text.includes("cached"))).toBe(true);
  });

  it("does not call a fresh reading cached", () => {
    bar();

    expect(descriptions().some((text) => text.includes("cached"))).toBe(false);
  });

  it("counts down to the reset in the compact form", () => {
    bar();

    expect(descriptions().some((text) => text.includes("Resets in 51m"))).toBe(true);
  });

  it("draws no rings while the account list is still arriving", () => {
    // Two empty rings would claim a reading was attempted. None was - there is nothing yet to
    // read a quota for.
    bar({ account: { state: "loading" }, quota: { state: "loading" } });

    expect(screen.queryByText("5H")).toBeNull();
    expect(screen.getByTitle(/Loading the current account/)).toBeDefined();
  });

  it("says there is no account rather than showing an empty one", () => {
    bar({
      account: { state: "ready", value: null },
      hasAccounts: false,
      quota: { state: "loading" },
    });

    expect(screen.getByTitle(/No account has been added yet/)).toBeDefined();
    expect(screen.queryByText("5H")).toBeNull();
  });

  it("lights the dot with the reason when an account needs signing in again", () => {
    bar({ notice: "reauth_required" });

    expect(screen.getByLabelText(/needs to be signed in again/)).toBeDefined();
  });

  it("says an unrepaired switch leaves the signed-in account in doubt", () => {
    bar({ notice: "recovery_failed" });

    expect(screen.getByLabelText(/could not be repaired/)).toBeDefined();
  });

  it("shows no dot when there is nothing to report", () => {
    bar();

    expect(screen.queryByLabelText(/signed in again/)).toBeNull();
  });

  it("uses the account's own initial", () => {
    bar();

    expect(screen.getByText("T")).toBeDefined();
  });

  it("mirrors its geometry when docked to the left", () => {
    const right = bar().container.querySelector("[data-testid='edge-bar']")?.className;
    cleanup();
    const left = bar({ side: "left" }).container.querySelector(
      "[data-testid='edge-bar']",
    )?.className;

    expect(right).toContain("right");
    expect(left).toContain("left");
    expect(left).not.toBe(right);
  });
});
