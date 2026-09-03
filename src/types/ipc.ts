/**
 * The shapes that cross the Tauri boundary.
 *
 * Each one mirrors a `Serialize` type in `src-tauri/src/commands/views.rs` or
 * `src-tauri/src/codex_home/detect.rs`, field for field. Nothing may be added here that the
 * Rust side does not send: an invented field would be a value the interface displays and the
 * backend never vouched for.
 *
 * The unions below are the exact strings the Rust enums serialise to. Widening one of them
 * silently is how an unknown state ends up rendered as a known one, so they stay closed and
 * the code that reads them handles the unmatched case.
 */

/** The commands the IPC layer is allowed to call. There is no general-purpose escape hatch. */
export type CommandName =
  | "detect_environment_command"
  | "list_accounts"
  | "startup_recovery"
  | "refresh_quota"
  | "remove_account"
  | "set_dock_expansion"
  | "move_dock"
  | "end_drag"
  | "inspect_clients"
  | "switch_account"
  | "read_settings"
  | "update_settings"
  | "start_login"
  | "finish_login"
  | "cancel_login"
  | "set_tray_summary"
  | "set_tray_labels";

/**
 * A call that did not produce a value.
 *
 * The rejection itself is deliberately discarded rather than carried: it can be an
 * operating-system message, and those hold absolute paths, which must not reach the frontend
 * or its logs. The command name is what the interface needs in order to say which step
 * failed.
 */
export interface IpcFailure {
  readonly command: CommandName;
}

export type IpcResult<T> =
  { readonly ok: true; readonly value: T } | { readonly ok: false; readonly failure: IpcFailure };

/** `AccountStatus` - nine states, no boolean combinations. */
export type AccountStatus =
  | "ready"
  | "active"
  | "refreshing"
  | "stale"
  | "offline"
  | "reauth_required"
  | "unsupported"
  | "switching"
  | "error";

/** `UserAction` - what the user can actually do about a failure. */
export type UserAction =
  | "retry"
  | "re_login"
  | "install_runtime"
  | "update_runtime"
  | "close_codex_client"
  | "fix_config_manually"
  | "restore_from_backup"
  | "check_network"
  | "unlock_credential_store"
  | "wait_for_switch"
  | "fix_permissions"
  | "resolve_external_change"
  | "none";

/** One account, exactly as `AccountView` sends it. */
export interface AccountView {
  /** The random internal id. The fingerprint and the credential key never leave Rust. */
  readonly id: string;
  readonly displayName: string;
  /** Already masked on the Rust side. The full address does not exist on this side at all. */
  readonly maskedEmail: string | null;
  /** `null` means the plan is unknown - it is never a placeholder or a guess. */
  readonly planType: string | null;
  readonly status: AccountStatus;
  readonly isActive: boolean;
}

/** `RemovalView` - what removing an account did. */
export interface RemovalView {
  /** Whether the account left the list. */
  readonly removed: boolean;
  /** Whether Codex was signed out for it. Only ever true for the account that was in use. */
  readonly signedOut: boolean;
  /** False when the profile is gone but its entry in the credential store could not be deleted. */
  readonly credentialDeleted: boolean;
  /** What happened to Codex's sign-in when a sign-out failed. `null` when nothing was touched. */
  readonly rollback: RollbackReport | null;
  readonly error: ErrorView | null;
}

/** `CheckId` - the seven first-run checks, in the order the report returns them. */
export type CheckId =
  | "operatingSystem"
  | "codexCommand"
  | "appServerMethods"
  | "defaultCodexHome"
  | "configFile"
  | "authState"
  | "importableAccount";

/**
 * `CheckStatus`. `notApplicable` means the check could not be reached because something it
 * depends on failed - it is never a pass.
 */
export type CheckStatus = "passed" | "failed" | "notApplicable";

export interface EnvironmentCheck {
  readonly id: CheckId;
  readonly status: CheckStatus;
  /** A stable error code when the check failed, `null` otherwise. */
  readonly code: string | null;
  readonly action: UserAction;
  /** A short non-sensitive fact: an OS name, an auth mode, a plan. Never a path. */
  readonly detail: string | null;
}

export interface EnvironmentReport {
  readonly checks: readonly EnvironmentCheck[];
}

/**
 * What the start-up recovery did about a switch that was interrupted, or `null` when there was
 * nothing to recover - which is the ordinary case.
 */
export type RecoveryOutcome = "rolled_back" | "completed" | "failed";

/**
 * The quota windows Rust hands over.
 *
 * Only these two reach the interface: `QuotaView::from_snapshot` drops the windows Toglet has no
 * meaning for rather than showing a number nobody can act on.
 */
export type QuotaWindowKind = "five_hour" | "weekly";

/**
 * One quota window.
 *
 * A window that exists always carries both percentages, which is why neither is nullable. A
 * window the server did not return is simply absent from the list - that absence is the whole
 * representation of "not returned", and it is why nothing has to be rendered as `0`.
 */
export interface QuotaWindowView {
  readonly kind: QuotaWindowKind;
  readonly usedPercent: number;
  readonly remainingPercent: number;
  /** Unix seconds, absolute. Converted to local time only when it is displayed. */
  readonly resetsAt: number | null;
}

export interface QuotaView {
  readonly accountId: string;
  /** May be empty, or hold only one window. Both are honest answers. */
  readonly windows: readonly QuotaWindowView[];
  /** Unix seconds. Staleness is recomputed against it as the clock moves on. */
  readonly fetchedAt: number;
  readonly source: string;
  readonly stale: boolean;
  readonly lastErrorCode: string | null;
}

export type Theme = "system" | "dark" | "light";

/**
 * `Language` - the stored preference, which is not the same as the language on screen.
 *
 * `system` means the user has never chosen. Only the interface can turn it into a language,
 * because only it knows what the operating system asked the webview for; `resolveLanguage` in
 * `src/i18n` is where that happens, and this union is narrow enough that a value Rust starts
 * sending which the dictionaries do not cover will not compile there.
 */
export type LanguagePreference = "system" | "en" | "zh";

/**
 * The settings the interface may edit.
 *
 * Deliberately not the whole of `AppSettings`: `activeAccountId` is not a setting, and
 * `displayId` is a stored fact about where the window was rather than a choice. The settings
 * whose behaviour does not exist yet - launch at login, fullscreen avoidance, the diagnostics
 * folder, "stop managing Codex authentication" - are absent for the same reason a button that
 * does nothing is absent.
 */
export interface SettingsView {
  readonly dockEdge: "left" | "right";
  /** The bar's centre, in logical pixels below the work area's centre. Already clamped by Rust. */
  readonly verticalOffset: number;
  readonly alwaysOnTop: boolean;
  readonly activeRefreshSeconds: number;
  readonly inactiveRefreshSeconds: number;
  readonly reopenCodexAfterSwitch: boolean;
  readonly theme: Theme;
  readonly reduceMotion: boolean;
  readonly language: LanguagePreference;
}

/**
 * What the tray menu's own entries say.
 *
 * Mirrors `TrayLabels` in `src-tauri/src/window/tray.rs`. The wording travels rather than being
 * held on both sides: the dictionary lives here, and a second copy of it in Rust is a copy that
 * can fall out of step with the panel the menu sits beside.
 */
export interface TrayLabels {
  readonly show: string;
  readonly refresh: string;
  readonly primary: string;
  readonly settings: string;
  readonly quit: string;
}

/** A change to some settings. An absent field is left alone. */
export type SettingsPatch = Partial<SettingsView>;

/** What the running Codex clients mean for a switch (`ClientVerdict`). */
export type ClientVerdict = "clear" | "desktop_only" | "blocked" | "unknown";

/** What happened to the previous authentication when a switch failed (`RollbackReport`). */
export type RollbackReport = "not_needed" | "restored" | "restored_unverified" | "failed";

/** What happened to the Codex client afterwards (`ClientOutcome`). */
export type ClientOutcome =
  "nothing_was_running" | "reopened" | "closed_not_reopened" | "closed_by_choice";

/** A failure, in the only form the interface needs. The error's detail stays in Rust. */
export interface ErrorView {
  readonly code: string;
  readonly phase: string;
  readonly retryable: boolean;
  readonly action: UserAction;
}

/**
 * The result of a switch.
 *
 * Deliberately two-part: `switched` says whether the account changed, and
 * `clientUpToDate` says whether Codex is actually running it. A switch that worked while the
 * client still holds the old credentials is **not** a plain success, and must not be shown as
 * one.
 */
export interface SwitchView {
  readonly switched: boolean;
  /** 0 to 4, and only ever the steps that actually finished. */
  readonly progress: number;
  readonly clientUpToDate: boolean;
  readonly clients: ClientVerdict;
  readonly rollback: RollbackReport | null;
  readonly error: ErrorView | null;
  /** The user has to put the previous credentials back by hand. The path is not sent. */
  readonly manualRecoveryRequired: boolean;
  readonly clientOutcome: ClientOutcome | null;
}

/**
 * What happened when a sign-in produced an account.
 *
 * `added: false` is **not an error**. The sign-in succeeded; the browser reused a ChatGPT session
 * that was already open, so the account it produced is one Toglet already had. The sign-in has no
 * way to ask for the account chooser - `account/login/start` takes no parameters beyond the
 * type - so this is a case the interface has to explain rather than prevent.
 */
export interface AddedAccountView {
  readonly account: AccountView;
  readonly added: boolean;
}
