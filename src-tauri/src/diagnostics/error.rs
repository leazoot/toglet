//! Structured, cross-platform error type.
//!
//! Every error says which step failed, whether the current account is still safe to use, and
//! what the user can do next, as codes rather than prose so the frontend can localise it.

use crate::diagnostics::redact::redact;

/// Stable error identifier shared by both platforms.
///
/// [`ErrorCode::as_str`] selects the frontend's user-facing copy, so never rename a variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// Codex runtime is not installed.
    RuntimeNotInstalled,
    /// Capability handshake failed; the runtime is incompatible.
    RuntimeIncompatible,
    /// The user canceled login; the app server reports this and a failure both as `success: false`.
    LoginCanceled,
    /// Login did not complete in time.
    LoginTimeout,
    /// The account is already present; no second profile is created.
    AccountAlreadyExists,
    /// Network, DNS, TLS or proxy failure.
    NetworkUnavailable,
    /// `401` or the refresh token is no longer valid.
    AuthExpired,
    /// The app server exited abnormally.
    AppServerCrashed,
    /// The server did not return a quota window. Never rendered as `0`.
    QuotaWindowNotReturned,
    /// The default Codex home is missing or not writable.
    CodexHomeUnwritable,
    /// `config.toml` could not be parsed; no modification is attempted.
    ConfigSyntaxError,
    /// The auth file is invalid or was changed by another program.
    AuthFileConflict,
    /// A Codex client is still running.
    ClientRunning,
    /// Graceful shutdown timed out. The process is never force-killed.
    ClientShutdownTimeout,
    /// Post-switch identity did not match the target account.
    SwitchVerificationMismatch,
    /// Rollback itself failed; manual recovery is required.
    RollbackFailed,
    /// The stored window position is off-screen.
    DisplayUnavailable,
    /// The credential store is unavailable. Never downgraded to plaintext.
    CredentialStoreUnavailable,
    /// The app server stayed alive but never answered a request, e.g. after an illegal frame.
    AppServerUnresponsive,
    /// A switch is already running. Concurrent switches are rejected, not queued.
    SwitchInProgress,
    /// `config.toml` changed between read and write (`configVersionConflict`); nothing is written.
    ConfigConflict,
    /// The configuration layer holding the key is read-only (`configLayerReadonly`), as with
    /// managed configuration.
    ConfigLayerReadonly,
    /// The switch target is already the active account.
    AlreadyActive,
    /// Somebody signed in to Codex outside Toglet since the last synchronisation.
    ExternalAuthChange,
    /// The bound Codex session is not found, not loadable, or has a malformed id. Codex 0.153.4
    /// answers all three with `-32600` and no `data`, so this is decided by method, not message.
    ThreadUnavailable,
    /// A notification service answered and refused the message; not a network failure.
    NotificationRejected,
    /// The remote bridge answered and refused the exchange; not a network failure.
    RemoteBridgeRejected,
    /// Unexpected internal failure. Carries a redacted detail for diagnosis.
    Internal,
}

impl ErrorCode {
    /// Stable wire form. Must not change once shipped.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeNotInstalled => "runtime_not_installed",
            Self::RuntimeIncompatible => "runtime_incompatible",
            Self::LoginCanceled => "login_canceled",
            Self::LoginTimeout => "login_timeout",
            Self::AccountAlreadyExists => "account_already_exists",
            Self::NetworkUnavailable => "network_unavailable",
            Self::AuthExpired => "auth_expired",
            Self::AppServerCrashed => "app_server_crashed",
            Self::QuotaWindowNotReturned => "quota_window_not_returned",
            Self::CodexHomeUnwritable => "codex_home_unwritable",
            Self::ConfigSyntaxError => "config_syntax_error",
            Self::AuthFileConflict => "auth_file_conflict",
            Self::ClientRunning => "client_running",
            Self::ClientShutdownTimeout => "client_shutdown_timeout",
            Self::SwitchVerificationMismatch => "switch_verification_mismatch",
            Self::RollbackFailed => "rollback_failed",
            Self::DisplayUnavailable => "display_unavailable",
            Self::CredentialStoreUnavailable => "credential_store_unavailable",
            Self::AppServerUnresponsive => "app_server_unresponsive",
            Self::SwitchInProgress => "switch_in_progress",
            Self::ConfigConflict => "config_conflict",
            Self::ConfigLayerReadonly => "config_layer_readonly",
            Self::AlreadyActive => "already_active",
            Self::ExternalAuthChange => "external_auth_change",
            Self::ThreadUnavailable => "thread_unavailable",
            Self::NotificationRejected => "notification_rejected",
            Self::RemoteBridgeRejected => "remote_bridge_rejected",
            Self::Internal => "internal",
        }
    }

    /// The inverse of [`as_str`](Self::as_str), for a code read back from a file this
    /// build wrote. Unknown text is `None`, never a guess.
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "runtime_not_installed" => Self::RuntimeNotInstalled,
            "runtime_incompatible" => Self::RuntimeIncompatible,
            "login_canceled" => Self::LoginCanceled,
            "login_timeout" => Self::LoginTimeout,
            "account_already_exists" => Self::AccountAlreadyExists,
            "network_unavailable" => Self::NetworkUnavailable,
            "auth_expired" => Self::AuthExpired,
            "app_server_crashed" => Self::AppServerCrashed,
            "quota_window_not_returned" => Self::QuotaWindowNotReturned,
            "codex_home_unwritable" => Self::CodexHomeUnwritable,
            "config_syntax_error" => Self::ConfigSyntaxError,
            "auth_file_conflict" => Self::AuthFileConflict,
            "client_running" => Self::ClientRunning,
            "client_shutdown_timeout" => Self::ClientShutdownTimeout,
            "switch_verification_mismatch" => Self::SwitchVerificationMismatch,
            "rollback_failed" => Self::RollbackFailed,
            "display_unavailable" => Self::DisplayUnavailable,
            "credential_store_unavailable" => Self::CredentialStoreUnavailable,
            "app_server_unresponsive" => Self::AppServerUnresponsive,
            "switch_in_progress" => Self::SwitchInProgress,
            "config_conflict" => Self::ConfigConflict,
            "config_layer_readonly" => Self::ConfigLayerReadonly,
            "already_active" => Self::AlreadyActive,
            "external_auth_change" => Self::ExternalAuthChange,
            "thread_unavailable" => Self::ThreadUnavailable,
            "notification_rejected" => Self::NotificationRejected,
            "remote_bridge_rejected" => Self::RemoteBridgeRejected,
            "internal" => Self::Internal,
            _ => return None,
        })
    }
}

/// The operation stage a failure happened in. Answers "which step failed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Detecting the Codex runtime and client installation.
    Detect,
    /// Adding an account through the official login flow.
    Login,
    /// Reading quota through an isolated Codex home.
    ReadQuota,
    /// Pre-switch checks: locks, target credentials, running clients, permissions.
    Precheck,
    /// Backing up the current default authentication.
    Backup,
    /// Writing and atomically replacing the default authentication.
    Write,
    /// Confirming the active identity equals the switch target.
    Verify,
    /// Restoring the backup after a failed switch.
    Rollback,
    /// Closing and reopening the Codex client.
    Restart,
    /// Automatic continuation: taking over, resuming and continuing the bound session.
    Autorun,
    /// Sending a notification to a channel the user configured.
    Notify,
    /// Exchanging a status receipt for a command with the user's own bridge.
    Remote,
    /// Local metadata and settings persistence.
    Storage,
    /// Placing the window against a screen edge.
    Dock,
}

impl Phase {
    /// Stable wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detect => "detect",
            Self::Login => "login",
            Self::ReadQuota => "read_quota",
            Self::Precheck => "precheck",
            Self::Backup => "backup",
            Self::Write => "write",
            Self::Verify => "verify",
            Self::Rollback => "rollback",
            Self::Restart => "restart",
            Self::Storage => "storage",
            Self::Dock => "dock",
            Self::Autorun => "autorun",
            Self::Notify => "notify",
            Self::Remote => "remote",
        }
    }
}

/// What the user can do next. The frontend maps this to localised copy; the Rust layer
/// never produces user-facing prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAction {
    /// Retry the same operation.
    Retry,
    /// Sign in to this account again.
    ReLogin,
    /// Install or repair the Codex runtime.
    InstallRuntime,
    /// Update Codex to a compatible version.
    UpdateRuntime,
    /// Close the running Codex client and try again.
    CloseCodexClient,
    /// Repair `config.toml` by hand, or restore the backup.
    FixConfigManually,
    /// Restore the authentication backup manually; includes recovery instructions.
    RestoreFromBackup,
    /// Check the network connection or proxy settings.
    CheckNetwork,
    /// Unlock the system credential store.
    UnlockCredentialStore,
    /// Wait for the running switch to finish.
    WaitForSwitch,
    /// Grant the current user write access to the Codex home.
    FixPermissions,
    /// Decide what to do with an account that signed in outside Toglet: import it, match it to
    /// a known account, or ignore it.
    ResolveExternalChange,
    /// Choose the Codex session to continue again; the bound one is gone.
    RebindSession,
    /// Correct the notification channel's address, key or account details.
    FixNotificationChannel,
    /// Nothing actionable; informational only.
    None,
}

impl UserAction {
    /// Stable wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::ReLogin => "re_login",
            Self::InstallRuntime => "install_runtime",
            Self::UpdateRuntime => "update_runtime",
            Self::CloseCodexClient => "close_codex_client",
            Self::FixConfigManually => "fix_config_manually",
            Self::RestoreFromBackup => "restore_from_backup",
            Self::CheckNetwork => "check_network",
            Self::UnlockCredentialStore => "unlock_credential_store",
            Self::WaitForSwitch => "wait_for_switch",
            Self::FixPermissions => "fix_permissions",
            Self::ResolveExternalChange => "resolve_external_change",
            Self::RebindSession => "rebind_session",
            Self::FixNotificationChannel => "fix_notification_channel",
            Self::None => "none",
        }
    }
}

/// A Toglet error.
///
/// Fields are private so the free-form `detail` can only be set through
/// [`TogletError::with_detail`], which redacts it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{} failed at {} (retryable={})", .code.as_str(), .phase.as_str(), .retryable)]
pub struct TogletError {
    code: ErrorCode,
    phase: Phase,
    retryable: bool,
    action: UserAction,
    detail: Option<String>,
}

impl TogletError {
    /// `retryable` and `action` are required so every call site decides what the user does next.
    pub fn new(code: ErrorCode, phase: Phase, retryable: bool, action: UserAction) -> Self {
        Self {
            code,
            phase,
            retryable,
            action,
            detail: None,
        }
    }

    /// Attaches diagnostic detail. The value is redacted here, before it can be stored,
    /// logged or returned over IPC.
    #[must_use]
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(redact(detail));
        self
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether retrying the same operation may succeed.
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn action(&self) -> UserAction {
        self.action
    }

    /// Redacted diagnostic detail, if any.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_is_redacted_on_construction() {
        let err = TogletError::new(
            ErrorCode::AuthFileConflict,
            Phase::Write,
            false,
            UserAction::Retry,
        )
        .with_detail(r"failed writing C:\Users\someone\.codex\auth.json for a@b.com");

        let detail = err.detail().expect("detail was set");
        assert!(!detail.contains(r"C:\Users"));
        assert!(!detail.contains("a@b.com"));
    }

    #[test]
    fn display_form_carries_code_phase_and_retryability_only() {
        let err = TogletError::new(
            ErrorCode::SwitchVerificationMismatch,
            Phase::Verify,
            false,
            UserAction::RestoreFromBackup,
        );
        assert_eq!(
            err.to_string(),
            "switch_verification_mismatch failed at verify (retryable=false)"
        );
    }

    #[test]
    fn display_form_never_includes_detail() {
        let err = TogletError::new(ErrorCode::Internal, Phase::Storage, true, UserAction::Retry)
            .with_detail("token eyJhbGciOiJIUzI1NiJ9.payloadpayloadpayload");
        assert!(!err.to_string().contains("redacted"));
        assert!(!err.to_string().contains("eyJ"));
    }

    #[test]
    fn every_error_code_has_a_distinct_stable_string() {
        let codes = [
            ErrorCode::RuntimeNotInstalled,
            ErrorCode::RuntimeIncompatible,
            ErrorCode::LoginCanceled,
            ErrorCode::LoginTimeout,
            ErrorCode::AccountAlreadyExists,
            ErrorCode::NetworkUnavailable,
            ErrorCode::AuthExpired,
            ErrorCode::AppServerCrashed,
            ErrorCode::QuotaWindowNotReturned,
            ErrorCode::CodexHomeUnwritable,
            ErrorCode::ConfigSyntaxError,
            ErrorCode::AuthFileConflict,
            ErrorCode::ClientRunning,
            ErrorCode::ClientShutdownTimeout,
            ErrorCode::SwitchVerificationMismatch,
            ErrorCode::RollbackFailed,
            ErrorCode::DisplayUnavailable,
            ErrorCode::CredentialStoreUnavailable,
            ErrorCode::AppServerUnresponsive,
            ErrorCode::SwitchInProgress,
            ErrorCode::ConfigConflict,
            ErrorCode::ConfigLayerReadonly,
            ErrorCode::AlreadyActive,
            ErrorCode::ExternalAuthChange,
            ErrorCode::ThreadUnavailable,
            ErrorCode::NotificationRejected,
            ErrorCode::RemoteBridgeRejected,
            ErrorCode::Internal,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for code in codes {
            assert!(
                seen.insert(code.as_str()),
                "duplicate code {}",
                code.as_str()
            );
            assert!(!code.as_str().is_empty());
            assert_eq!(ErrorCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(seen.len(), codes.len());
        assert_eq!(ErrorCode::parse("no_such_code"), None);
    }
}
