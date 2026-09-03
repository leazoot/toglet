//! The switch command, and the recovery that runs before the window ever appears.

use tauri::{Emitter, State, WebviewWindow};

use super::state::{AppState, codex_home};
use super::views::{ErrorView, SwitchView, client_outcome_name, rollback_name, verdict_name};
use crate::accounts::external_change::ActiveAccount;
use crate::accounts::repository;
use crate::app_server::CodexBinary;
use crate::credentials::CredentialRef;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::process::{
    self, ClientProbe, RestartPlan, SHUTDOWN_TIMEOUT, SystemClientProbe, SystemClientRestart,
};
use crate::switching::{
    NoFaults, Preflight, PreflightPassed, RecoveryOutcome, RollbackReport, StepObserver, Switch,
    SwitchStep, SwitchTarget, recover, verdict,
};

/// Switches to `account_id`.
///
/// The order is fixed, and it is the reason this lives here rather than in
/// `switching`: **close the client first, then replace, then reopen.** A client that is still
/// running keeps using the credentials it started with, so replacing them underneath it
/// produces the disagreement between Toglet and Codex that the honest-state rule forbids.
/// The event each finished step is announced on. Stable wire name.
pub const SWITCH_STEP_EVENT: &str = "switch://step";

/// **Asynchronous on purpose.** A synchronous command runs on the main thread, and the webview
/// could not receive a step until the whole switch had finished - which would turn the four-step
/// progress into a single jump at the end, and make the panel show four steps it never watched.
#[tauri::command]
pub async fn switch_account(
    state: State<'_, AppState>,
    window: WebviewWindow,
    account_id: String,
    now: i64,
) -> std::result::Result<SwitchView, ErrorView> {
    let observer = WindowObserver { window: &window };
    switch(state.inner(), &observer, &account_id, now).map_err(ErrorView::from)
}

/// Forwards each finished step to the interface.
///
/// A step that cannot be delivered is recorded and does not stop the switch: the progress display
/// is a courtesy, and failing a switch because a notification did not arrive would be the wrong
/// trade every time.
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

    let preflight = Preflight {
        lock: state.switch_lock(),
        credential_lock: state.credential_lock(),
        store: state.secrets(),
        probe: &probe,
        binary: &binary,
        default_home: &home,
        own_processes: &[],
    };

    let passed = preflight
        .run(
            active.as_ref().map(|(id, _, _)| id.as_str()),
            match (&active_reference, &active) {
                (Some(reference), Some((_, _, fingerprint))) => Some(ActiveAccount {
                    credentials: reference,
                    fingerprint,
                }),
                _ => None,
            },
            SwitchTarget {
                account_id,
                credentials: &target_reference,
            },
        )
        .map_err(|failure| failure.error)?;

    let restart = SystemClientRestart::new();
    let plan = process::plan(&passed.clients);
    // Before anything is replaced. A client that will not close stops the switch here, where
    // stopping costs nothing because the credentials have not been touched.
    process::close(&restart, &plan, SHUTDOWN_TIMEOUT)?;

    finish(
        state, observer, passed, account_id, &home, &binary, &plan, &restart, now,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish(
    state: &AppState,
    observer: &dyn StepObserver,
    passed: PreflightPassed<'_>,
    account_id: &str,
    home: &std::path::Path,
    binary: &CodexBinary,
    plan: &RestartPlan,
    restart: &SystemClientRestart,
    now: i64,
) -> Result<SwitchView> {
    let verdict = passed.verdict;
    let from =
        state.read_document(|document| document.settings.active_account_id().map(str::to_owned));

    let switch = Switch {
        binary,
        default_home: home,
        journal_directory: state.data_directory(),
        faults: &NoFaults,
        observer,
    };

    match switch.run(
        passed,
        from.as_deref(),
        account_id,
        &format!("switch-{now}"),
        &format!("{now}"),
    ) {
        Ok(succeeded) => {
            // Only now, and only with the token the verification produced.
            state.with_document(|document| {
                document
                    .settings
                    .set_active_account_id(Some(account_id.to_owned()), &succeeded.verified);
                Ok(((), true))
            })?;

            // The user can turn the reopen off. Without this the setting would be a switch
            // that changes nothing.
            let reopen =
                state.read_document(|document| document.settings.reopen_codex_after_switch);
            let outcome = if reopen {
                process::reopen(restart, plan)
            } else {
                process::ClientOutcome::ClosedByChoice
            };
            Ok(SwitchView {
                switched: true,
                progress: succeeded.progress.number(),
                client_up_to_date: outcome.client_is_up_to_date(),
                clients: verdict_name(verdict),
                rollback: None,
                error: None,
                manual_recovery_required: false,
                client_outcome: Some(client_outcome_name(&outcome)),
            })
        }
        Err(failed) => Ok(SwitchView {
            switched: false,
            progress: failed.progress.number(),
            client_up_to_date: false,
            clients: verdict_name(verdict),
            rollback: Some(rollback_name(&failed.rollback)),
            manual_recovery_required: matches!(failed.rollback, RollbackReport::Failed { .. }),
            error: Some(ErrorView::from(failed.error)),
            client_outcome: None,
        }),
    }
}

/// Finishes or undoes a switch that a crash interrupted.
///
/// Called at start-up, before anything is shown. `expected_target` is not resolved here: doing
/// so would mean decrypting and verifying the target's credentials during start-up, and a
/// recovery that rolls back is already correct and safe. Completing an interrupted switch is
/// left to the user repeating it.
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
// `async`: walking the process list is not free, and the main thread is the event loop.
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
