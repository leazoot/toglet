/**
 * Shapes that cross the Tauri boundary, mirroring the `Serialize` types in
 * `src-tauri/src/commands/views.rs` and `src-tauri/src/codex_home/detect.rs` field for field.
 * Unions are the exact strings the Rust enums serialise to and stay closed.
 */

/** The commands the IPC layer may call. There is no general-purpose escape hatch. */
export type CommandName =
  | "detect_environment_command"
  | "list_accounts"
  | "startup_recovery"
  | "refresh_quota"
  | "consume_reset_credit"
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
  | "set_tray_labels"
  | "read_autorun"
  | "list_threads"
  | "bind_autorun"
  | "set_autorun_enabled"
  | "pause_autorun"
  | "resume_autorun"
  | "cancel_autorun"
  | "read_notify_channels"
  | "save_notify_channel"
  | "remove_notify_channel"
  | "read_remote"
  | "save_remote"
  | "forget_remote"
  | "remote_secret_minimum"
  | "send_notification";

/**
 * A call that did not produce a value. The raw rejection is discarded: it can be an OS message
 * holding absolute paths.
 */
export interface IpcFailure {
  readonly command: CommandName;
  /** Rust's structured error, or `null` when the rejection was not one (e.g. bridge failure). */
  readonly error: IpcError | null;
}

/** The part of `ErrorView` the interface keeps. Never any detail: that can name a path. */
export interface IpcError {
  /** The stable code, as `ErrorCode::as_str` writes it - `login_canceled`, and so on. */
  readonly code: string;
  readonly retryable: boolean;
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
  | "rebind_session"
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

/** What start-up recovery did about an interrupted switch; `null` when there was nothing to do. */
export type RecoveryOutcome = "rolled_back" | "completed" | "failed";

/** `QuotaView::from_snapshot` drops every other window kind. */
export type QuotaWindowKind = "five_hour" | "weekly";

/**
 * One quota window. A window the server did not return is absent from the list, never a
 * zeroed entry.
 */
export interface QuotaWindowView {
  readonly kind: QuotaWindowKind;
  readonly usedPercent: number;
  readonly remainingPercent: number;
  /** Unix seconds, absolute. Converted to local time only when it is displayed. */
  readonly resetsAt: number | null;
}

/**
 * Reset credits, which clear the rate-limit windows when one is redeemed. Not the purchased
 * usage balance the server reports beside them, which Toglet does not interpret.
 */
export interface ResetCreditsView {
  /** The server's own count. Never the length of a detail list, which it may cap. */
  readonly availableCount: number;
  /** Unix seconds of the next expiry, when the server sent details that carry one. */
  readonly earliestExpiry: number | null;
}

/**
 * What redeeming a reset credit did. Only `reset` spent a credit and cleared the windows; every
 * other value is a refusal and is shown as one.
 */
export type ResetOutcome = "reset" | "nothingToReset" | "noCredit" | "alreadyRedeemed" | "unknown";

export interface ResetCreditOutcomeView {
  readonly outcome: ResetOutcome;
  readonly succeeded: boolean;
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
  /** `null` when the server never mentioned them: not reported, not "none held". */
  readonly resetCredits: ResetCreditsView | null;
}

export type Theme = "system" | "dark" | "light";

/** `Language`, the stored preference. `system` is resolved by `resolveLanguage` in src/i18n. */
export type LanguagePreference = "system" | "en" | "zh";

/** The collapsed surface (`DockShape`). Rust sizes the hover target from the same value. */
export type DockShape = "bar" | "ring";

/**
 * The settings the interface may edit: a subset of `AppSettings` that excludes
 * `activeAccountId`, `displayId` and settings with no behaviour yet.
 */
export interface SettingsView {
  readonly dockEdge: "left" | "right";
  readonly dockShape: DockShape;
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
 * The tray menu's entries, mirroring `TrayLabels` in `src-tauri/src/window/tray.rs`. The copy is
 * sent from here so the dictionary exists only on this side.
 */
export interface TrayLabels {
  readonly show: string;
  /** The first entry's other reading; Rust picks which one is on the menu. */
  readonly hide: string;
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
 * The result of a switch. `switched` says whether the account changed; `clientUpToDate` says
 * whether Codex is running it. Switched but not up to date is not a plain success.
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
 * What a sign-in produced. `added: false` is not an error: the browser reused a signed-in
 * ChatGPT session for an account Toglet already holds, which `account/login/start` cannot prevent.
 */
export interface AddedAccountView {
  readonly account: AccountView;
  readonly added: boolean;
}

/* ---------------------------------------------------------------- automatic continuation */

/** `autorun::machine::State`. The interface renders states; it never advances one. */
export type AutoRunState =
  | "disabled"
  | "armed"
  | "selecting"
  | "waiting_quota"
  | "verifying"
  | "switching"
  | "resuming"
  | "running"
  | "waiting_network"
  | "round_completed"
  | "needs_human"
  | "paused"
  | "stopped";

/** `ExecutionEnvironment`. Only the desktop app is supported. */
export type ExecutionEnvironment = "desktop";

/**
 * One session the app server can see (`ThreadView`). The project's folder name is all the
 * interface gets of where it lives: the path stays in Rust.
 */
export interface ThreadView {
  readonly threadId: string;
  readonly title: string | null;
  /**
   * One-line excerpt of the first message, to tell unnamed sessions apart. Listing only: the
   * plan never carries it.
   */
  readonly preview: string | null;
  readonly projectLabel: string | null;
  /** Unix seconds. */
  readonly updatedAt: number;
}

export interface ThreadListView {
  readonly threads: readonly ThreadView[];
  /** The server had more than one page; a session that is not here may still exist. */
  readonly truncated: boolean;
  /** The app server version that produced the list, for explaining an empty one. */
  readonly runtimeVersion: string | null;
}

/** The binding as the interface sees it (`BindingView`): the folder name, never the path. */
export interface BindingView {
  readonly executionEnvironment: ExecutionEnvironment;
  readonly projectLabel: string;
  readonly threadId: string;
  readonly threadTitle: string | null;
  readonly resumeInstruction: string;
  readonly boundAt: string;
}

/** `Participant`: an account in the plan, and its place in the priority order. */
export interface AutoRunParticipant {
  readonly accountId: string;
  readonly order: number;
}

/** `ResultKind` - what the last notable thing was. */
export type AutoRunResultKind = "resumed" | "switched" | "waited" | "failed" | "completed";

/** `LastResultRecord`. `code` is a stable code, never a message. */
export interface AutoRunLastResult {
  readonly kind: AutoRunResultKind;
  readonly accountId: string | null;
  readonly turnId: string | null;
  readonly code: string | null;
  readonly at: string;
}

/**
 * `AutoRunView`: the plan minus `projectPath`, which never leaves Rust. Times are Unix seconds;
 * `expectedAvailableAt: null` means unknown, never a guessed countdown.
 */
export interface AutoRunView {
  readonly enabled: boolean;
  readonly binding: BindingView | null;
  readonly participants: readonly AutoRunParticipant[];
  readonly state: AutoRunState;
  readonly generation: number;
  readonly executingAccountId: string | null;
  /** A stable code (`WaitReason::as_str`), or `null`. */
  readonly waitReason: string | null;
  readonly expectedAvailableAt: number | null;
  readonly nextCheckAt: number | null;
  readonly lastResult: AutoRunLastResult | null;
  readonly resumeCount: number;
  /** `null` means no limit. */
  readonly maxResumes: number | null;
  readonly deadline: number | null;
  readonly updatedAt: string;
}

/**
 * `BindRequest`. The thread must be one the last `list_threads` returned; Rust looks its path
 * up from that listing.
 */
export interface BindRequest {
  readonly threadId: string;
  readonly resumeInstruction: string;
  /** Account ids, in priority order. */
  readonly participants: readonly string[];
  readonly maxResumes: number | null;
  /** Unix seconds. */
  readonly deadline: number | null;
}

/* -------------------------------------------------------------------------------------------
 * Task notifications. Connection details (webhook address, device key, password) are sent once
 * and never read back; `NotifyChannelView` carries only a host name.
 * ------------------------------------------------------------------------------------------- */

/** The services a notification can be sent to. Mirrors `notify::ChannelKind`. */
export type NotifyChannelKind = "bark" | "wecom" | "telegram" | "webhook" | "email";

/** How a mail server expects the connection to be encrypted. There is no plaintext option. */
export type MailSecurity = "tls" | "startTls";

/** What the last attempt on a channel did. `code` is a stable Rust code, for translation here. */
export interface NotifyDelivery {
  /** Unix seconds. */
  readonly at: number;
  readonly ok: boolean;
  readonly code: string | null;
}

/** One channel, as the settings sheet shows it. */
export interface NotifyChannelView {
  readonly id: string;
  readonly kind: NotifyChannelKind;
  readonly label: string;
  readonly enabled: boolean;
  /** The host it talks to, or a masked recipient for e-mail. Never a path, key or token. */
  readonly hint: string;
  readonly lastDelivery: NotifyDelivery | null;
}

export interface NotifyView {
  readonly channels: readonly NotifyChannelView[];
  readonly maxChannels: number;
}

/** Connection details by service. Omitted `server` / `apiBase` default in Rust. */
export type NotifyConnectionInput =
  | { readonly kind: "bark"; readonly server?: string; readonly deviceKey: string }
  | { readonly kind: "wecom"; readonly webhook: string }
  | {
      readonly kind: "telegram";
      readonly apiBase?: string;
      readonly botToken: string;
      readonly chatId: string;
    }
  | { readonly kind: "webhook"; readonly url: string }
  | {
      readonly kind: "email";
      readonly host: string;
      readonly port: number;
      readonly security: MailSecurity;
      readonly username: string;
      readonly password: string;
      readonly from: string;
      readonly to: string;
    };

/**
 * Adds or changes a channel. No `id` means a new channel; no `connection` keeps the stored
 * details, which this side never holds.
 */
export interface SaveChannelRequest {
  readonly id?: string;
  readonly label: string;
  readonly enabled: boolean;
  readonly connection?: NotifyConnectionInput;
}

/** What became of the most recent remote command. Stable codes, never prose. */
export interface RemoteLastCommand {
  /** Unix seconds. */
  readonly at: number;
  /** One of `resume` / `pause` / `cancel` / `status`. */
  readonly action: string;
  /** `applied`, or one of the `remote_*` refusal codes. */
  readonly result: string;
}

/** Phone remote state. The shared secret is never read back. */
export interface RemoteView {
  readonly enabled: boolean;
  readonly paired: boolean;
  readonly bridgeHost: string;
  /** The full bridge address, offered for editing. */
  readonly bridgeEndpoint: string | null;
  readonly lastCommand: RemoteLastCommand | null;
}

/**
 * Turns remote control on or off, and pairs it. Leaving out `endpoint` and `secret` keeps the
 * stored ones.
 */
export interface RemoteDraft {
  readonly enabled: boolean;
  readonly endpoint?: string;
  readonly secret?: string;
}

/** What one channel did with one message. */
export interface NotifyOutcome {
  readonly channelId: string;
  readonly ok: boolean;
  readonly code: string | null;
}
