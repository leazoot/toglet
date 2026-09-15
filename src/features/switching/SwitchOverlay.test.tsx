import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AccountView, SwitchView } from "../../types/ipc";
import { SwitchOverlay } from "./SwitchOverlay";
import type { SwitchOverlayProps } from "./SwitchOverlay";

const TARGET: AccountView = {
  id: "acct-2",
  displayName: "Personal",
  maskedEmail: "ope***@gmail.com",
  planType: "pro",
  status: "ready",
  isActive: false,
};

function view(overrides: Partial<SwitchView> = {}): SwitchView {
  return {
    switched: false,
    progress: 2,
    clientUpToDate: false,
    clients: "clear",
    rollback: "restored",
    error: { code: "auth_write_failed", phase: "write", retryable: true, action: "retry" },
    manualRecoveryRequired: false,
    clientOutcome: null,
    ...overrides,
  };
}

function overlay(overrides: Partial<SwitchOverlayProps> = {}) {
  const props: SwitchOverlayProps = {
    phase: "confirm",
    target: TARGET,
    verdict: null,
    step: 0,
    result: null,
    unreachable: false,
    detailsOpen: false,
    onConfirm: () => undefined,
    onCancel: () => undefined,
    onToggleDetails: () => undefined,
    ...overrides,
  };
  return render(<SwitchOverlay {...props} />);
}

describe("the switch overlay", () => {
  afterEach(cleanup);

  it("shows nothing at all while idle", () => {
    overlay({ phase: "idle" });

    expect(screen.queryByTestId("switch-overlay")).toBeNull();
  });

  it("names the account it is about to switch to", () => {
    overlay();

    expect(screen.getByText("Switch to Personal?")).toBeDefined();
  });

  it("makes checking again the primary action when Codex is running", () => {
    overlay({ phase: "blocked", verdict: "blocked" });

    expect(screen.getByText("Check again")).toBeDefined();
    expect(screen.getByText(/Nothing has been changed/)).toBeDefined();
  });

  it("does not claim Codex is running when the probe could not answer", () => {
    overlay({ phase: "blocked", verdict: "unknown" });

    expect(screen.getByText("Toglet cannot tell what Codex is doing")).toBeDefined();
    expect(screen.queryByText("Codex is still running")).toBeNull();
    expect(screen.getByText(/Nothing has been changed/)).toBeDefined();
  });

  it("marks only the steps that finished", () => {
    overlay({ phase: "running", step: 2 });

    const steps = screen.getAllByRole("listitem");
    expect(steps).toHaveLength(4);
    expect(steps[0]?.className).toContain("stepDone");
    expect(steps[1]?.className).toContain("stepDone");
    expect(steps[2]?.className).not.toContain("stepDone");
    expect(steps[3]?.className).not.toContain("stepDone");
  });

  it("marks nothing at all when nothing has finished", () => {
    overlay({ phase: "running", step: 0 });

    for (const step of screen.getAllByRole("listitem")) {
      expect(step.className).not.toContain("stepDone");
    }
  });

  it("does not call a switch done while Codex is still on the old account", () => {
    overlay({
      phase: "done",
      result: view({ switched: true, clientUpToDate: false, clientOutcome: "closed_not_reopened" }),
    });

    expect(screen.getByText(/still running the previous one/)).toBeDefined();
  });

  it("says plainly that new sessions use the account when Codex is up to date", () => {
    overlay({ phase: "done", result: view({ switched: true, clientUpToDate: true }) });

    expect(screen.getByText("New sessions will use this account.")).toBeDefined();
  });

  it("does not describe the user's own setting as a problem", () => {
    overlay({
      phase: "done",
      result: view({ switched: true, clientUpToDate: false, clientOutcome: "closed_by_choice" }),
    });

    expect(screen.getByText(/as your settings ask/)).toBeDefined();
    expect(screen.queryByText(/still running the previous one/)).toBeNull();
  });

  it("says nothing extra when there was no Codex running to begin with", () => {
    overlay({
      phase: "done",
      result: view({ switched: true, clientUpToDate: false, clientOutcome: "nothing_was_running" }),
    });

    expect(screen.getByText("New sessions will use this account.")).toBeDefined();
  });

  it.each([
    ["not_needed" as const, /still on the account you were on/],
    ["restored" as const, /previous account has been restored/],
    ["restored_unverified" as const, /could not be read back to confirm/],
    ["failed" as const, /could not be put back automatically/],
  ])("says what happened to the previous account when the rollback is %s", (rollback, expected) => {
    overlay({ phase: "failed", result: view({ rollback }) });

    expect(screen.getByText(expected)).toBeDefined();
  });

  it("says the switch never started when the backend could not be reached", () => {
    overlay({ phase: "failed", result: null, unreachable: true });

    expect(screen.getByText(/never started/)).toBeDefined();
    expect(screen.getByText(/Nothing was changed/)).toBeDefined();
  });

  it("hides the failure details until they are asked for", () => {
    overlay({ phase: "failed", result: view() });

    expect(screen.queryByText(/auth_write_failed/)).toBeNull();
    expect(screen.getByText("View details")).toBeDefined();
  });

  it("shows the stable code verbatim once details are open", () => {
    overlay({ phase: "failed", result: view(), detailsOpen: true });

    expect(screen.getByText(/auth_write_failed · write/)).toBeDefined();
  });

  it("offers another attempt only when Rust said the failure is retryable", () => {
    overlay({
      phase: "failed",
      result: view({
        error: { code: "config_layer_readonly", phase: "write", retryable: false, action: "none" },
      }),
    });

    expect(screen.queryByText("Try again")).toBeNull();
    expect(screen.getByText("Close")).toBeDefined();
  });

  it("calls back when the switch is confirmed", () => {
    const onConfirm = vi.fn();
    overlay({ onConfirm });

    screen.getByText("Switch account").click();

    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
