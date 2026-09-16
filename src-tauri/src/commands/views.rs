//! The only shapes that cross the IPC boundary, kept separate from domain types so widening them
//! is deliberate. None may carry a token, absolute path, command line or full email address.

use serde::Serialize;

use crate::accounts::AccountProfile;
use crate::app_server::ResetOutcome;
use crate::diagnostics::TogletError;
use crate::process::ClientOutcome;
use crate::quota::{QuotaSnapshotView, ResetCredits, WindowKind};
use crate::switching::{ClientVerdict, RollbackReport};

/// What removing an account did.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalView {
    pub removed: bool,
    /// Whether Codex was signed out for it. Only ever true for the account that was in use.
    pub signed_out: bool,
    /// False when the profile is gone but the credential store kept its entry.
    pub credential_deleted: bool,
    /// What happened to Codex's sign-in when a sign-out failed; `None` when nothing was touched.
    /// Same vocabulary as `SwitchView.rollback`.
    pub rollback: Option<&'static str>,
    /// Present only on failure.
    pub error: Option<ErrorView>,
}

/// One account, as the panel shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    /// The random internal id - the only account identifier that may leave the Rust layer.
    pub id: String,
    pub display_name: String,
    /// Masked when stored; the full address never reaches this side.
    pub masked_email: Option<String>,
    /// `null` when unknown, never a guess.
    pub plan_type: Option<String>,
    pub status: &'static str,
    pub is_active: bool,
}

impl AccountView {
    pub fn from_profile(profile: &AccountProfile, active_account_id: Option<&str>) -> Self {
        Self {
            id: profile.id.clone(),
            display_name: profile.display_name.clone(),
            masked_email: profile.masked_email.clone(),
            plan_type: profile.plan_type.clone(),
            status: profile.status.as_str(),
            is_active: active_account_id == Some(profile.id.as_str()),
        }
    }
}

/// One quota window. Percentages are not optional: a window the server did not return is absent
/// from the list, so nothing is rendered as `0`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindowView {
    pub kind: &'static str,
    pub used_percent: f64,
    pub remaining_percent: f64,
    /// Unix seconds, absolute. The frontend converts to local time.
    pub resets_at: Option<i64>,
}

/// A quota reading.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaView {
    pub account_id: String,
    /// May be empty or hold only one window.
    pub windows: Vec<QuotaWindowView>,
    pub fetched_at: i64,
    pub source: &'static str,
    pub stale: bool,
    pub last_error_code: Option<String>,
    /// Reset credits, or `None` when the server did not report them at all. A count of zero is
    /// sent as-is; deciding not to draw it is the interface's job.
    pub reset_credits: Option<ResetCredits>,
}

impl QuotaView {
    pub fn from_snapshot(view: QuotaSnapshotView<'_>) -> Self {
        Self {
            account_id: view.account_id.to_owned(),
            windows: view
                .quota
                .windows
                .iter()
                // `Other` and `Unknown` windows stay in the data model but are not shown.
                .filter(|window| matches!(window.kind, WindowKind::FiveHour | WindowKind::Weekly))
                .map(|window| QuotaWindowView {
                    kind: window.kind.as_str(),
                    used_percent: window.used_percent,
                    remaining_percent: window.remaining_percent,
                    resets_at: window.resets_at,
                })
                .collect(),
            fetched_at: view.fetched_at,
            source: view.source,
            stale: view.stale,
            last_error_code: view.last_error_code.map(str::to_owned),
            reset_credits: view.quota.reset_credits,
        }
    }
}

/// What redeeming a reset credit did.
///
/// `succeeded` is true only for `reset`. Every other outcome left the account's credits untouched
/// and must be presented as a refusal, not a quiet success.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCreditOutcomeView {
    pub outcome: ResetOutcome,
    pub succeeded: bool,
}

impl From<ResetOutcome> for ResetCreditOutcomeView {
    fn from(outcome: ResetOutcome) -> Self {
        Self {
            outcome,
            succeeded: outcome.succeeded(),
        }
    }
}

/// The result of a switch. `switched` says whether the account changed; `clientUpToDate` says
/// whether Codex is running it, since a client holding old credentials is not a plain success.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchView {
    pub switched: bool,
    /// 0 to 4, and only ever the steps that actually finished.
    pub progress: u8,
    pub client_up_to_date: bool,
    /// What the running clients meant for this switch.
    pub clients: &'static str,
    /// What happened to the previous authentication when a switch failed.
    pub rollback: Option<&'static str>,
    /// Present only on failure.
    pub error: Option<ErrorView>,
    /// The user must restore the previous credentials by hand. The backup path is not sent.
    pub manual_recovery_required: bool,
    /// What happened to the Codex client afterwards; `null` when the switch failed.
    pub client_outcome: Option<&'static str>,
}

/// A failure as the frontend sees it. `detail` is omitted: OS messages can contain paths.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorView {
    pub code: &'static str,
    pub phase: &'static str,
    pub retryable: bool,
    pub action: &'static str,
}

impl From<TogletError> for ErrorView {
    fn from(error: TogletError) -> Self {
        Self {
            code: error.code().as_str(),
            phase: error.phase().as_str(),
            retryable: error.retryable(),
            action: error.action().as_str(),
        }
    }
}

/// Stable wire forms for the switch outcome pieces.
pub fn verdict_name(verdict: ClientVerdict) -> &'static str {
    match verdict {
        ClientVerdict::Clear => "clear",
        ClientVerdict::DesktopOnly => "desktop_only",
        ClientVerdict::Blocked => "blocked",
        ClientVerdict::Unknown => "unknown",
    }
}

pub fn rollback_name(report: &RollbackReport) -> &'static str {
    report.as_str()
}

pub fn client_outcome_name(outcome: &ClientOutcome) -> &'static str {
    match outcome {
        ClientOutcome::NothingWasRunning => "nothing_was_running",
        ClientOutcome::Reopened => "reopened",
        ClientOutcome::ClosedNotReopened { .. } => "closed_not_reopened",
        ClientOutcome::ClosedByChoice => "closed_by_choice",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::AccountStatus;
    use crate::diagnostics::{ErrorCode, Phase, UserAction};

    fn profile() -> AccountProfile {
        AccountProfile {
            id: "acct-1".to_owned(),
            display_name: "Work".to_owned(),
            masked_email: Some("lea***@gmail.com".to_owned()),
            account_fingerprint: "a4759ad1f614bd70348afc1960200e7b".to_owned(),
            plan_type: Some("plus".to_owned()),
            auth_mode: "chatgpt".to_owned(),
            credential_ref: "cred-acct-1".to_owned(),
            status: AccountStatus::Ready,
            created_at: "1788197453".to_owned(),
            updated_at: "1788197453".to_owned(),
            last_validated_at: None,
        }
    }

    #[test]
    fn an_account_view_carries_neither_the_fingerprint_nor_the_credential_key() {
        // Both are account identifiers the frontend must not see.
        let json = serde_json::to_string(&AccountView::from_profile(&profile(), Some("acct-1")))
            .expect("serialises");

        assert!(!json.contains("a4759ad1"), "{json}");
        assert!(!json.contains("cred-acct-1"), "{json}");
        assert!(
            json.contains("lea***@gmail.com"),
            "the masked form is what is shown"
        );
        assert!(json.contains("\"isActive\":true"));
    }

    #[test]
    fn an_error_view_does_not_carry_the_detail() {
        // The detail can hold an operating-system message, and those hold paths.
        let error = TogletError::new(
            ErrorCode::CodexHomeUnwritable,
            Phase::Write,
            true,
            UserAction::FixPermissions,
        )
        .with_detail("could not create C:/Users/somebody/.codex/auth.json");

        let json = serde_json::to_string(&ErrorView::from(error)).expect("serialises");

        assert!(!json.contains("Users"), "{json}");
        assert!(!json.contains(".codex"), "{json}");
        assert!(json.contains("codex_home_unwritable"));
        assert!(json.contains("fix_permissions"));
    }

    #[test]
    fn a_failed_switch_is_never_serialised_as_a_success() {
        let view = SwitchView {
            switched: false,
            progress: 2,
            client_up_to_date: false,
            clients: "clear",
            rollback: Some("restored"),
            error: Some(ErrorView {
                code: "switch_verification_mismatch",
                phase: "verify",
                retryable: false,
                action: "restore_from_backup",
            }),
            manual_recovery_required: false,
            client_outcome: None,
        };

        let json = serde_json::to_string(&view).expect("serialises");

        assert!(json.contains("\"switched\":false"));
        assert!(json.contains("\"progress\":2"), "progress must not claim 4");
        assert!(json.contains("switch_verification_mismatch"));
    }

    #[test]
    fn a_switch_whose_client_was_not_reopened_is_not_reported_as_fully_done() {
        // The two-part result: the account changed, Codex is not running it yet.
        let view = SwitchView {
            switched: true,
            progress: 4,
            client_up_to_date: false,
            clients: "desktop_only",
            rollback: None,
            error: None,
            manual_recovery_required: false,
            client_outcome: Some("closed_not_reopened"),
        };

        let json = serde_json::to_string(&view).expect("serialises");

        assert!(json.contains("\"switched\":true"));
        assert!(json.contains("\"clientUpToDate\":false"));
    }

    #[test]
    fn a_rollback_that_failed_asks_for_manual_recovery_without_sending_the_path() {
        let report = RollbackReport::Failed {
            backup: std::path::PathBuf::from("C:/Users/somebody/.codex/auth.json.toglet-switch-1"),
        };

        let view = SwitchView {
            switched: false,
            progress: 2,
            client_up_to_date: false,
            clients: "clear",
            rollback: Some(rollback_name(&report)),
            error: None,
            manual_recovery_required: true,
            client_outcome: None,
        };
        let json = serde_json::to_string(&view).expect("serialises");

        assert!(json.contains("\"manualRecoveryRequired\":true"));
        assert!(
            !json.contains("Users"),
            "the path must stay on the Rust side: {json}"
        );
    }
}
