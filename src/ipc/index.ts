/**
 * The only place in the application that talks to Rust.
 *
 * Every `invoke` and every `listen` lives here, and ESLint enforces it: importing
 * `@tauri-apps/api` anywhere else under src/ is an error. The layer wraps calls and narrows
 * return types - it makes no business decisions and formats nothing.
 *
 * `import_current_account` and `switch_account` change the active account and arrive with the
 * screens that trigger them; wrapping them now would add call sites nothing reaches.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type {
  AccountView,
  AddedAccountView,
  ClientVerdict,
  CommandName,
  EnvironmentReport,
  IpcResult,
  QuotaView,
  RemovalView,
  RecoveryOutcome,
  SettingsPatch,
  SettingsView,
  SwitchView,
  TrayLabels,
} from "../types/ipc";

/**
 * A rejection here means either the bridge did not work - the shell is not running under Tauri,
 * or the command is not registered - or the command itself returned `Err`. The distinction is
 * not made here: either way nothing was read, and the caller has no different action to take.
 *
 * The rejection value is dropped rather than carried. It can be an operating-system message,
 * and those hold absolute paths.
 *
 * The payload is not re-validated at runtime. Both sides ship in the same binary and the Rust
 * tests assert the serialised shapes; a second schema here would be a copy of the truth that
 * can drift away from it.
 */
async function call<T>(
  command: CommandName,
  args?: Record<string, unknown>,
): Promise<IpcResult<T>> {
  try {
    return { ok: true, value: await invoke<T>(command, args) };
  } catch {
    return { ok: false, failure: { command } };
  }
}

/** Runs the seven first-run checks. Infallible on the Rust side. */
export function detectEnvironment(): Promise<IpcResult<EnvironmentReport>> {
  return call<EnvironmentReport>("detect_environment_command");
}

/** The stored accounts, already masked, with the active one flagged by Rust rather than here. */
export function listAccounts(): Promise<IpcResult<readonly AccountView[]>> {
  return call<readonly AccountView[]>("list_accounts");
}

/**
 * What the start-up recovery did about an interrupted switch. `null` is the ordinary answer:
 * there was nothing to recover.
 */
export function startupRecovery(): Promise<IpcResult<RecoveryOutcome | null>> {
  return call<RecoveryOutcome | null>("startup_recovery");
}

/**
 * Reads one account's quota.
 *
 * `now` is passed in rather than read inside Rust because the same clock has to date the reading
 * and decide when it has gone stale; two clocks would disagree across a suspend.
 *
 * This command cannot change the default authentication - the credentials are decrypted into a
 * directory that deletes itself.
 */
export function refreshQuota(accountId: string, nowSeconds: number): Promise<IpcResult<QuotaView>> {
  return call<QuotaView>("refresh_quota", { accountId, now: nowSeconds });
}

/**
 * Tells Rust whether the panel is open.
 *
 * The window never changes size. While the panel is open the whole window is surface; while it
 * is not, the transparent strip lets clicks through to the desktop and only the bar listens -
 * and that decision is Rust's, because a window that is letting the pointer through gets no
 * pointer events to decide with.
 */
export function setDockExpansion(expanded: boolean): Promise<IpcResult<null>> {
  return call<null>("set_dock_expansion", { expanded });
}

/**
 * Moves the window by a drag's latest increment, in logical pixels.
 *
 * Relative, so the interface never handles screen coordinates or scale factors. Nothing is
 * stored: an abandoned drag leaves the remembered place untouched.
 */
export function moveDock(dx: number, dy: number): Promise<IpcResult<null>> {
  return call<null>("move_dock", { dx, dy });
}

/** Ends a drag: Rust decides the monitor, the edge and the height, and remembers all three. */
/**
 * Answers with the settings as stored: the settled offset is what the stylesheet places the bar
 * from, and it is only known once Rust has clamped it to the monitor the drag ended on.
 */
/**
 * Removes an account and its saved sign-in.
 *
 * `signOut` is the explicit choice the account in use needs: Rust signs Codex out first, under
 * the switch's own checks and rollback, and refuses the account in use without it.
 */
export function removeAccount(
  accountId: string,
  signOut: boolean,
  nowSeconds: number,
): Promise<IpcResult<RemovalView>> {
  return call<RemovalView>("remove_account", { accountId, signOut, now: nowSeconds });
}

export function endDrag(): Promise<IpcResult<SettingsView>> {
  return call<SettingsView>("end_drag");
}

/**
 * What the running Codex clients mean for a switch, without starting one.
 *
 * Asked before the confirmation is offered, so the user is not asked to confirm something that
 * is about to be refused.
 */
export function inspectClients(): Promise<IpcResult<ClientVerdict>> {
  return call<ClientVerdict>("inspect_clients");
}

/**
 * Switches to `accountId`.
 *
 * Resolves only when the switch has finished, one way or the other. The steps that finish along
 * the way arrive through {@link onSwitchStep}; the returned view is the outcome, and it is the
 * only thing that decides whether the interface may say "switched".
 */
export function switchAccount(
  accountId: string,
  nowSeconds: number,
): Promise<IpcResult<SwitchView>> {
  return call<SwitchView>("switch_account", { accountId, now: nowSeconds });
}

/**
 * Subscribes to the steps a running switch actually completes.
 *
 * The payload is the step number, 1 to 4. Rust sends one only after the step is recorded, and it
 * refuses to record anything but the next step - so this stream cannot run ahead of the work.
 * Returns a promise for the unsubscribe function.
 */
export function onSwitchStep(handler: (step: number) => void): Promise<() => void> {
  return listen<number>("switch://step", (event) => {
    handler(event.payload);
  });
}

/** The settings as they are stored. */
export function readSettings(): Promise<IpcResult<SettingsView>> {
  return call<SettingsView>("read_settings");
}

/**
 * Changes some settings and returns them as they now are.
 *
 * The returned view is what the interface shows. It never assumes its own change took: a value
 * out of range is corrected on the Rust side, and showing the requested value instead of the
 * stored one would be showing a setting that is not in force.
 */
export function updateSettings(patch: SettingsPatch): Promise<IpcResult<SettingsView>> {
  return call<SettingsView>("update_settings", { patch });
}

/**
 * Starts a sign-in and opens the browser.
 *
 * Returns nothing on purpose: the authorisation URL carries the PKCE challenge and the OAuth
 * state, and Rust hands it to the browser itself. It never crosses this boundary.
 */
export function startLogin(): Promise<IpcResult<null>> {
  return call<null>("start_login");
}

/**
 * Waits for the browser, then verifies and stores the account. Resolves when it is over.
 *
 * No name is sent: the account is named after itself - the name it carries at ChatGPT, or the
 * local part of its address.
 */
export function finishLogin(nowSeconds: number): Promise<IpcResult<AddedAccountView>> {
  return call<AddedAccountView>("finish_login", { displayName: null, now: nowSeconds });
}

/** Abandons a sign-in the user gave up on, and cleans up after it. */
export function cancelLogin(): Promise<IpcResult<null>> {
  return call<null>("cancel_login");
}

/**
 * Puts the interface's own summary line into the tray menu.
 *
 * The text is formatted here rather than in Rust: this side already owns percentage rounding,
 * the compact reset form and the three-state rules, and formatting them again on the other side
 * would let the tray and the panel disagree about the same number.
 */
export function setTraySummary(summary: string): Promise<IpcResult<null>> {
  return call<null>("set_tray_summary", { summary });
}

/**
 * Relabels the tray menu in the language the interface is showing.
 *
 * Same reason as the summary: the copy dictionary is on this side. A menu reading `Show Toglet`
 * beside a panel reading `显示 Toglet` is the failure this avoids, and it is the kind that only
 * appears on the machine of someone who does not read English.
 */
export function setTrayLabels(labels: TrayLabels): Promise<IpcResult<null>> {
  return call<null>("set_tray_labels", { labels });
}

/** The tray asking the interface to show itself, which for a bar that is always on screen means
 *  opening the panel. */
export function onTrayShow(handler: () => void): Promise<() => void> {
  return listen("tray://show", () => {
    handler();
  });
}

/** The tray asking the interface to refresh. The tray cannot read quota itself. */
export function onTrayRefresh(handler: () => void): Promise<() => void> {
  return listen("tray://refresh", () => {
    handler();
  });
}

/** The tray asking the interface to open the settings sheet. */
export function onTraySettings(handler: () => void): Promise<() => void> {
  return listen("tray://settings", () => {
    handler();
  });
}
