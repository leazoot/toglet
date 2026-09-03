//! Listing accounts, importing the one Codex is signed in as, reading a quota, and removing an
//! account - the one in use included, by signing Codex out.

use tauri::State;

use super::state::{AppState, codex_home};
use super::views::{AccountView, ErrorView, QuotaView, RemovalView, rollback_name};
use crate::accounts::external_change::ActiveAccount;
use crate::accounts::fingerprint::DuplicateCheck;
use crate::accounts::{onboarding, repository};
use crate::app_server::{AppServerClient, AppServerSession, CodexBinary, RawRateLimits};
use crate::codex_home::{IsolatedHome, ServerHome, atomic_write};
use crate::credentials::{CredentialRef, write_back_if_refreshed};
use crate::diagnostics::{ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction};
use crate::process::{self, SHUTDOWN_TIMEOUT, SystemClientProbe, SystemClientRestart};
use crate::quota::{NormalisedQuota, QuotaSnapshot};
use crate::storage::SwitchVerified;
use crate::switching::{RollbackReport, SignOut, SignOutFailed, adopt_current_session};

/// Every account Toglet knows about.
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

/// Imports whoever the default Codex home is currently signed in as.
///
/// Read-only with respect to the default home: the credentials are copied into a throwaway home
/// to be verified, and the default `auth.json` is never written.
///
/// `display_name: None` names the account after itself.
// `async` for the same reason as `refresh_quota`: it verifies the account through an app-server.
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

        // A duplicate is reported by pointing at the account that already exists, not by
        // creating a second profile.
        let target = match &outcome {
            DuplicateCheck::AlreadyPresent { existing_id } => existing_id.clone(),
            DuplicateCheck::New => id.clone(),
        };
        // Matching the default home to a known account: it is signed in as this
        // one, so it is the active one. This is the only path other than a switch that may say
        // so, and it says it holding the same proof a switch produces.
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
/// Called before the document lock is taken: it starts an app server, and a lock is not held
/// across something that slow.
///
/// `None` does not fail the caller. The account was read and verified and belongs in the list
/// either way; what is not established is that Codex is *using* it, and the honest expression
/// of that is `isActive: false` rather than a refused import or sign-in. The reason is logged:
/// a "no" that came from a reading that failed looks, from the interface, exactly like one that
/// came from a different sign-in, and only the log can tell them apart.
pub(crate) fn adoption(
    binary: &CodexBinary,
    home: &std::path::Path,
    verified: &onboarding::VerifiedCredentials,
) -> Option<SwitchVerified> {
    match adopt_current_session(binary, home, verified) {
        Ok(token) => Some(token),
        Err(error) => {
            // A warning, not an error: the usual reason is that Codex is signed in as somebody
            // else, which is a fact about the machine rather than a fault. The code says which.
            crate::diagnostics::log(
                &LogRecord::new(Level::Warn, "current_session_not_adopted")
                    .with_phase(error.phase())
                    .with_code(error.code()),
            );
            None
        }
    }
}

/// Reads one account's quota.
///
/// **This command never writes the default authentication.** An account that is not in use is
/// read through a throwaway home holding a decrypted copy of its snapshot, which deletes
/// itself. The account in use is read where Codex itself reads it - see
/// [`read_in_default_home`].
// `async`: this starts a `codex app-server` and waits for it. A plain command runs on the main
// thread, and the main thread is the event loop - for the seconds a reading takes, the tray menu,
// every other command and the pointer gate would all be waiting behind it.
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
        read_in_default_home(&binary)?
    } else {
        read_in_isolated_home(state, &binary, &reference)?
    };

    let snapshot = QuotaSnapshot::fresh(account_id, NormalisedQuota::from_raw(&raw), now);
    Ok(QuotaView::from_snapshot(snapshot.view(now)))
}

/// The account in use is read in the default home, the way Codex reads it.
///
/// Reading it from a copy would let the server refresh a token in the copy while Codex keeps
/// the original. Whether the server retires the old refresh token when it does so is not known,
/// and a copy that could silently sign Codex out is not a risk worth carrying to answer a
/// question that reading in place makes moot. Toglet writes nothing here: the server is
/// started against the home and asked one question, and a token it refreshes lands in Codex's
/// own file, where the watcher picks it up for the snapshot (`external_change`).
fn read_in_default_home(binary: &CodexBinary) -> Result<RawRateLimits> {
    let home = ServerHome::Default {
        path: codex_home()?,
        phase: Phase::ReadQuota,
    };
    let mut session = AppServerSession::open(AppServerClient::start(binary, home)?)?;
    let raw = session.read_rate_limits();
    // Closed on both paths, so a failed read still leaves no subprocess behind.
    let closed = session.close();
    let raw = raw?;
    closed?;
    Ok(raw)
}

/// Any other account is read through a throwaway home holding a copy of its snapshot.
fn read_in_isolated_home(
    state: &AppState,
    binary: &CodexBinary,
    reference: &CredentialRef,
) -> Result<RawRateLimits> {
    let home = IsolatedHome::create(Phase::ReadQuota)?;
    let secret = state.secrets().load(reference)?;
    atomic_write(&home.path().join("auth.json"), secret.expose()).map_err(|error| {
        TogletError::new(
            ErrorCode::CodexHomeUnwritable,
            Phase::ReadQuota,
            true,
            UserAction::Retry,
        )
        .with_detail(&error.to_string())
    })?;

    let mut session = AppServerSession::open(AppServerClient::start(binary, home)?)?;
    let raw = session.read_rate_limits();
    // A token the server refreshed during the read exists only in this directory, which goes
    // with the session; the stored snapshot is brought up to date first.
    // Checked whether or not the read succeeded - a refresh can have happened either way. A
    // write-back that failed is recorded and does not fail the reading: the numbers are still
    // right, and the next read compares again.
    if let Err(error) = write_back_if_refreshed(
        state.credential_lock(),
        state.secrets(),
        reference,
        &session.home_path(),
        &secret,
        Phase::ReadQuota,
    ) {
        crate::diagnostics::log(
            &LogRecord::new(Level::Warn, "refreshed_token_not_stored")
                .with_phase(Phase::ReadQuota)
                .with_code(error.code()),
        );
    }
    // Closed on both paths, so a failed read still leaves no subprocess behind.
    let closed = session.close();
    let raw = raw?;
    closed?;
    Ok(raw)
}

/// Removes an account and its saved sign-in.
///
/// The account in use is refused unless `sign_out` is set: Codex would otherwise be left signed
/// in as an account Toglet no longer knows. With it set, Codex is signed out first - the
/// explicit "sign out of the current Codex login" action - under the switch's
/// own discipline, and the account only leaves the list once the sign-out is confirmed.
///
/// The profile goes before the credential and is saved whatever the credential store then says:
/// an orphaned entry there is recoverable, a profile pointing at nothing is not. The answer
/// says which happened rather than folding the second into a failure the list would contradict.
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

/// Signs Codex out of `account_id` and clears `activeAccountId` on the proof that it took.
///
/// Codex is closed first, for the same reason a switch closes it, and **not
/// reopened**: there is nothing for it to open as. The reopen setting is about the account a
/// switch installs, and a Codex reopened here would only show its sign-in screen.
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

    // Only now, and only with the token the verification produced.
    state
        .with_document(|document| {
            document
                .settings
                .set_active_account_id(None, &signed_out.verified);
            Ok(((), true))
        })
        .map_err(untouched)
}

/// A failure before anything was touched: no rollback to report.
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
