import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { AccountStatus, AccountView, QuotaView, QuotaWindowView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { AccountRow } from "./AccountRow";

const NOW = 1_800_000_000;

const ACCOUNT: AccountView = {
  id: "acct-1",
  displayName: "Team",
  maskedEmail: "lea***@gmail.com",
  planType: "plus",
  status: "ready",
  isActive: false,
};

const BOTH: readonly QuotaWindowView[] = [
  { kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: NOW + 51 * 60 },
  { kind: "weekly", usedPercent: 58, remainingPercent: 42, resetsAt: null },
];

function ready(windows: readonly QuotaWindowView[]): Loadable<QuotaView> {
  return {
    state: "ready",
    value: {
      accountId: ACCOUNT.id,
      windows,
      fetchedAt: NOW,
      source: "codex_app_server",
      stale: false,
      lastErrorCode: null,
    },
  };
}

function row(account: Partial<AccountView> = {}, quota: Loadable<QuotaView> = ready(BOTH)) {
  return render(
    <AccountRow account={{ ...ACCOUNT, ...account }} quota={quota} nowSeconds={NOW} last={false} />,
  );
}

function descriptions(): string[] {
  return screen.getAllByRole("img").map((node) => node.getAttribute("aria-label") ?? "");
}

describe("an account row", () => {
  afterEach(cleanup);

  it("shows the name, plan, address and both quota windows", () => {
    row();

    expect(screen.getByText("Team")).toBeDefined();
    expect(screen.getByText("plus")).toBeDefined();
    expect(screen.getByText("lea***@gmail.com")).toBeDefined();
    expect(screen.getByText("68%")).toBeDefined();
    expect(screen.getByText("42%")).toBeDefined();
  });

  it("says an unknown plan and a missing address are unknown", () => {
    row({ planType: null, maskedEmail: null });

    expect(screen.getByText("Plan unknown")).toBeDefined();
    expect(screen.getByText("No address recorded")).toBeDefined();
  });

  it("marks the active account with a word, not only a colour", () => {
    row({ isActive: true });

    expect(screen.getByText("Active")).toBeDefined();
  });

  it("does not mark a row the backend did not call active", () => {
    row();

    expect(screen.queryByText("Active")).toBeNull();
  });

  it("shows a weekly window the server did not return as unknown, not as zero", () => {
    row({}, ready(BOTH.slice(0, 1)));

    expect(screen.getByText("—")).toBeDefined();
    expect(screen.queryByText("0%")).toBeNull();
    expect(descriptions().some((text) => text.includes("was not returned"))).toBe(true);
  });

  it("shows a reading that failed as unreadable rather than as zero", () => {
    row({}, { state: "failed", failure: { command: "refresh_quota" } });

    expect(screen.getAllByText("—")).toHaveLength(2);
    expect(descriptions().some((text) => text.includes("could not be read"))).toBe(true);
  });

  it("shows a genuine zero as zero", () => {
    row(
      {},
      ready([
        { kind: "five_hour", usedPercent: 100, remainingPercent: 0, resetsAt: null },
        { kind: "weekly", usedPercent: 100, remainingPercent: 0, resetsAt: null },
      ]),
    );

    expect(screen.getAllByText("0%")).toHaveLength(2);
    expect(screen.queryByText("—")).toBeNull();
  });

  it("shows a low reading with its own tone but keeps the number readable", () => {
    row({}, ready([{ kind: "five_hour", usedPercent: 88, remainingPercent: 12, resetsAt: null }]));

    expect(screen.getByText("12%")).toBeDefined();
  });

  it("counts down to the reset in the compact form", () => {
    row();

    expect(screen.getByText("51m")).toBeDefined();
  });

  it("leaves the reset column empty rather than inventing a time", () => {
    // The weekly window in this fixture carries `resetsAt: null`.
    row();

    expect(screen.queryByText("null")).toBeNull();
    expect(screen.queryByText("NaNm")).toBeNull();
  });

  it.each<[AccountStatus, RegExp]>([
    ["reauth_required", /signed in again/],
    ["unsupported", /cannot be managed/],
  ])("replaces the quota lines with a notice when the account is %s", (status, expected) => {
    row({ status });

    expect(screen.getByText(expected)).toBeDefined();
    // The row is a fixed 90 tall; the notice takes the space the quota lines would have used.
    expect(screen.queryByText("68%")).toBeNull();
  });

  it("keeps the numbers while a switch runs, and says so where the arrow would be", () => {
    // The design's SWITCHING row (Toglet.dc.html §05) keeps both quota lines: they are still
    // true while the switch runs, and this is the row the panel is currently about. Only the
    // arrow column changes - to a spinner, which is why the word goes with it.
    row({ status: "switching" });

    expect(screen.getByText("68%")).toBeDefined();
    expect(screen.getByText(/Switching/)).toBeDefined();
    // No affordance to start a second switch on a row already being switched to.
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("shows the amber badge only for an account that needs signing in again", () => {
    row({ status: "reauth_required" });

    expect(screen.getByLabelText(/needs to be signed in again/)).toBeDefined();
  });

  it("keeps the quota lines for a status that does not stop the numbers being true", () => {
    row({ status: "stale" });

    expect(screen.getByText("68%")).toBeDefined();
  });
});
