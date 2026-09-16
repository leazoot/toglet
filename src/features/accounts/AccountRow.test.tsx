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
      resetCredits: null,
    },
  };
}

function row(account: Partial<AccountView> = {}, quota: Loadable<QuotaView> = ready(BOTH)) {
  return render(
    <AccountRow account={{ ...ACCOUNT, ...account }} quota={quota} nowSeconds={NOW} last={false} />,
  );
}

function withCredits(availableCount: number, earliestExpiry: number | null): Loadable<QuotaView> {
  const held = ready(BOTH);
  return {
    state: "ready",
    value: {
      ...(held as { value: QuotaView }).value,
      resetCredits: { availableCount, earliestExpiry },
    },
  };
}

function descriptions(): string[] {
  return screen.getAllByRole("img").map((node) => node.getAttribute("aria-label") ?? "");
}

describe("an account row", () => {
  afterEach(cleanup);

  it("shows the reset credit count the server reported", () => {
    render(
      <AccountRow account={ACCOUNT} quota={withCredits(2, null)} nowSeconds={NOW} last={false} />,
    );

    expect(screen.getByText("2")).toBeDefined();
  });

  it("shows no reset credit mark when none are held or none were reported", () => {
    render(
      <AccountRow account={ACCOUNT} quota={withCredits(0, null)} nowSeconds={NOW} last={false} />,
    );
    expect(screen.queryByText("0")).toBeNull();

    cleanup();
    // An older Codex never mentions them, which is not the same as holding none.
    row();
    expect(screen.queryByLabelText(/Reset credits/)).toBeNull();
  });

  it("offers the count as a button only when spending one is possible", () => {
    const asked: string[] = [];
    render(
      <AccountRow
        account={ACCOUNT}
        quota={withCredits(3, null)}
        nowSeconds={NOW}
        last={false}
        onResetCredits={(account) => asked.push(account.id)}
      />,
    );

    screen.getByRole("button", { name: /Reset credits/ }).click();
    expect(asked).toEqual([ACCOUNT.id]);
  });

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
    row({}, { state: "failed", failure: { command: "refresh_quota", error: null } });

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
    // The fixture's weekly window has `resetsAt: null`.
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
    expect(screen.queryByText("68%")).toBeNull();
  });

  it("keeps the numbers while a switch runs, and says so where the arrow would be", () => {
    row({ status: "switching" });

    expect(screen.getByText("68%")).toBeDefined();
    expect(screen.getByText(/Switching/)).toBeDefined();
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

describe("the automatic-continuation marks", () => {
  afterEach(cleanup);

  it("draws nothing extra by default", () => {
    row();

    expect(screen.queryByTestId("participant-mark")).toBeNull();
    expect(screen.queryByText("Continuing")).toBeNull();
  });

  it("marks a participant at the avatar, with words for assistive technology", () => {
    render(
      <AccountRow
        account={ACCOUNT}
        quota={ready(BOTH)}
        nowSeconds={NOW}
        last={false}
        participating
      />,
    );

    const mark = screen.getByTestId("participant-mark");
    expect(mark.getAttribute("aria-label")).toBe("Takes part in automatic continuation");
    expect(mark.parentElement?.className).toContain("avatar");
  });

  it("says the executing account is continuing, in the active chip's shape", () => {
    render(
      <AccountRow
        account={{ ...ACCOUNT, isActive: true }}
        quota={ready(BOTH)}
        nowSeconds={NOW}
        last={false}
        executing
      />,
    );

    const chip = screen.getByText("Continuing");
    expect(chip.className).toBe(screen.getByText("Active").className);
  });
});
