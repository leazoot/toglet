//! Account commands: list, import the current Codex sign-in, read a quota, remove.

use tauri::State;

use super::state::{AppState, codex_home};
use super::views::{
    AccountView, ErrorView, QuotaView, RemovalView, ResetCreditOutcomeView, rollback_name,
};
use crate::accounts::external_change::ActiveAccount;
use crate::accounts::fingerprint::DuplicateCheck;
use crate::accounts::{onboarding, rate_limits, repository};
use crate::app_server::CodexBinary;
use crate::codex_home::IsolatedHome;
use crate::credentials::CredentialRef;
use crate::diagnostics::{ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction};
use crate::process::{self, SHUTDOWN_TIMEOUT, SystemClientProbe, SystemClientRestart};
use crate::quota::{NormalisedQuota, QuotaSnapshot};
use crate::storage::SwitchVerified;
use crate::switching::{RollbackReport, SignOut, SignOutFailed, adopt_current_session};

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Vec<AccountView> {
    state.read_document(|document| {
        let active = document.settings.active_account_id();
        document
            .accounts
            .iter()
            .map(|profile| AccountView::from_profile(profile, active))
            .collect()
    })
}

/// Imports whoever the default Codex home is signed in as, without writing its `auth.json`.
// `async`: verification starts an app server.
#[tauri::command(async)]
pub fn import_current_account(
    state: State<'_, AppState>,
    display_name: Option<String>,
    now: i64,
) -> std::result::Result<AccountView, ErrorView> {
    import(state.inner(), display_name.as_deref(), now).map_err(ErrorView::from)
}

fn import(state: &AppState, display_name: Option<&str>, now: i64) -> Result<AccountView> {
    let home = codex_home()?;
    let binary = CodexBinary::resolve(Phase::Detect)?;
    let secret = onboarding::read_default_credentials(&home, Phase::Detect)?;
    let verified = onboarding::verify(
        &binary,
        IsolatedHome::create(Phase::Detect)?,
        secret,
        Phase::Detect,
    )?;

    let adopted = adoption(&binary, &home, &verified);

    let id = format!("acct-{now}");
    let created = format!("{now}");
    state.with_document(|document| {
        let outcome = onboarding::adopt(
            state.secrets(),
            document,
            &verified,
            display_name,
            &id,
            &created,
        )?;

        // A duplicate points at the existing account rather than creating a second profile.
        let target = match &outcome {
            DuplicateCheck::AlreadyPresent { existing_id } => existing_id.clone(),
            DuplicateCheck::New => id.clone(),
        };
        // Besides a switch, this is the only path that may set the active account, and it
        // holds the same verification proof a switch produces.
        if let Some(verified_token) = &adopted {
            document
                .settings
                .set_active_account_id(Some(target.clone()), verified_token);
        }

        let active = document.settings.active_account_id().map(str::to_owned);
        let profile = repository::find(document, &target).ok_or_else(|| {
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail("the account was adopted but cannot be found")
        })?;

        let view = AccountView::from_profile(profile, active.as_deref());
        Ok((view, matches!(outcome, DuplicateCheck::New)))
    })
}

/// The proof that the default home is signed in as `verified`, when it is.
///
/// Call before taking the document lock (it starts an app server). `None` does not fail the
/// caller: the account is still listed, just not active; the reason is logged.
pub(crate) fn adoption(
    binary: &CodexBinary,
    home: &std::path::Path,
    verified: &onboarding::VerifiedCredentials,
) -> Option<SwitchVerified> {
    match adopt_current_session(binary, home, verified) {
        Ok(token) => Some(token),
        Err(error) => {
            // A warning: usually Codex is simply signed in as somebody else.
            crate::diagnostics::log(
                &LogRecord::new(Level::Warn, "current_session_not_adopted")
                    .with_phase(error.phase())
                    .with_code(error.code()),
            );
            None
        }
    }
}

/// Reads one account's quota. Never writes the default authentication: an inactive account is
/// read through a throwaway home.
// `async`: starts an app server; a sync command would block the event loop for seconds.
#[tauri::command(async)]
pub fn refresh_quota(
    state: State<'_, AppState>,
    account_id: String,
    now: i64,
) -> std::result::Result<QuotaView, ErrorView> {
    refresh(state.inner(), &account_id, now).map_err(ErrorView::from)
}

fn refresh(state: &AppState, account_id: &str, now: i64) -> Result<QuotaView> {
    let found = state.read_document(|document| {
        let is_active = document.settings.active_account_id() == Some(account_id);
        repository::find(document, account_id)
            .map(|profile| (profile.credential_ref.clone(), is_active))
    });
    let (reference, is_active) = found.ok_or_else(unknown_account)?;
    let reference = CredentialRef::new(&reference)?;
    let binary = CodexBinary::resolve(Phase::ReadQuota)?;

    let raw = if is_active {
        rate_limits::read_active(&binary, &codex_home()?)?
    } else {
        rate_limits::read_stored(
            state.credential_lock(),
            state.secrets(),
            &binary,
            &reference,
        )?
    };

    let snapshot = QuotaSnapshot::fresh(account_id, NormalisedQuota::from_raw(&raw), now);
    Ok(QuotaView::from_snapshot(snapshot.view(now)))
}

/// Redeems one reset credit for an account, clearing the windows it is eligible to clear.
///
/// Irreversible, so the interface confirms first. `now` identifies the attempt rather than the
/// credit: retrying with the same value answers `alreadyRedeemed` instead of spending a second
/// credit. A runtime too old to know the method fails as incompatible, which is shown as such.
// `async`: starts an app server.
#[tauri::command(async)]
pub fn consume_reset_credit(
    state: State<'_, AppState>,
    account_id: String,
    now: i64,
) -> std::result::Result<ResetCreditOutcomeView, ErrorView> {
    consume(state.inner(), &account_id, now).map_err(ErrorView::from)
}

fn consume(state: &AppState, account_id: &str, now: i64) -> Result<ResetCreditOutcomeView> {
    let found = state.read_document(|document| {
        let is_active = document.settings.active_account_id() == Some(account_id);
        repository::find(document, account_id)
            .map(|profile| (profile.credential_ref.clone(), is_active))
    });
    let (reference, is_active) = found.ok_or_else(unknown_account)?;
    let reference = CredentialRef::new(&reference)?;
    let binary = CodexBinary::resolve(Phase::ReadQuota)?;
    let key = format!("reset-{account_id}-{now}");

    let outcome = if is_active {
        rate_limits::consume_active(&binary, &codex_home()?, &key)?
    } else {
        rate_limits::consume_stored(
            state.credential_lock(),
            state.secrets(),
            &binary,
            &reference,
            &key,
        )?
    };
    Ok(ResetCreditOutcomeView::from(outcome))
}

/// Removes an account and its saved sign-in.
///
/// The active account is refused unless `sign_out` is set; then Codex is signed out first and
/// the account leaves the list only once that is verified. The profile is removed before the
/// credential: an orphaned credential is recoverable, a profile pointing at nothing is not.
// `async`: a sign-out starts app servers and may wait for Codex to close.
#[tauri::command(async)]
pub fn remove_account(
    state: State<'_, AppState>,
    account_id: String,
    sign_out: bool,
    now: i64,
) -> std::result::Result<RemovalView, ErrorView> {
    remove(state.inner(), &account_id, sign_out, now).map_err(ErrorView::from)
}

fn remove(state: &AppState, account_id: &str, sign_out: bool, now: i64) -> Result<RemovalView> {
    let found = state.read_document(|document| {
        let is_active = document.settings.active_account_id() == Some(account_id);
        repository::find(document, account_id).map(|profile| {
            (
                profile.credential_ref.clone(),
                profile.account_fingerprint.clone(),
                is_active,
            )
        })
    });
    let (reference, fingerprint, is_active) = found.ok_or_else(unknown_account)?;
    let reference = CredentialRef::new(&reference)?;

    if is_active {
        if !sign_out {
            return Err(TogletError::new(
                ErrorCode::Internal,
                Phase::Storage,
                false,
                UserAction::WaitForSwitch,
            )
            .with_detail("the active account is only removed by switching away or signing out"));
        }
        if let Err(failed) = sign_codex_out(state, account_id, &reference, &fingerprint, now) {
            crate::diagnostics::log(
                &LogRecord::new(Level::Warn, "sign_out_not_completed")
                    .with_phase(failed.error.phase())
                    .with_code(failed.error.code()),
            );
            return Ok(RemovalView {
                removed: false,
                signed_out: false,
                credential_deleted: false,
                rollback: Some(rollback_name(&failed.rollback)),
                error: Some(ErrorView::from(failed.error)),
            });
        }
    }

    let deleted = state.with_document(|document| {
        repository::remove(document, account_id)?;
        Ok((state.secrets().delete(&reference), true))
    })?;

    let credential_deleted = match deleted {
        Ok(()) => true,
        Err(error) => {
            crate::diagnostics::log(
                &LogRecord::new(Level::Warn, "credential_not_deleted")
                    .with_phase(Phase::Storage)
                    .with_code(error.code()),
            );
            false
        }
    };
    Ok(RemovalView {
        removed: true,
        signed_out: is_active,
        credential_deleted,
        rollback: None,
        error: None,
    })
}

/// Signs Codex out of `account_id` and clears `activeAccountId` on verified proof.
///
/// Codex is closed first and not reopened: it would only show its sign-in screen.
fn sign_codex_out(
    state: &AppState,
    account_id: &str,
    reference: &CredentialRef,
    fingerprint: &str,
    now: i64,
) -> std::result::Result<(), SignOutFailed> {
    let home = codex_home().map_err(untouched)?;
    let binary = CodexBinary::resolve(Phase::Precheck).map_err(untouched)?;
    let probe = SystemClientProbe::new();
    let sign_out = SignOut {
        lock: state.switch_lock(),
        credential_lock: state.credential_lock(),
        store: state.secrets(),
        probe: &probe,
        binary: &binary,
        default_home: &home,
        journal_directory: state.data_directory(),
        own_processes: &[],
    };

    let passed = sign_out
        .prepare(Some(ActiveAccount {
            credentials: reference,
            fingerprint,
        }))
        .map_err(|failure| untouched(failure.error))?;

    let restart = SystemClientRestart::new();
    let plan = process::plan(&passed.clients);
    process::close(&restart, &plan, SHUTDOWN_TIMEOUT).map_err(untouched)?;

    let signed_out = sign_out.run(
        passed,
        account_id,
        &format!("signout-{now}"),
        &format!("{now}"),
    )?;

    state
        .with_document(|document| {
            document
                .settings
                .set_active_account_id(None, &signed_out.verified);
            Ok(((), true))
        })
        .map_err(untouched)
}

fn untouched(error: TogletError) -> SignOutFailed {
    SignOutFailed {
        error,
        rollback: RollbackReport::NotNeeded,
    }
}

fn unknown_account() -> TogletError {
    TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
        .with_detail("no account with that id")
}
