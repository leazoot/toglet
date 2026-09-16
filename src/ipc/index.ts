/**
 * The only module that talks to Rust: every `invoke` and `listen` lives here (enforced by ESLint).
 * It wraps calls and narrows types; no business decisions, no formatting.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";

import type {
  AccountView,
  AddedAccountView,
  AutoRunView,
  BindRequest,
  ClientVerdict,
  CommandName,
  EnvironmentReport,
  IpcError,
  IpcResult,
  NotifyOutcome,
  NotifyView,
  QuotaView,
  RemoteDraft,
  RemoteView,
  RemovalView,
  RecoveryOutcome,
  ResetCreditOutcomeView,
  SaveChannelRequest,
  SettingsPatch,
  SettingsView,
  SwitchView,
  ThreadListView,
  TrayLabels,
} from "../types/ipc";

/**
 * A rejection is either the bridge failing (possibly an OS message holding a path) or a Rust
 * `ErrorView`. Only `code` and `retryable` are copied out by name, so nothing else can leak.
 * Success payloads are not re-validated: both sides ship in one binary and Rust tests the shapes.
 */
async function call<T>(
  command: CommandName,
  args?: Record<string, unknown>,
): Promise<IpcResult<T>> {
  try {
    return { ok: true, value: await invoke<T>(command, args) };
  } catch (rejection) {
    return { ok: false, failure: { command, error: reportedError(rejection) } };
  }
}

/** The structured error Rust sent, or `null` if this rejection is not one. */
function reportedError(rejection: unknown): IpcError | null {
  if (typeof rejection !== "object" || rejection === null) {
    return null;
  }
  const { code, retryable } = rejection as Record<string, unknown>;
  if (typeof code !== "string" || typeof retryable !== "boolean") {
    return null;
  }
  return { code, retryable };
}

/** Runs the seven first-run checks. Infallible on the Rust side. */
export function detectEnvironment(): Promise<IpcResult<EnvironmentReport>> {
  return call<EnvironmentReport>("detect_environment_command");
}

/** The stored accounts, already masked, with the active one flagged by Rust rather than here. */
export function listAccounts(): Promise<IpcResult<readonly AccountView[]>> {
  return call<readonly AccountView[]>("list_accounts");
}

/** What start-up recovery did about an interrupted switch; `null` when there was nothing to do. */
export function startupRecovery(): Promise<IpcResult<RecoveryOutcome | null>> {
  return call<RecoveryOutcome | null>("startup_recovery");
}

/**
 * Reads one account's quota. `now` is passed in so one clock both dates the reading and judges
 * staleness. Never changes the default authentication.
 */
export function refreshQuota(accountId: string, nowSeconds: number): Promise<IpcResult<QuotaView>> {
  return call<QuotaView>("refresh_quota", { accountId, now: nowSeconds });
}

/**
 * Redeems one reset credit for an account. Irreversible, so the confirmation must have happened
 * before this is called.
 */
export function consumeResetCredit(
  accountId: string,
  nowSeconds: number,
): Promise<IpcResult<ResetCreditOutcomeView>> {
  // `now` identifies this attempt on the Rust side, so retrying it cannot spend a second credit.
  return call<ResetCreditOutcomeView>("consume_reset_credit", { accountId, now: nowSeconds });
}

/**
 * Tells Rust whether the panel is open, so it can decide click-through for the transparent strip
 * (a window passing the pointer through gets no pointer events to decide with).
 */
export function setDockExpansion(expanded: boolean): Promise<IpcResult<null>> {
  return call<null>("set_dock_expansion", { expanded });
}

/**
 * Moves the window sideways by a drag increment, in logical pixels. Vertical travel stays in the
 * interface until `endDrag`. Nothing is stored until then.
 */
export function moveDock(dx: number): Promise<IpcResult<null>> {
  return call<null>("move_dock", { dx });
}

/**
 * Ends a drag; Rust picks and stores the monitor, edge and height. `lift` is the total vertical
 * travel in logical pixels, positive downward. Returns the settings with the clamped offset.
 */
export function endDrag(lift: number): Promise<IpcResult<SettingsView>> {
  return call<SettingsView>("end_drag", { lift });
}

/**
 * Removes an account and its saved sign-in. The account in use is refused unless `signOut` is
 * set, in which case Rust signs Codex out first with the switch's checks and rollback.
 */
export function removeAccount(
  accountId: string,
  signOut: boolean,
  nowSeconds: number,
): Promise<IpcResult<RemovalView>> {
  return call<RemovalView>("remove_account", { accountId, signOut, now: nowSeconds });
}

/** What running Codex clients mean for a switch, asked before offering the confirmation. */
export function inspectClients(): Promise<IpcResult<ClientVerdict>> {
  return call<ClientVerdict>("inspect_clients");
}

/**
 * Switches to `accountId`, resolving when the switch has finished. Steps arrive through
 * {@link onSwitchStep}; only the returned view decides whether the interface may say "switched".
 */
export function switchAccount(
  accountId: string,
  nowSeconds: number,
): Promise<IpcResult<SwitchView>> {
  return call<SwitchView>("switch_account", { accountId, now: nowSeconds });
}

/**
 * Subscribes to completed switch steps (1 to 4). Rust emits a step only after recording it, so
 * the stream cannot run ahead of the work. Resolves to the unsubscribe function.
 */
export function onSwitchStep(handler: (step: number) => void): Promise<() => void> {
  return listen<number>("switch://step", (event) => {
    handler(event.payload);
  });
}

export function readSettings(): Promise<IpcResult<SettingsView>> {
  return call<SettingsView>("read_settings");
}

/**
 * Changes some settings and returns them as stored. Show the returned view, not the request:
 * Rust corrects out-of-range values.
 */
export function updateSettings(patch: SettingsPatch): Promise<IpcResult<SettingsView>> {
  return call<SettingsView>("update_settings", { patch });
}

/**
 * Starts a sign-in and opens the browser. The authorisation URL (PKCE challenge, OAuth state)
 * never crosses this boundary.
 */
export function startLogin(): Promise<IpcResult<null>> {
  return call<null>("start_login");
}

/**
 * Waits for the browser, then verifies and stores the account. No name is sent: Rust uses the
 * ChatGPT name or the address's local part.
 */
export function finishLogin(nowSeconds: number): Promise<IpcResult<AddedAccountView>> {
  return call<AddedAccountView>("finish_login", { displayName: null, now: nowSeconds });
}

/** Abandons a sign-in the user gave up on, and cleans up after it. */
export function cancelLogin(): Promise<IpcResult<null>> {
  return call<null>("cancel_login");
}

/** Sets the tray summary line. Formatted here so the tray and the panel cannot disagree. */
export function setTraySummary(summary: string): Promise<IpcResult<null>> {
  return call<null>("set_tray_summary", { summary });
}

/** Relabels the tray menu in the interface's language; the dictionary lives on this side. */
export function setTrayLabels(labels: TrayLabels): Promise<IpcResult<null>> {
  return call<null>("set_tray_labels", { labels });
}

/** The tray asking to show Toglet; the bar is always visible, so this opens the panel. */
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

/** The scheduler recorded a new active account. No payload: handlers re-read the list. */
export function onAccountsChanged(handler: () => void): Promise<() => void> {
  return listen("accounts://changed", () => {
    handler();
  });
}

/** The automatic-continuation plan as last written. Never waits on the scheduler. */
export function readAutoRun(): Promise<IpcResult<AutoRunView>> {
  return call<AutoRunView>("read_autorun");
}

/**
 * The sessions the app server can see. Starts an app server, so it takes seconds. Paths stay in
 * Rust keyed by thread id, which is why a binding sends only the id.
 */
export function listThreads(): Promise<IpcResult<ThreadListView>> {
  return call<ThreadListView>("list_threads");
}

/**
 * Binds the plan in one step, sent only after confirmation. The instruction is user text bound
 * for a `turn/start` payload and crosses the boundary only here.
 */
export function bindAutoRun(request: BindRequest): Promise<IpcResult<null>> {
  return call<null>("bind_autorun", { request });
}

/** Turns the plan on, or off - off is a cancel that keeps the binding for next time. */
export function setAutoRunEnabled(enabled: boolean): Promise<IpcResult<null>> {
  return call<null>("set_autorun_enabled", { enabled });
}

/** Pauses the plan: nothing more is scheduled until it is continued. The binding is kept. */
export function pauseAutoRun(): Promise<IpcResult<null>> {
  return call<null>("pause_autorun");
}

/** Continues a paused, completed, stopped or attention-needing plan from its check step. */
export function resumeAutoRun(): Promise<IpcResult<null>> {
  return call<null>("resume_autorun");
}

/** Cancels the plan. Same as turning it off: the binding is kept, nothing is scheduled. */
export function cancelAutoRun(): Promise<IpcResult<null>> {
  return call<null>("cancel_autorun");
}

/** Subscribes to the plan as Rust writes it. Resolves to the unsubscribe function. */
export function onAutoRunState(handler: (view: AutoRunView) => void): Promise<() => void> {
  return listen<AutoRunView>("autorun://state", (event) => {
    handler(event.payload);
  });
}

/**
 * Posts a system notification, asking permission on first use. Resolves `false` when refused or
 * unsupported. Title and body contain only dictionary copy and account display names.
 */
export async function notify(title: string, body: string): Promise<boolean> {
  try {
    let granted = await isPermissionGranted();
    if (!granted) {
      granted = (await requestPermission()) === "granted";
    }
    if (!granted) {
      return false;
    }
    sendNotification({ title, body });
    return true;
  } catch {
    // No plugin or no notification centre; the interface already shows the state.
    return false;
  }
}

/** Phone remote state. Never carries the shared secret. */
export function readRemote(): Promise<IpcResult<RemoteView>> {
  return call<RemoteView>("read_remote");
}

/**
 * Turns remote control on or off, pairing it when new details were typed. The secret crosses
 * only here, on its way to the credential store; omitted details keep the stored ones.
 */
export function saveRemote(draft: RemoteDraft): Promise<IpcResult<RemoteView>> {
  return call<RemoteView>("save_remote", { draft });
}

/** Forgets the pairing: the stored details, the host, and the replay state. */
export function forgetRemote(): Promise<IpcResult<RemoteView>> {
  return call<RemoteView>("forget_remote");
}

/** The shortest secret Rust will accept, so the page can say so before the user presses save. */
export function remoteSecretMinimum(): Promise<IpcResult<number>> {
  return call<number>("remote_secret_minimum");
}

/** The notification channels as they are stored. Never carries what a channel needs to send. */
export function readNotifyChannels(): Promise<IpcResult<NotifyView>> {
  return call<NotifyView>("read_notify_channels");
}

/**
 * Adds or changes a channel and returns the updated list. Connection details cross only here, on
 * their way to the credential store; omitting them keeps the stored ones.
 */
export function saveNotifyChannel(request: SaveChannelRequest): Promise<IpcResult<NotifyView>> {
  return call<NotifyView>("save_notify_channel", { request });
}

/** Removes a channel and the details stored for it. */
export function removeNotifyChannel(channelId: string): Promise<IpcResult<NotifyView>> {
  return call<NotifyView>("remove_notify_channel", { channelId });
}

/**
 * Sends a message through the configured channels, or one channel (the test button, which also
 * sends to a disabled channel). Named `deliver…` to avoid clashing with the plugin's
 * `sendNotification`. May take seconds; callers need not wait.
 */
export function deliverNotification(
  title: string,
  body: string,
  channelId?: string,
): Promise<IpcResult<readonly NotifyOutcome[]>> {
  return call<readonly NotifyOutcome[]>("send_notification", {
    title,
    body,
    channelId: channelId ?? null,
  });
}
