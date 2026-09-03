//! Adding an account through the official sign-in.
//!
//! Three commands rather than one, because the sign-in has a middle: the browser is open and
//! nothing has been decided yet. A single blocking command would give the interface no way to
//! offer "cancel", and no way to say what it is waiting for.
//!
//! **The authorisation URL never crosses the boundary.** It carries the PKCE challenge and the
//! OAuth state; Rust hands it to the browser itself (`process::open_url`) and the frontend is
//! told only that a browser was opened.
//!
//! ## What the sign-in cannot do, and why the interface has to say so
//!
//! `account/login/start` takes **no parameters other than `type: "chatgpt"`** - established from
//! `codex app-server generate-json-schema` (`LoginAccountParams`, 2026-09-01). There is no
//! `prompt`, no `loginHint`, no way to ask for the account chooser. So a browser already signed
//! in to ChatGPT reuses that session silently and the user never sees a chooser.
//!
//! Toglet does not work around this by editing the URL: it is an authorisation request the
//! server built, and appending parameters to it would be tampering with an auth request whose
//! effect could not be verified. The interface states the constraint before the browser opens
//! instead, and says plainly what happened when the account turns out to be one already added.

use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use super::accounts::adoption;
use super::state::{AppState, codex_home};
use super::views::{AccountView, ErrorView};
use crate::accounts::fingerprint::DuplicateCheck;
use crate::accounts::onboarding::{self, LoginOutcome, PendingLogin};
use crate::app_server::CodexBinary;
use crate::codex_home::IsolatedHome;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::process;

/// The sign-in that is waiting for the browser, if there is one.
///
/// One at a time. Two would mean two throwaway homes and two app servers open at once, and the
/// second would have no way to tell the user which browser tab belongs to it.
#[derive(Default)]
pub struct PendingSignIn(Mutex<Option<PendingLogin>>);

/// What happened when the account arrived.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedAccountView {
    pub account: AccountView,
    /// `false` when the sign-in produced an account Toglet already had. Not an error - the user
    /// signed in successfully; the browser just reused a session.
    pub added: bool,
}

/// Starts a sign-in and opens the browser.
///
/// Returns nothing: there is nothing the interface may see. The URL stays on this side.
// `async`: starting the sign-in launches a child process. Off the main thread, so the event loop
// keeps serving the tray and the pointer gate meanwhile.
#[tauri::command(async)]
pub fn start_login(
    state: State<'_, AppState>,
    pending: State<'_, PendingSignIn>,
) -> std::result::Result<(), ErrorView> {
    begin(state.inner(), pending.inner()).map_err(ErrorView::from)
}

fn begin(_state: &AppState, pending: &PendingSignIn) -> Result<()> {
    let mut slot = lock(pending);
    if slot.is_some() {
        return Err(TogletError::new(
            ErrorCode::LoginTimeout,
            Phase::Login,
            true,
            UserAction::Retry,
        )
        .with_detail("a sign-in is already waiting for the browser"));
    }

    let binary = CodexBinary::resolve(Phase::Login)?;
    let login = PendingLogin::start(&binary, IsolatedHome::create(Phase::Login)?, Phase::Login)?;

    // Handed straight to the browser. If that fails the sign-in is torn down rather than left
    // running with nowhere for the user to complete it.
    if let Err(error) = process::open_url(login.auth_url(), Phase::Login) {
        drop(login.finish());
        return Err(error);
    }

    *slot = Some(login);
    Ok(())
}

/// Waits for the sign-in to finish, then verifies and stores the account.
///
/// **Asynchronous**: it blocks for as long as the user takes in the browser, and a synchronous
/// command would hold the main thread for all of it.
#[tauri::command]
pub async fn finish_login(
    state: State<'_, AppState>,
    pending: State<'_, PendingSignIn>,
    display_name: Option<String>,
    now: i64,
) -> std::result::Result<AddedAccountView, ErrorView> {
    finish(state.inner(), pending.inner(), display_name.as_deref(), now).map_err(ErrorView::from)
}

/// `display_name: None` names the account after itself - the name it carries at ChatGPT, or
/// the local part of its address. The interface no longer asks for a name before the browser
/// opens.
fn finish(
    state: &AppState,
    pending: &PendingSignIn,
    display_name: Option<&str>,
    now: i64,
) -> Result<AddedAccountView> {
    let mut login = lock(pending).take().ok_or_else(no_sign_in)?;

    let outcome = login.wait(onboarding::LOGIN_TIMEOUT);
    if outcome != LoginOutcome::Completed {
        // Nothing was produced, so there is nothing to store. Reported as what it was - a
        // cancellation and a timeout are different things to the person who caused them.
        login.finish()?;
        return Err(match outcome {
            LoginOutcome::Canceled => TogletError::new(
                ErrorCode::LoginCanceled,
                Phase::Login,
                false,
                UserAction::None,
            ),
            _ => TogletError::new(
                ErrorCode::LoginTimeout,
                Phase::Login,
                true,
                UserAction::Retry,
            ),
        });
    }

    // Read before `finish`, which drops the throwaway home and deletes it.
    let secret = login.credentials(Phase::Login)?;
    login.finish()?;

    let binary = CodexBinary::resolve(Phase::Login)?;
    let verified = onboarding::verify(
        &binary,
        IsolatedHome::create(Phase::Login)?,
        secret,
        Phase::Login,
    )?;

    // The sign-in ran in a throwaway home, so the default home was not touched - but it may
    // already be signed in as exactly this account. Found on the real machine (2026-09-02): a
    // user whose Codex was signed in added that same account through the browser, and Toglet
    // listed it with no current account, while every switch was refused as an external sign-in
    // it did not recognise. The evidence is the same as import's, so the claim is the same:
    // the default home is signed in as a known account.
    let adopted = adoption(&binary, &codex_home()?, &verified);

    let id = format!("acct-{now}");
    let timestamp = now.to_string();
    let (view, added) = state.with_document(|document| {
        let outcome = onboarding::adopt(
            state.secrets(),
            document,
            &verified,
            display_name,
            &id,
            &timestamp,
        )?;

        let (account_id, added) = match &outcome {
            DuplicateCheck::New => (id.as_str(), true),
            DuplicateCheck::AlreadyPresent { existing_id } => (existing_id.as_str(), false),
        };
        if let Some(verified_token) = &adopted {
            document
                .settings
                .set_active_account_id(Some(account_id.to_owned()), verified_token);
        }
        let active = document.settings.active_account_id().map(str::to_owned);
        let profile = crate::accounts::repository::find(document, account_id).ok_or_else(|| {
            TogletError::new(ErrorCode::Internal, Phase::Login, false, UserAction::None)
                .with_detail("the account was not in the document after it was adopted")
        })?;
        Ok((
            (AccountView::from_profile(profile, active.as_deref()), added),
            added,
        ))
    })?;

    Ok(AddedAccountView {
        account: view,
        added,
    })
}

/// Abandons a sign-in the user gave up on.
///
/// Tears the session down whatever the server says: an unknown login id is not an error, and the
/// throwaway home has to go either way.
#[tauri::command]
pub fn cancel_login(pending: State<'_, PendingSignIn>) -> std::result::Result<(), ErrorView> {
    let taken = lock(pending.inner()).take();
    let Some(mut login) = taken else {
        return Ok(());
    };
    let canceled = login.cancel();
    let finished = login.finish();
    canceled.and(finished).map_err(ErrorView::from)
}

fn lock(pending: &PendingSignIn) -> std::sync::MutexGuard<'_, Option<PendingLogin>> {
    // A thread that panicked while holding this has not corrupted the option; refusing every
    // later sign-in would be worse than carrying on.
    pending
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn no_sign_in() -> TogletError {
    TogletError::new(ErrorCode::Internal, Phase::Login, false, UserAction::None)
        .with_detail("no sign-in was waiting")
}
