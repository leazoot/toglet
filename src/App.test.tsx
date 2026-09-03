import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const listen = vi.hoisted(() =>
  vi.fn<(event: string, handler: () => void) => Promise<() => void>>(() =>
    Promise.resolve(() => undefined),
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
  verticalOffset: 0,
  alwaysOnTop: true,
  activeRefreshSeconds: 60,
  inactiveRefreshSeconds: 300,
  reopenCodexAfterSwitch: true,
  theme: "system",
  reduceMotion: false,
  // Pinned rather than left at `system`, so these assertions read the English dictionary whatever
  // locale the machine running them is set to.
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
      // Rust stores the change and answers with the settings as they now are, which is what the
      // interface then draws against. Nothing else reports the docked edge.
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

/** Opens the panel. The hover delay is a token, and no stylesheet is attached here, so the
 *  fallback applies - the timers are advanced rather than waited on. */
async function open(): Promise<void> {
  fireEvent.pointerEnter(screen.getByTestId("dock-bar"));
  await settle(200);
}

/**
 * Closes the panel: the 260ms grace, then the 160ms exit. Two advances rather than one, because
 * the exit timer is only set once React has rendered the close the first timer asked for.
 */
async function close(): Promise<void> {
  fireEvent.pointerLeave(screen.getByTestId("dock-bar"));
  await settle(300);
  await settle(200);
}

/** Calls what the interface registered for a tray event, as Rust would when the menu is used. */
async function tray(event: string): Promise<void> {
  const registered = listen.mock.calls.filter((call) => call[0] === event);
  const handler = registered[registered.length - 1]?.[1];
  await act(async () => {
    handler?.();
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
    // The dictionary in force is module state, so it outlives a store reset. A test that
    // switched to Chinese would otherwise leave every later assertion reading Chinese.
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
    // The window never changes size. What Rust needs is which state the pointer gate
    // is in; a height here would be a request to resize, and resizing is what flashed.
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
    // No measuring stage any more: the window is already its final size, because it always is.
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
    // The gap between them is the panel wrapper's own padding, and the bar is a second hover
    // target; leaving one for the other must not count as leaving.
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
    // The intent fires 120ms after the pointer returns and the exit ends 160ms after it began:
    // 40ms apart. A clock that also runs with real time could cross that gap on its own while
    // React renders, so from here the clock moves only when the test moves it.
    vi.useFakeTimers({ shouldAdvanceTime: false });

    fireEvent.pointerLeave(screen.getByTestId("dock-bar"));
    await settle(300);
    expect(screen.getByTestId("dock").dataset["stage"]).toBe("leaving");

    fireEvent.pointerEnter(screen.getByTestId("dock-bar"));
    // Two advances, so React acts on the intent before the clock reaches the end of the exit -
    // as it does for real.
    await settle(130);
    await settle(100);

    expect(screen.getByTestId("dock").dataset["stage"]).toBe("open");
    expect(screen.getByTestId("panel")).toBeDefined();
  });

  it("re-reads the account list after a sign-in that produced an account already held", async () => {
    // Rust may have claimed that account as the current one, and the list is the only thing
    // that says who is current.
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
    // The scrim is the panel's; the nub is outside the panel. Without its own copy the nub stayed
    // bright beside a dimmed edge and looked stuck on rather than part of the panel.
    render(<App />);
    await screen.findByTestId("edge-bar");
    await open();
    expect(screen.queryByTestId("nub-scrim")).toBeNull();

    await tray("tray://settings");

    expect(screen.getByTestId("nub-scrim")).toBeDefined();
  });

  it("draws the bar where the drag settled", async () => {
    // Rust stores the settled offset and places its hover target from it. The bar is drawn from
    // the same number, and it has to be the new one: drawn at the old offset, the bar sat where
    // the pointer was no longer let through and could not be hovered or dragged again.
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
    // The bar is always on screen, so "show" that only showed the window did nothing anyone
    // could see - which is how a working menu reads as a broken one.
    render(<App />);
    await screen.findByText("68%");
    expect(screen.queryByTestId("panel")).toBeNull();

    await tray("tray://show");

    expect(screen.getByTestId("panel")).toBeDefined();
  });

  it("opens the panel with the settings sheet when the tray asks for settings", async () => {
    // A settings sheet inside a closed panel is invisible. The sheet used to open without the
    // panel, so the menu entry appeared to do nothing.
    render(<App />);
    await screen.findByText("68%");

    await tray("tray://settings");

    expect(screen.getByTestId("panel")).toBeDefined();
    expect(screen.getByText("Settings")).toBeDefined();
  });

  it("says the current account is not known when accounts exist but none is active", async () => {
    // An account added by signing in is not the one Codex is using until it is switched to.
    // Saying "no account has been added" beside it, and "reading quota" under it forever, were
    // both false.
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
    // The design refreshes on a timer and on every expansion.
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
    // With no account yet the bar is the whole interface; its one control has to do both.
    reply({ list_accounts: () => Promise.resolve([]) });

    render(<App />);
    fireEvent.click(await screen.findByTestId("bar-add"));
    await settle(200);

    expect(screen.getByTestId("panel")).toBeDefined();
    expect(screen.getByTestId("add-sheet")).toBeDefined();
  });

  it("does not call Codex unmanageable because nobody is signed in to it", async () => {
    // After a sign-out through Toglet the start-up report says "no importable account", which is
    // a fact about the sign-in and not about the installation. The status line used to answer it
    // with "Codex cannot be managed on this machine".
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
    // The rows are the chooser, so the bar only has to get the panel open.
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
    // Every hover used to start an app server per account. Only a reading missing, failed, or
    // older than two minutes is asked for again.
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
    // There is nothing to do, and touching the authentication for nothing is worse than
    // doing nothing.
    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(screen.queryByLabelText("Switch to Team")).toBeNull();
  });

  it("re-reads the account list after a switch rather than assuming who is active", async () => {
    // `isActive` is written by Rust after verification. The interface copies it.
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
    // The bug this stands guard over: the dock reversed its row for the left edge and the panel
    // separately re-ordered itself, which put the panel against the screen edge with the bar
    // pushed inward and clipped. Whichever way the mirror is expressed, it belongs to the dock -
    // the panel's own contents read the same on both edges.
    stored = { ...SETTINGS, dockEdge: "left" };

    render(<App />);
    await screen.findByText("68%");
    await open();

    expect(screen.getByTestId("dock").className).toContain("left");
    expect(screen.getByTestId("panel").className).not.toMatch(/left|right/);
  });

  it("relabels everything already on screen when the language changes", async () => {
    // No restart, and no reload. The panel is open and a sheet is up while this
    // happens, so this is also what says the change reaches surfaces that are already drawn.
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
    // The menu is drawn by the operating system. It is the one surface that goes on saying
    // whatever it last said, so it has to be told - and a menu reading "Quit Toglet" beside a
    // panel reading 中文 only ever shows up on the machine of someone who cannot read it.
    render(<App />);
    await screen.findByText("68%");

    await act(async () => {
      await useSettings.getState().update({ language: "zh" });
    });

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("set_tray_labels", {
        labels: {
          show: "显示 Toglet",
          refresh: "刷新额度",
          primary: "移到主显示器",
          settings: "设置…",
          quit: "退出 Toglet",
        },
      });
    });
  });

  it("follows the edge to its new side the moment the setting changes", async () => {
    // The bug this stands guard over: the side used to come from a second command that was read
    // once and never again, so Rust moved the window to the other edge while the bar went on
    // drawing its hit buffer and its rounded corners against the side it had left.
    render(<App />);
    await screen.findByText("68%");
    expect(screen.getByTestId("edge-bar").className).toContain("right");

    await act(async () => {
      await useSettings.getState().update({ dockEdge: "left" });
    });

    expect(screen.getByTestId("edge-bar").className).toContain("left");
  });
});
