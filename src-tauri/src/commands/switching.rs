//! The switch command, and the recovery that runs before the window ever appears.

use tauri::{Emitter, State, WebviewWindow};

use super::state::{AppState, codex_home};
use super::views::{ErrorView, SwitchView, client_outcome_name, rollback_name, verdict_name};
use crate::accounts::repository;
use crate::app_server::CodexBinary;
use crate::credentials::CredentialRef;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::process::{self, ClientProbe, SystemClientProbe, SystemClientRestart};
use crate::switching::{
    ActiveRecord, ClientVerdict, NoFaults, RecoveryOutcome, RollbackReport, StepObserver,
    SwitchContext, SwitchReport, SwitchStep, SwitchTarget, perform, recover, verdict,
};

/// Event announcing each finished switch step. Stable wire name.
pub const SWITCH_STEP_EVENT: &str = "switch://step";

/// Switches to `account_id`: close the client, replace, then reopen, since a running client
/// keeps the credentials it started with. Async so step events reach the webview during the
/// switch. A refusal before anything changed comes back as a view with step 0 and the error.
#[tauri::command]
pub async fn switch_account(
    state: State<'_, AppState>,
    autorun: State<'_, super::autorun::AutoRun>,
    window: WebviewWindow,
    account_id: String,
    now: i64,
) -> std::result::Result<SwitchView, ErrorView> {
    let observer = WindowObserver { window: &window };
    let view = switch(state.inner(), &observer, &account_id, now).unwrap_or_else(refused);
    // A manual switch pauses automatic continuation; a refused one changed nothing.
    if view.switched {
        autorun.observe(crate::autorun::Observation::ManualSwitch);
    }
    Ok(view)
}

/// View of a switch that stopped before touching anything: every `Err` from [`switch`] happens
/// before the backup and replacement. Clients are reported unknown because the probe may not
/// have run yet.
fn refused(error: TogletError) -> SwitchView {
    SwitchView {
        switched: false,
        progress: 0,
        client_up_to_date: false,
        clients: verdict_name(ClientVerdict::Unknown),
        rollback: Some(rollback_name(&RollbackReport::NotNeeded)),
        error: Some(ErrorView::from(error)),
        manual_recovery_required: false,
        client_outcome: None,
    }
}

/// Forwards each finished step to the interface. An undelivered step is logged and does not stop
/// the switch.
struct WindowObserver<'a> {
    window: &'a WebviewWindow,
}

impl StepObserver for WindowObserver<'_> {
    fn completed(&self, step: SwitchStep) {
        if self.window.emit(SWITCH_STEP_EVENT, step.number()).is_err() {
            crate::diagnostics::log(
                &crate::diagnostics::LogRecord::new(
                    crate::diagnostics::Level::Warn,
                    "switch_step_not_delivered",
                )
                .with_phase(Phase::Write),
            );
        }
    }
}

fn switch(
    state: &AppState,
    observer: &dyn StepObserver,
    account_id: &str,
    now: i64,
) -> Result<SwitchView> {
    let home = codex_home()?;
    let binary = CodexBinary::resolve(Phase::Precheck)?;
    let probe = SystemClientProbe::new();
    let restart = SystemClientRestart::new();

    let (target_reference, active) = state.read_document(|document| {
        let target = repository::find(document, account_id).map(|p| p.credential_ref.clone());
        let active = document
            .settings
            .active_account_id()
            .and_then(|id| repository::find(document, id))
            .map(|p| {
                (
                    p.id.clone(),
                    p.credential_ref.clone(),
                    p.account_fingerprint.clone(),
                )
            });
        (target, active)
    });
    let target_reference = CredentialRef::new(&target_reference.ok_or_else(unknown_account)?)?;
    let active_reference = match &active {
        Some((_, reference, _)) => Some(CredentialRef::new(reference)?),
        None => None,
    };

    let context = SwitchContext {
        lock: state.switch_lock(),
        credential_lock: state.credential_lock(),
        store: state.secrets(),
        probe: &probe,
        restart: &restart,
        binary: &binary,
        default_home: &home,
        journal_directory: state.data_directory(),
        own_processes: &[],
        faults: &NoFaults,
        observer,
    };
    let report = perform(
        &context,
        match (&active_reference, &active) {
            (Some(reference), Some((id, _, fingerprint))) => Some(ActiveRecord {
                account_id: id,
                credentials: reference,
                fingerprint,
            }),
            _ => None,
        },
        SwitchTarget {
            account_id,
            credentials: &target_reference,
        },
        &format!("switch-{now}"),
        &format!("{now}"),
    )?;

    Ok(match report {
        SwitchReport::Switched {
            verified,
            progress,
            verdict,
            plan,
        } => {
            // Recorded only now, with the verification token. A failed write does not undo a
            // verified switch: it is logged, and the next sync with the default home repairs it.
            let recorded = state.with_document(|document| {
                document
                    .settings
                    .set_active_account_id(Some(account_id.to_owned()), &verified);
                Ok(((), true))
            });
            let record_error = match recorded {
                Ok(()) => None,
                Err(error) => {
                    crate::diagnostics::log(&crate::diagnostics::LogRecord::from_error(
                        "active_account_not_recorded",
                        &error,
                    ));
                    Some(ErrorView::from(error))
                }
            };

            let reopen =
                state.read_document(|document| document.settings.reopen_codex_after_switch);
            let outcome = if reopen {
                process::reopen(&restart, &plan)
            } else {
                process::ClientOutcome::ClosedByChoice
            };
            SwitchView {
                switched: true,
                progress: progress.number(),
                client_up_to_date: outcome.client_is_up_to_date(),
                clients: verdict_name(verdict),
                rollback: None,
                error: record_error,
                manual_recovery_required: false,
                client_outcome: Some(client_outcome_name(&outcome)),
            }
        }
        SwitchReport::Failed {
            error,
            progress,
            verdict,
            rollback,
        } => SwitchView {
            switched: false,
            progress: progress.number(),
            client_up_to_date: false,
            clients: verdict_name(verdict),
            rollback: Some(rollback_name(&rollback)),
            manual_recovery_required: matches!(rollback, RollbackReport::Failed { .. }),
            error: Some(ErrorView::from(error)),
            client_outcome: None,
        },
    })
}

/// Finishes or undoes a crash-interrupted switch, at start-up before anything is shown.
/// No expected target is passed, to avoid decrypting credentials at start-up; rolling back is
/// safe, and the user can repeat the switch.
pub fn recover_interrupted_switch(state: &AppState) -> Result<Option<&'static str>> {
    let home = codex_home()?;
    let binary = CodexBinary::resolve(Phase::Verify)?;
    let outcome = recover(&binary, &home, state.data_directory(), None)?;

    Ok(match outcome {
        RecoveryOutcome::NothingToDo => None,
        RecoveryOutcome::RolledBack => Some("rolled_back"),
        RecoveryOutcome::Completed { .. } => Some("completed"),
        RecoveryOutcome::Failed { .. } => Some("failed"),
    })
}

/// What the running clients mean for a switch, without starting one.
// `async`: walking the process list is slow and must stay off the main thread.
#[tauri::command(async)]
pub fn inspect_clients() -> &'static str {
    let probe = SystemClientProbe::new();
    verdict_name(verdict(&probe.running_clients(&[])))
}

fn unknown_account() -> TogletError {
    TogletError::new(
        ErrorCode::Internal,
        Phase::Precheck,
        false,
        UserAction::None,
    )
    .with_detail("no account with that id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_switch_is_a_view_that_says_nothing_was_replaced() {
        let view = refused(TogletError::new(
            ErrorCode::ClientRunning,
            Phase::Precheck,
            true,
            UserAction::CloseCodexClient,
        ));

        assert!(!view.switched);
        assert_eq!(view.progress, 0);
        assert_eq!(view.rollback, Some("not_needed"));
        assert!(!view.manual_recovery_required);
        assert_eq!(view.client_outcome, None);
        let error = view.error.expect("the refusal is carried");
        assert_eq!(error.code, "client_running");
        assert_eq!(error.phase, "precheck");
        assert!(error.retryable);
    }
}
