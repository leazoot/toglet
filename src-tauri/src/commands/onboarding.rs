//! Adding an account through the official sign-in: start, finish and cancel, so the interface
//! can offer cancel. The authorisation URL (PKCE challenge, OAuth state) never leaves Rust.
//!
//! `account/login/start` accepts only `type: "chatgpt"` (no prompt or login hint), so a browser
//! already signed in reuses its session silently. The URL is deliberately not modified.

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

/// The sign-in waiting for the browser; at most one at a time.
#[derive(Default)]
pub struct PendingSignIn(Mutex<Option<PendingLogin>>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedAccountView {
    pub account: AccountView,
    /// `false` when the account already existed; not an error.
    pub added: bool,
}

/// Starts a sign-in and opens the browser; the URL is not returned.
// `async`: launches a child process off the event loop.
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

    // If the browser cannot open, tear the sign-in down rather than leave it running.
    if let Err(error) = process::open_url(login.auth_url(), Phase::Login) {
        drop(login.finish());
        return Err(error);
    }

    *slot = Some(login);
    Ok(())
}

/// Waits for the sign-in, then verifies and stores the account. Async: it blocks for as long as
/// the user takes in the browser.
#[tauri::command]
pub async fn finish_login(
    state: State<'_, AppState>,
    pending: State<'_, PendingSignIn>,
    display_name: Option<String>,
    now: i64,
) -> std::result::Result<AddedAccountView, ErrorView> {
    finish(state.inner(), pending.inner(), display_name.as_deref(), now).map_err(ErrorView::from)
}

/// `display_name: None` names the account after its ChatGPT name or address local part.
fn finish(
    state: &AppState,
    pending: &PendingSignIn,
    display_name: Option<&str>,
    now: i64,
) -> Result<AddedAccountView> {
    let mut login = lock(pending).take().ok_or_else(no_sign_in)?;

    let outcome = login.wait(onboarding::LOGIN_TIMEOUT);
    if outcome != LoginOutcome::Completed {
        // Cancellation and timeout are reported distinctly.
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

    // The default home may already be signed in as this account; without adopting it, the
    // account would be listed as inactive and switches refused as an unknown external sign-in.
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

/// Tears the sign-in down whatever the server says; the throwaway home goes either way.
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
    // A panic cannot leave the option inconsistent, so poisoning is ignored.
    pending
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn no_sign_in() -> TogletError {
    TogletError::new(ErrorCode::Internal, Phase::Login, false, UserAction::None)
        .with_detail("no sign-in was waiting")
}
