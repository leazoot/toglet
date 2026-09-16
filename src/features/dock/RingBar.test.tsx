import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountView, QuotaView, QuotaWindowView } from "../../types/ipc";
import type { Loadable } from "../../types/load";
import type { EdgeBarProps } from "./EdgeBar";
import { RingBar } from "./RingBar";

const NOW = 1_800_000_000;

const ACCOUNT: AccountView = {
  id: "acct-1",
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
      resetCredits: null,
    },
  };
}

function form(overrides: Partial<EdgeBarProps> = {}) {
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
  return render(<RingBar {...props} />);
}

function arcs(container: HTMLElement): (string | null)[] {
  return [...container.querySelectorAll("circle")].map((circle) =>
    circle.getAttribute("stroke-dasharray"),
  );
}

describe("the ring form", () => {
  afterEach(cleanup);

  it("shows both numbers on one line, outer first, and the account's initial in the middle", () => {
    form();

    const row = screen.getByText("68%").parentElement;
    expect(row?.textContent).toBe("68%42%");
    expect(screen.getByText("T")).toBeDefined();
    // No `5H` / `W`: the colours and the order say which is which.
    expect(screen.queryByText("5H")).toBeNull();
    expect(screen.queryByText("W")).toBeNull();
  });

  it("draws the five-hour window on the outer ring and the weekly on the inner", () => {
    const { container } = form();

    // 68% of 2π × 16.5, and 42% of 2π × 11.
    expect(arcs(container)).toContain("70.5 103.67");
    expect(arcs(container)).toContain("29.0 69.12");
  });

  it("gives the healthy weekly ring and its number their own colour", () => {
    // Both healthy - the fixture's 42% weekly is "warn", which rightly keeps the status colour.
    const { container } = form({
      quota: quota([
        { kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null },
        { kind: "weekly", usedPercent: 16, remainingPercent: 84, resetsAt: null },
      ]),
    });

    const arcClasses = [...container.querySelectorAll("circle")].map(
      (c) => c.getAttribute("class") ?? "",
    );
    expect(arcClasses.some((c) => c.includes("healthyInner"))).toBe(true);
    expect(arcClasses.some((c) => c.includes("healthy") && !c.includes("healthyInner"))).toBe(true);
    expect(screen.getByText("84%").className).toContain("value-healthyInner");
    expect(screen.getByText("68%").className).toContain("value-healthy");
    expect(screen.getByText("68%").className).not.toContain("healthyInner");
  });

  it("puts the status colour on the weekly ring when it is low, not its own hue", () => {
    form({
      quota: quota([
        { kind: "five_hour", usedPercent: 2, remainingPercent: 98, resetsAt: null },
        { kind: "weekly", usedPercent: 90, remainingPercent: 10, resetsAt: null },
      ]),
    });

    expect(screen.getByText("10%").className).toContain("value-low");
  });

  it("carries both windows as one sentence the image and the tooltip share", () => {
    form();

    const image = screen.getByRole("img");
    const sentence = image.getAttribute("aria-label") ?? "";
    expect(sentence).toContain("68%");
    expect(sentence).toContain("42%");
    expect(image.getAttribute("title")).toBe(sentence);
  });

  it("shows a weekly window the server did not return as unknown, not as zero", () => {
    const { container } = form({
      quota: quota([{ kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null }]),
    });

    expect(screen.getByText("—")).toBeDefined();
    expect(screen.queryByText("0%")).toBeNull();
    expect(arcs(container)).toContain("0 69.12");
    expect(screen.getByRole("img").getAttribute("aria-label")).toContain("was not returned");
  });

  it("offers to add an account when none has been added, and draws no numbers", () => {
    const onAddAccount = vi.fn();
    form({ account: { state: "ready", value: null }, hasAccounts: false, onAddAccount });

    fireEvent.click(screen.getByTestId("bar-add"));

    expect(onAddAccount).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: /Add a Codex account/ })).toBeDefined();
    expect(screen.queryByText("5H")).toBeNull();
  });

  it("offers to pick an account when accounts exist but none is current", () => {
    const onPickAccount = vi.fn();
    form({ account: { state: "ready", value: null }, hasAccounts: true, onPickAccount });

    fireEvent.click(screen.getByTestId("bar-pick"));

    expect(onPickAccount).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId("bar-add")).toBeNull();
  });

  it("draws no rings and no button while the account is still loading", () => {
    // Nothing is known yet: two empty rings would say a reading came back blank.
    const { container } = form({ account: { state: "loading" }, hasAccounts: false });

    expect(screen.queryByRole("button")).toBeNull();
    expect(container.querySelectorAll("circle")).toHaveLength(0);
    expect(screen.getByRole("img").getAttribute("aria-label")).toContain("Loading");
  });

  it("lights the amber dot with the reason in its label", () => {
    form({ notice: "reauth_required" });

    expect(screen.getByRole("img", { name: /signed in again/ })).toBeDefined();
  });

  it("mirrors its corners with the edge and keeps the text reading left to right", () => {
    form({ side: "left" });

    expect(screen.getByTestId("ring-bar").className).toContain("left");
  });
});
