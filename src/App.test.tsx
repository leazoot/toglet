import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() =>
  vi.fn<(event: string, handler: (event: { payload: unknown }) => void) => Promise<() => void>>(
    () => Promise.resolve(() => undefined),
  ),
);
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { App } from "./App";
import { setLanguage } from "./i18n";
import { useAccounts } from "./features/accounts/store";
import { useAdding } from "./features/onboarding/store";
import { useQuota } from "./features/quotas/store";
import { useSettings } from "./features/settings/store";
import { useStartup } from "./features/startup/store";
import { useSwitching } from "./features/switching/store";
import type { AccountView, SettingsPatch, SettingsView } from "./types/ipc";

const NOW = Math.floor(Date.now() / 1000);

const ACTIVE: AccountView = {
  id: "acct-1",
  displayName: "Team",
  maskedEmail: "lea***@gmail.com",
  planType: "plus",
  status: "active",
  isActive: true,
};

const OTHER: AccountView = {
  id: "acct-2",
  displayName: "Personal",
  maskedEmail: "ope***@gmail.com",
  planType: "pro",
  status: "ready",
  isActive: false,
};

const SETTINGS: SettingsView = {
  dockEdge: "right",
  dockShape: "bar",
  verticalOffset: 0,
  alwaysOnTop: true,
  activeRefreshSeconds: 60,
  inactiveRefreshSeconds: 300,
  reopenCodexAfterSwitch: true,
  theme: "system",
  reduceMotion: false,
  // Pinned so assertions read English whatever the machine's locale.
  language: "en",
};

/** The stored settings for the run in progress. Rust answers a change with the whole of them. */
let stored: SettingsView = SETTINGS;

function quota(accountId: string): unknown {
  return {
    accountId,
    windows: [
      { kind: "five_hour", usedPercent: 32, remainingPercent: 68, resetsAt: null },
      { kind: "weekly", usedPercent: 58, remainingPercent: 42, resetsAt: null },
    ],
    fetchedAt: NOW,
    source: "codex_app_server",
    stale: false,
    lastErrorCode: null,
    resetCredits: null,
  };
}

function answer(command: string, args?: { accountId?: string; patch?: SettingsPatch }): unknown {
  switch (command) {
    case "list_accounts":
      return [ACTIVE, OTHER];
    case "detect_environment_command":
      return { checks: [] };
    case "startup_recovery":
      return null;
    case "refresh_quota":
      return quota(args?.accountId ?? "acct-1");
    case "read_settings":
      return stored;
    case "update_settings":
      // Rust answers a change with the full stored settings.
      stored = { ...stored, ...args?.patch };
      return stored;
    case "set_dock_expansion":
    case "move_dock":
      return null;
    case "end_drag":
      stored = { ...stored, verticalOffset: 120 };
      return stored;
    case "set_tray_summary":
    case "set_tray_labels":
      return null;
    case "inspect_clients":
      return "clear";
    case "read_autorun":
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
        updatedAt: "2026-09-11T00:00:00Z",
      };
    case "switch_account":
      return {
        switched: true,
        progress: 4,
        clientUpToDate: true,
        clients: "clear",
        rollback: null,
        error: null,
        manualRecoveryRequired: false,
        clientOutcome: "nothing_was_running",
      };
    default:
      throw new Error(`unexpected command ${command}`);
  }
}

function reply(overrides: Partial<Record<string, () => Promise<unknown>>> = {}): void {
  invoke.mockImplementation(
    (command: string, args?: { accountId?: string; patch?: SettingsPatch }) => {
      const override = overrides[command];
      return override === undefined ? Promise.resolve(answer(command, args)) : override();
    },
  );
}

function calls(command: string): unknown[][] {
  return invoke.mock.calls.filter(([name]) => name === command);
}

/** Advances the clock and lets React flush what the timers set in motion. */
async function settle(milliseconds: number): Promise<void> {
  await act(async () => {
    vi.advanceTimersByTime(milliseconds);
    await Promise.resolve();
  });
}

/** Opens the panel. No stylesheet is attached, so the hover delay uses its fallback. */
async function open(): Promise<void> {
  fireEvent.pointerEnter(screen.getByTestId("dock-bar"));
  await settle(200);
}

/**
 * Closes the panel: the 260ms grace, then the 160ms exit. Two advances, because the exit timer is
 * set only after React renders the first close.
 */
async function close(): Promise<void> {
  fireEvent.pointerLeave(screen.getByTestId("dock-bar"));
  await settle(300);
  await settle(200);
}

/** Calls what the interface registered for a tray event, as Rust would when the menu is used. */
async function tray(event: string): Promise<void> {
  await emit(event, null);
}

/** Delivers an event with a payload, as Rust does when it emits one. */
async function emit(event: string, payload: unknown): Promise<void> {
  const registered = listen.mock.calls.filter((call) => call[0] === event);
  const handler = registered[registered.length - 1]?.[1];
  await act(async () => {
    handler?.({ payload });
    await Promise.resolve();
  });
}

describe("the docked application", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    invoke.mockReset();
    reply();
    useAccounts.setState({ accounts: { state: "loading" } });
    useAdding.getState().dismiss();
    useStartup.setState({ environment: { state: "loading" }, recovery: { state: "loading" } });
    useQuota.setState({ quotas: {}, refreshing: false });
    stored = SETTINGS;
    useSettings.setState({ settings: { state: "loading" }, saving: false });
    // The active language is module state and outlives store resets.
    setLanguage("en");
    useSwitching.setState({
      phase: "idle",
      target: null,
      verdict: null,
      step: 0,
      result: null,
      failure: null,
      detailsOpen: false,
    });
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("shows only the bar until the pointer has lingered", async () => {
    render(<App />);
    await screen.findByText("68%");

    fireEvent.pointerEnter(screen.getByTestId("dock-bar"));
    expect(screen.queryByTestId("panel")).toBeNull();

    await open();
    expect(screen.getByTestId("panel")).toBeDefined();
  });

  it("does not open for a pointer that only passes over the edge", async () => {
    // The bar sits exactly where a pointer travels on its way to a scrollbar.
    render(<App />);
    await screen.findByText("68%");

    fireEvent.pointerEnter(screen.getByTestId("dock-bar"));
    fireEvent.pointerLeave(screen.getByTestId("dock-bar"));
    await act(async () => {
      vi.advanceTimersByTime(500);
      await Promise.resolve();
    });

    expect(screen.queryByTestId("panel")).toBeNull();
  });

  it("closes again after the pointer leaves", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    await close();

    expect(screen.queryByTestId("panel")).toBeNull();
  });

  it("tells Rust the panel is open, and nothing about its size", async () => {
    // The window never resizes; Rust only needs the pointer-gate state.
    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(invoke).toHaveBeenCalledWith("set_dock_expansion", { expanded: true });
    const sizes = invoke.mock.calls.filter(
      (call) => call[0] === "set_dock_expansion" && "contentHeight" in (call[1] ?? {}),
    );
    expect(sizes).toHaveLength(0);
  });

  it("tells Rust when the panel has closed, so the strip lets clicks through again", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();
    invoke.mockClear();

    await close();

    expect(invoke).toHaveBeenCalledWith("set_dock_expansion", { expanded: false });
  });

  it("lets the panel enter in the frame it renders", async () => {
    // No measuring stage: the window is always its final size.
    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(screen.getByTestId("dock").dataset["stage"]).toBe("open");
  });

  it("holds the panel on screen for its exit animation", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    fireEvent.pointerLeave(screen.getByTestId("dock-bar"));
    await settle(300);

    // Past the grace, into the exit: still on screen while it fades.
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("leaving");
    expect(screen.getByTestId("panel")).toBeDefined();

    await settle(200);

    expect(screen.queryByTestId("panel")).toBeNull();
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("closed");
  });

  it("stays open while the pointer crosses from the panel to the bar", async () => {
    // The bar is a second hover target; moving between the two must not count as leaving.
    render(<App />);
    await screen.findByText("68%");
    await open();

    fireEvent.pointerLeave(screen.getByTestId("dock-bar"));
    fireEvent.pointerEnter(screen.getByTestId("dock-panel"));
    await settle(500);

    expect(screen.getByTestId("dock").dataset["stage"]).toBe("open");
  });

  it("reopens in place when the pointer comes back during the exit", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();
    invoke.mockClear();
    // Intent (120ms) and exit end (160ms) are 40ms apart; real time could cross that gap during a
    // render, so the clock moves only when the test moves it.
    vi.useFakeTimers({ shouldAdvanceTime: false });

    fireEvent.pointerLeave(screen.getByTestId("dock-bar"));
    await settle(300);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("leaving");

    fireEvent.pointerEnter(screen.getByTestId("dock-bar"));
    // Two advances, so React acts on the intent before the exit ends.
    await settle(130);
    await settle(100);

    expect(screen.getByTestId("dock").dataset["stage"]).toBe("open");
    expect(screen.getByTestId("panel")).toBeDefined();
  });

  it("re-reads the account list after a sign-in that produced an account already held", async () => {
    // Rust may have marked that account current.
    render(<App />);
    await screen.findByTestId("edge-bar");
    const before = invoke.mock.calls.filter(([command]) => command === "list_accounts").length;

    act(() => {
      useAdding.setState({ phase: "duplicate", account: ACTIVE });
    });
    await settle(0);

    const after = invoke.mock.calls.filter(([command]) => command === "list_accounts").length;
    expect(after).toBe(before + 1);
  });

  it("dims the nub together with the panel while a sheet is open", async () => {
    // The nub sits outside the panel, so it needs its own scrim.
    render(<App />);
    await screen.findByTestId("edge-bar");
    await open();
    expect(screen.queryByTestId("nub-scrim")).toBeNull();

    await tray("tray://settings");

    expect(screen.getByTestId("nub-scrim")).toBeDefined();
  });

  it("closes the settings sheet along with the panel when the pointer leaves", async () => {
    // Settings save as they change, so the sheet closes after the panel's exit and the next
    // opening starts at the list.
    render(<App />);
    await screen.findByText("68%");
    await tray("tray://settings");

    fireEvent.pointerLeave(screen.getByTestId("dock-panel"));
    await settle(300);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("leaving");
    expect(screen.getByTestId("settings-sheet")).toBeDefined();

    await settle(200);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("closed");

    await open();
    expect(screen.queryByTestId("settings-sheet")).toBeNull();
    expect(screen.getByTestId("panel")).toBeDefined();
  });

  it("stays open while a sign-in is waiting on the browser, and no longer once it has answered", async () => {
    // The pointer is in the browser meanwhile; collapsing would lose the result.
    render(<App />);
    await screen.findByText("68%");
    await open();
    act(() => {
      useAdding.setState({ phase: "waiting" });
    });

    fireEvent.pointerLeave(screen.getByTestId("dock-panel"));
    await settle(500);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("open");
    expect(screen.getByTestId("add-sheet")).toBeDefined();

    act(() => {
      useAdding.setState({ phase: "added", account: ACTIVE });
    });
    fireEvent.pointerLeave(screen.getByTestId("dock-panel"));
    await settle(300);
    await settle(200);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("closed");
    expect(useAdding.getState().phase).toBe("idle");
  });

  it("stays open while a removal or its report is showing", async () => {
    // The report says what happened to Codex's sign-in when the account in use was removed.
    render(<App />);
    await screen.findByText("68%");
    await tray("tray://settings");
    act(() => {
      useAccounts.setState({ removal: { phase: "orphaned", name: "Lea" } });
    });

    fireEvent.pointerLeave(screen.getByTestId("dock-panel"));
    await settle(500);

    expect(screen.getByTestId("dock").dataset["stage"]).toBe("open");
  });

  it("stays open while the continuation instruction is being typed", async () => {
    // A focused text field pins the panel; blurring it releases the pin.
    render(<App />);
    await screen.findByText("68%");
    await open();
    fireEvent.click(screen.getByRole("button", { name: "Automatic continuation" }));
    fireEvent.click(screen.getByRole("button", { name: /More options/ }));
    const field = screen.getByLabelText("Instruction");

    fireEvent.focus(field);
    fireEvent.pointerLeave(screen.getByTestId("dock-panel"));
    await settle(500);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("open");

    fireEvent.blur(field);
    fireEvent.pointerLeave(screen.getByTestId("dock-panel"));
    await settle(300);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("leaving");
  });

  it("re-reads the account list when Rust says the scheduler changed the active account", async () => {
    // The list is the only thing that says who is active; the plan is never read for it.
    render(<App />);
    await screen.findByText("68%");
    const before = calls("list_accounts").length;

    await emit("accounts://changed", null);

    expect(calls("list_accounts").length).toBe(before + 1);
  });

  it("draws the bar where the drag settled", async () => {
    // Rust places the hover target at the settled offset, so the bar must be drawn there too.
    render(<App />);
    const bar = await screen.findByTestId("edge-bar");
    bar.setPointerCapture = vi.fn();
    bar.releasePointerCapture = vi.fn();
    bar.hasPointerCapture = vi.fn(() => true);

    fireEvent.pointerDown(bar, { pointerId: 1, button: 0, screenX: 100, screenY: 100 });
    fireEvent.pointerMove(bar, { pointerId: 1, screenX: 100, screenY: 220 });
    fireEvent.pointerUp(bar, { pointerId: 1 });
    await settle(0);

    expect(screen.getByTestId("dock").style.getPropertyValue("--tg-dock-offset")).toBe("120px");
  });

  it("opens the panel when the tray asks to show Toglet", async () => {
    // The bar is always visible, so "show" has to mean the panel.
    render(<App />);
    await screen.findByText("68%");
    expect(screen.queryByTestId("panel")).toBeNull();

    await tray("tray://show");

    expect(screen.getByTestId("panel")).toBeDefined();
  });

  it("opens the panel with the settings sheet when the tray asks for settings", async () => {
    // A settings sheet inside a closed panel is invisible.
    render(<App />);
    await screen.findByText("68%");

    await tray("tray://settings");

    expect(screen.getByTestId("panel")).toBeDefined();
    expect(screen.getByText("Settings")).toBeDefined();
  });

  it("says the current account is not known when accounts exist but none is active", async () => {
    // An added account is not the one Codex uses until switched to.
    reply({
      list_accounts: () =>
        Promise.resolve([
          { ...ACTIVE, status: "ready", isActive: false },
          { ...OTHER, isActive: false },
        ]),
    });
    render(<App />);
    // The bar's one control says what the state is while there is nothing to draw.
    await screen.findByTitle(/using none of these accounts/);
    await open();

    expect(screen.getAllByText(/No current account is known/)).toHaveLength(1);
    expect(screen.queryByText("Reading quota…")).toBeNull();
    const summaries = invoke.mock.calls
      .filter((call) => call[0] === "set_tray_summary")
      .map((call) => JSON.stringify(call[1]));
    expect(summaries.some((one) => one.includes("No current account"))).toBe(true);
  });

  it("shows every account once the panel is open", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(screen.getAllByTestId("account-row")).toHaveLength(2);
    expect(screen.getByText("Team")).toBeDefined();
    expect(screen.getByText("Personal")).toBeDefined();
  });

  it("marks exactly the account Rust called active", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(screen.getAllByText("Active")).toHaveLength(1);
  });

  it("reads every account's quota when the panel opens", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    await waitFor(() => {
      const asked = invoke.mock.calls
        .filter(([command]) => command === "refresh_quota")
        .map(([, args]) => (args as { accountId: string }).accountId);
      expect(new Set(asked)).toStrictEqual(new Set(["acct-1", "acct-2"]));
    });
  });

  it("opens the panel with the add sheet from the bar's own add button", async () => {
    reply({ list_accounts: () => Promise.resolve([]) });

    render(<App />);
    fireEvent.click(await screen.findByTestId("bar-add"));
    await settle(200);

    expect(screen.getByTestId("panel")).toBeDefined();
    expect(screen.getByTestId("add-sheet")).toBeDefined();
  });

  it("does not call Codex unmanageable because nobody is signed in to it", async () => {
    // "No importable account" describes the sign-in, not the installation.
    reply({
      detect_environment_command: () =>
        Promise.resolve({
          checks: [
            { id: "codexCommand", status: "passed", code: null, action: "none", detail: null },
            {
              id: "authState",
              status: "passed",
              code: null,
              action: "none",
              detail: "not_signed_in",
            },
            {
              id: "importableAccount",
              status: "failed",
              code: "auth_expired",
              action: "re_login",
              detail: null,
            },
          ],
        }),
      list_accounts: () => Promise.resolve([{ ...ACTIVE, status: "ready", isActive: false }]),
    });

    render(<App />);
    await screen.findByTestId("bar-pick");
    await open();

    expect(screen.queryByText(/cannot be managed/)).toBeNull();
    expect(screen.queryByTitle(/cannot be managed/)).toBeNull();
    expect(screen.getByText(/No current account is known/)).toBeDefined();
  });

  it("does call Codex unmanageable when the installation itself failed a check", async () => {
    reply({
      detect_environment_command: () =>
        Promise.resolve({
          checks: [
            {
              id: "codexCommand",
              status: "failed",
              code: "runtime_not_found",
              action: "install_runtime",
              detail: null,
            },
          ],
        }),
    });

    render(<App />);
    await screen.findByTitle(/cannot be managed/);
    await open();

    expect(screen.getByText(/Codex cannot be managed on this machine/)).toBeDefined();
  });

  it("opens the panel from the bar's pick button when no account is current", async () => {
    reply({
      list_accounts: () =>
        Promise.resolve([
          { ...ACTIVE, status: "ready", isActive: false },
          { ...OTHER, isActive: false },
        ]),
    });

    render(<App />);
    fireEvent.click(await screen.findByTestId("bar-pick"));
    await settle(200);

    expect(screen.getByTestId("panel")).toBeDefined();
    expect(screen.queryByTestId("add-sheet")).toBeNull();
    expect(screen.getByText("Team")).toBeDefined();
  });

  it("does not re-read a fresh quota each time the panel opens", async () => {
    // Only readings missing, failed or older than two minutes are re-read.
    render(<App />);
    await screen.findByText("68%");
    await open();
    await waitFor(() => {
      expect(calls("refresh_quota")).toHaveLength(2);
    });
    await close();

    await open();
    await settle(50);

    expect(calls("refresh_quota")).toHaveLength(2);
  });

  it("says the list could not be read rather than showing it as empty", async () => {
    reply({ list_accounts: () => Promise.reject(new Error("bridge unavailable")) });

    render(<App />);
    await open();

    expect(screen.getAllByText(/could not read its own state/).length).toBeGreaterThan(0);
    expect(screen.queryByText("No accounts yet")).toBeNull();
  });

  it("reports an unrepaired switch ahead of anything else", async () => {
    reply({ startup_recovery: () => Promise.resolve("failed") });

    render(<App />);
    await open();

    expect(await screen.findByText(/could not be repaired/)).toBeDefined();
  });

  it("says the numbers are cached once a reading has aged out", async () => {
    reply({
      refresh_quota: () =>
        Promise.resolve({ ...(quota("acct-1") as object), fetchedAt: NOW - 3600 }),
    });

    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(await screen.findByText(/Showing cached values/)).toBeDefined();
  });

  it("offers to switch to a row that is not the active account", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    fireEvent.click(screen.getByLabelText("Switch to Personal"));
    await act(async () => {
      await Promise.resolve();
    });

    expect(await screen.findByText("Switch to Personal?")).toBeDefined();
  });

  it("does not offer to switch to the account already in use", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(screen.queryByLabelText("Switch to Team")).toBeNull();
  });

  it("re-reads the account list after a switch rather than assuming who is active", async () => {
    render(<App />);
    await screen.findByText("68%");
    await open();

    fireEvent.click(screen.getByLabelText("Switch to Personal"));
    await act(async () => {
      await Promise.resolve();
    });
    const before = invoke.mock.calls.filter(([command]) => command === "list_accounts").length;

    fireEvent.click(screen.getByText("Switch account"));
    await act(async () => {
      vi.advanceTimersByTime(50);
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    const after = invoke.mock.calls.filter(([command]) => command === "list_accounts").length;
    expect(after).toBeGreaterThan(before);
  });

  it("shows the first-run panel when there is genuinely no account", async () => {
    reply({ list_accounts: () => Promise.resolve([]) });

    render(<App />);
    await open();

    expect(await screen.findByText("No accounts yet")).toBeDefined();
    expect(screen.getByText(/No account is being managed yet/)).toBeDefined();
  });

  it("draws against the edge the stored settings report", async () => {
    stored = { ...SETTINGS, dockEdge: "left" };

    render(<App />);
    await screen.findByText("68%");

    expect(screen.getByTestId("edge-bar").className).toContain("left");
  });

  it("mirrors the docked surface in one place, so the two cannot cancel out", async () => {
    // Mirroring twice (dock and panel) would put the panel against the screen edge; only the
    // dock mirrors.
    stored = { ...SETTINGS, dockEdge: "left" };

    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(screen.getByTestId("dock").className).toContain("left");
    expect(screen.getByTestId("panel").className).not.toMatch(/left|right/);
  });

  it("relabels everything already on screen when the language changes", async () => {
    // No restart or reload; surfaces already drawn are relabelled.
    render(<App />);
    await screen.findByText("68%");
    await open();
    expect(screen.getByRole("button", { name: "Refresh quota" })).toBeDefined();
    expect(screen.getByText("2 accounts")).toBeDefined();

    await act(async () => {
      await useSettings.getState().update({ language: "zh" });
    });

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "刷新额度" })).toBeDefined();
    });
    // Visible copy and the labels assistive technology reads, both.
    expect(screen.getByText("2 个账户")).toBeDefined();
    expect(screen.queryByRole("button", { name: "Refresh quota" })).toBeNull();
    expect(document.documentElement.getAttribute("lang")).toBe("zh");
  });

  it("relabels the tray menu too, which no re-render can reach", async () => {
    // The OS draws the tray menu, so it has to be sent the new labels.
    render(<App />);
    await screen.findByText("68%");

    await act(async () => {
      await useSettings.getState().update({ language: "zh" });
    });

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("set_tray_labels", {
        labels: {
          show: "显示 Toglet",
          hide: "隐藏 Toglet",
          refresh: "刷新额度",
          primary: "移到主显示器",
          settings: "设置…",
          quit: "退出 Toglet",
        },
      });
    });
  });

  it("follows the edge to its new side the moment the setting changes", async () => {
    // The side comes from the settings Rust returns on save, so the bar follows the window.
    render(<App />);
    await screen.findByText("68%");
    expect(screen.getByTestId("edge-bar").className).toContain("right");

    await act(async () => {
      await useSettings.getState().update({ dockEdge: "left" });
    });

    expect(screen.getByTestId("edge-bar").className).toContain("left");
  });

  it("swaps the bar for the ring form the moment that setting changes", async () => {
    // Read from stored settings, so the bar and Rust's hover target agree at once.
    render(<App />);
    await screen.findByText("68%");
    expect(screen.queryByTestId("ring-bar")).toBeNull();

    await act(async () => {
      await useSettings.getState().update({ dockShape: "ring" });
    });

    expect(screen.queryByTestId("edge-bar")).toBeNull();
    expect(screen.getByTestId("ring-bar").className).toContain("right");
    expect(screen.getByTestId("dock").className).toContain("ring");
  });
});
