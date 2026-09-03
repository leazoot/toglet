import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import type { AccountView, SettingsView } from "../../types/ipc";
import type { Removal } from "../accounts/store";
import { SettingsSheet } from "./SettingsSheet";
import { useSettings } from "./store";

function stored(overrides: Partial<SettingsView> = {}): SettingsView {
  return {
    dockEdge: "right",
    verticalOffset: 0,
    alwaysOnTop: true,
    activeRefreshSeconds: 60,
    inactiveRefreshSeconds: 300,
    reopenCodexAfterSwitch: true,
    theme: "system",
    reduceMotion: false,
    // Pinned rather than left at `system`, so these assertions read the English dictionary
    // whatever locale the machine running them is set to.
    language: "en",
    ...overrides,
  };
}

describe("the settings store", () => {
  beforeEach(() => {
    invoke.mockReset();
    useSettings.setState({ settings: { state: "loading" }, saving: false });
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.removeAttribute("data-motion");
  });

  it("holds what Rust says is stored, not what was asked for", async () => {
    // Rust corrects a value out of range. Showing the requested number would show a setting that
    // is not in force.
    invoke.mockResolvedValueOnce(stored());
    await useSettings.getState().load();
    invoke.mockResolvedValueOnce(stored({ activeRefreshSeconds: 30 }));

    await useSettings.getState().update({ activeRefreshSeconds: 5 });

    const held = useSettings.getState().settings;
    expect(held.state).toBe("ready");
    if (held.state !== "ready") return;
    expect(held.value.activeRefreshSeconds).toBe(30);
  });

  it("leaves the settings in force showing when a change did not get through", async () => {
    invoke.mockResolvedValueOnce(stored());
    await useSettings.getState().load();

    invoke.mockRejectedValueOnce(new Error("bridge unavailable"));
    await useSettings.getState().update({ alwaysOnTop: false });

    const held = useSettings.getState().settings;
    expect(held.state).toBe("ready");
    if (held.state !== "ready") return;
    // Nothing about the old values has become untrue - they are still the ones in force.
    expect(held.value.alwaysOnTop).toBe(true);
    expect(useSettings.getState().saving).toBe(false);
  });

  it("puts an explicit theme on the document and takes it off again for system", async () => {
    invoke.mockResolvedValueOnce(stored({ theme: "dark" }));
    await useSettings.getState().load();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

    invoke.mockResolvedValueOnce(stored({ theme: "system" }));
    await useSettings.getState().update({ theme: "system" });

    // Removing it is what puts the media query back in charge, which is what "system" means.
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  it("never uses the motion setting to take away a system preference", async () => {
    // The toggle can only add reduced motion. Off means "follow the system", so the attribute
    // comes off rather than being set to an opposite value.
    invoke.mockResolvedValueOnce(stored({ reduceMotion: true }));
    await useSettings.getState().load();
    expect(document.documentElement.getAttribute("data-motion")).toBe("reduced");

    invoke.mockResolvedValueOnce(stored({ reduceMotion: false }));
    await useSettings.getState().update({ reduceMotion: false });

    expect(document.documentElement.hasAttribute("data-motion")).toBe(false);
  });
});

describe("the settings sheet", () => {
  afterEach(cleanup);

  function sheet(
    settings = stored(),
    saving = false,
    onChange = vi.fn(),
    accounts: readonly AccountView[] = [],
    onRemove = vi.fn(),
    removal: Removal | null = null,
  ) {
    render(
      <SettingsSheet
        settings={{ state: "ready", value: settings }}
        saving={saving}
        onChange={onChange}
        onClose={() => undefined}
        accounts={{ state: "ready", value: accounts }}
        removal={removal}
        onRemove={onRemove}
        onDismissRemoval={() => undefined}
      />,
    );
    return { onChange, onRemove };
  }

  const IN_USE: AccountView = {
    id: "acct-1",
    displayName: "Team",
    maskedEmail: "lea***@gmail.com",
    planType: "plus",
    status: "active",
    isActive: true,
  };
  const SPARE: AccountView = {
    id: "acct-2",
    displayName: "Personal",
    maskedEmail: "per***@gmail.com",
    planType: "plus",
    status: "ready",
    isActive: false,
  };

  it("shows only the settings that do something today", () => {
    sheet();

    expect(screen.getByText("Dock to")).toBeDefined();
    expect(screen.getByText("Theme")).toBeDefined();
    // Absent because the behaviour behind them does not exist yet.
    expect(screen.queryByText(/Launch at login/i)).toBeNull();
    expect(screen.queryByText(/fullscreen/i)).toBeNull();
  });

  it("marks the stored choice, not the first option", () => {
    sheet(stored({ dockEdge: "left", theme: "dark" }));

    expect(screen.getByRole("radio", { name: "Left" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: "Right" }).getAttribute("aria-checked")).toBe("false");
    expect(screen.getByRole("radio", { name: "Dark" }).getAttribute("aria-checked")).toBe("true");
  });

  it("states a toggle's state to assistive technology, not only in colour", () => {
    sheet(stored({ alwaysOnTop: false }));

    expect(screen.getByRole("switch", { name: "Always on top" }).getAttribute("aria-checked")).toBe(
      "false",
    );
  });

  it("offers the two languages the design draws, each named in itself", () => {
    sheet();

    expect(screen.getByText("Language")).toBeDefined();
    expect(screen.getByRole("radio", { name: "English" })).toBeDefined();
    expect(screen.getByRole("radio", { name: "中文" })).toBeDefined();
    // The design's control has two buttons and no "System" (Toglet.dc.html, board 08).
    expect(screen.queryByRole("radio", { name: "system" })).toBeNull();
  });

  it("marks the language a stored `system` currently resolves to", () => {
    // A fresh install holds `system`. Leaving both buttons unmarked would say the interface is
    // in no language at all, when it is plainly in one.
    sheet(stored({ language: "system" }));

    expect(screen.getByRole("radio", { name: "English" }).getAttribute("aria-checked")).toBe(
      "true",
    );
  });

  it("asks for exactly the one setting that was changed", () => {
    const { onChange } = sheet();

    screen.getByRole("radio", { name: "Left" }).click();

    expect(onChange).toHaveBeenCalledWith({ dockEdge: "left" });
  });

  it("shows the intervals in the compact form the rest of the interface uses", () => {
    sheet(stored({ activeRefreshSeconds: 30, inactiveRefreshSeconds: 3600 }));

    expect(screen.getByRole("radio", { name: "30s" }).getAttribute("aria-checked")).toBe("true");
    expect(screen.getByRole("radio", { name: "1h" }).getAttribute("aria-checked")).toBe("true");
  });

  it("offers a 25-minute step for the current account", () => {
    // Asked for by the user. Within the 30-3600 Rust accepts.
    const { onChange } = sheet(stored({ activeRefreshSeconds: 1500 }));

    const step = screen.getByRole("radio", { name: "25m" });
    expect(step.getAttribute("aria-checked")).toBe("true");

    fireEvent.click(screen.getByRole("radio", { name: "1m" }));
    expect(onChange).toHaveBeenCalledWith({ activeRefreshSeconds: 60 });
  });

  it("offers to remove every account, and marks the one Codex is using", () => {
    sheet(stored(), false, vi.fn(), [IN_USE, SPARE]);

    expect(screen.getByRole("button", { name: "Remove Personal" })).toBeDefined();
    expect(screen.getByRole("button", { name: "Remove Team" })).toBeDefined();
    // The explanation is on the mark, for whoever hovers it, rather than in the row.
    const marks = screen.getAllByTestId("in-use-mark");
    expect(marks).toHaveLength(1);
    expect(marks[0]?.getAttribute("aria-label")).toMatch(/signs Codex out/);
  });

  it("calls the second press on the account in use what it is: a sign-out", () => {
    // An explicit choice. The words change; the two-step shape does not.
    const { onRemove } = sheet(stored(), false, vi.fn(), [IN_USE, SPARE]);

    fireEvent.click(screen.getByRole("button", { name: "Remove Team" }));
    expect(screen.queryByRole("button", { name: "Confirm removal" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Sign out and remove" }));

    expect(onRemove).toHaveBeenCalledWith(IN_USE);
  });

  it("says what happened to Codex's sign-in when a sign-out failed", () => {
    sheet(stored(), false, vi.fn(), [IN_USE], vi.fn(), {
      phase: "failed",
      name: "Team",
      rollback: "restored",
    });

    const alert = screen.getByRole("alert").textContent;
    expect(alert).toMatch(/could not be signed out of Team/);
    expect(alert).toMatch(/previous account has been restored/);
  });

  it("says nothing about Codex when a removal that touched nothing failed", () => {
    sheet(stored(), false, vi.fn(), [IN_USE, SPARE], vi.fn(), {
      phase: "failed",
      name: "Personal",
      rollback: null,
    });

    const alert = screen.getByRole("alert").textContent;
    expect(alert).toMatch(/Personal could not be removed/);
    expect(alert).not.toMatch(/signed out/);
  });

  it("removes only after a second press, and not after a cancel", () => {
    // A single press on a 38px row is too easy to make by accident for something that deletes
    // a saved sign-in.
    const { onRemove } = sheet(stored(), false, vi.fn(), [IN_USE, SPARE]);

    fireEvent.click(screen.getByRole("button", { name: "Remove Personal" }));
    expect(onRemove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onRemove).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Remove Personal" }));
    fireEvent.click(screen.getByRole("button", { name: "Confirm removal" }));
    expect(onRemove).toHaveBeenCalledWith(SPARE);
  });

  it("says when the list lost the account but the credential store kept its sign-in", () => {
    // The honest half-result. Folding it into a failure would contradict the list, which no
    // longer shows the account.
    sheet(stored(), false, vi.fn(), [IN_USE], vi.fn(), { phase: "orphaned", name: "Personal" });

    expect(screen.getByRole("alert").textContent).toMatch(/Personal was removed from the list/);
  });

  it("does not accept a second change while one is still on its way", () => {
    sheet(stored(), true);

    expect(screen.getByRole("radio", { name: "Left" }).hasAttribute("disabled")).toBe(true);
  });

  it("says the settings could not be read rather than showing defaults as if they were stored", () => {
    render(
      <SettingsSheet
        settings={{ state: "failed", failure: { command: "read_settings" } }}
        saving={false}
        onChange={() => undefined}
        onClose={() => undefined}
        accounts={{ state: "ready", value: [] }}
        removal={null}
        onRemove={() => undefined}
        onDismissRemoval={() => undefined}
      />,
    );

    expect(screen.getByText(/could not be read/)).toBeDefined();
    expect(screen.queryByRole("radio", { name: "Left" })).toBeNull();
  });
});
