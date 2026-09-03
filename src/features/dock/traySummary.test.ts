import { describe, expect, it } from "vitest";

import type { AccountView, QuotaView, QuotaWindowView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import { traySummary } from "./traySummary";

const NOW = 1_800_000_000;

const ACCOUNT: AccountView = {
  id: "acct-1",
  displayName: "Team",
  maskedEmail: "lea***@gmail.com",
  planType: "plus",
  status: "active",
  isActive: true,
};

function ready(windows: readonly QuotaWindowView[], fetchedAt = NOW): Loadable<QuotaView> {
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

const BOTH: readonly QuotaWindowView[] = [
  { kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null },
  { kind: "weekly", usedPercent: 58, remainingPercent: 42, resetsAt: null },
];

const HELD: Loadable<AccountView | null> = { state: "ready", value: ACCOUNT };

describe("the tray summary", () => {
  it("puts the account and both windows on one line", () => {
    expect(traySummary(HELD, ready(BOTH), NOW, true)).toBe("Team · 5H 68% · W 42%");
  });

  it("never shows the address", () => {
    // A tray menu is visible to anyone looking over a shoulder, and the name already says which
    // account it is.
    expect(traySummary(HELD, ready(BOTH), NOW, true)).not.toContain("@");
  });

  it("shows a window with no reading as an em dash, not as zero", () => {
    // The rule carries over unchanged: the tray is another surface, not another set of rules.
    const line = traySummary(HELD, ready(BOTH.slice(0, 1)), NOW, true);

    expect(line).toContain("W —");
    expect(line).not.toContain("W 0%");
  });

  it("says a reading is cached when it has aged out", () => {
    // Often the only thing on screen, so it has to carry the same warning the panel does.
    expect(traySummary(HELD, ready(BOTH, NOW - 3600), NOW, true)).toContain("cached");
  });

  it("does not call a fresh reading cached", () => {
    expect(traySummary(HELD, ready(BOTH), NOW, true)).not.toContain("cached");
  });

  it("says it is still reading rather than showing a number it does not have", () => {
    expect(traySummary(HELD, { state: "loading" }, NOW, true)).toBe("Team - reading quota…");
  });

  it("says there is no account when there genuinely is none", () => {
    expect(
      traySummary({ state: "ready", value: null }, { state: "loading" }, NOW, false),
    ).toContain("No account");
  });

  it("says the current account is not known when accounts exist but none is current", () => {
    // Saying "no account has been added" beside a list of them is the wrong sentence.
    const line = traySummary({ state: "ready", value: null }, { state: "loading" }, NOW, true);

    expect(line).toContain("No current account");
    expect(line).not.toContain("No account has");
  });

  it("says the state could not be read rather than inventing one", () => {
    const line = traySummary(
      { state: "failed", failure: { command: "list_accounts" } },
      { state: "loading" },
      NOW,
      true,
    );

    expect(line).toContain("could not read");
  });
});
