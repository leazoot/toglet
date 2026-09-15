//! Account import, sign-in, re-authentication and removal.
//!
//! Credentials are verified by an app server before they are stored or replace a snapshot. The
//! default Codex home is only ever read here; writing it is `switching`'s job alone.

use std::path::Path;
use std::time::Duration;

use super::auth_file;
use super::fingerprint::{self, DuplicateCheck, check_duplicate};
use super::identity::AccountIdentity;
use super::kind::AccountKind;
use super::profile::{AccountProfile, default_display_name, mask_email, validate_display_name};
use super::repository;
use crate::app_server::{AppServerClient, AppServerSession, CodexBinary};
use crate::codex_home::IsolatedHome;
use crate::credentials::{CredentialRef, Secret, SecretStore};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::storage::MetadataDocument;

/// Generous so a password manager or second factor never races the timeout.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Constructed only by [`verify`], so a profile cannot be created from unchecked credentials.
pub struct VerifiedCredentials {
    identity: AccountIdentity,
    kind: AccountKind,
    fingerprint: String,
    display_name: Option<String>,
    secret: Secret,
}

impl VerifiedCredentials {
    pub fn identity(&self) -> &AccountIdentity {
        &self.identity
    }

    pub fn kind(&self) -> &AccountKind {
        &self.kind
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// `pub(crate)`: only the switch may install these bytes into the default home.
    pub(crate) fn secret(&self) -> &Secret {
        &self.secret
    }

    /// The only form of the address that may leave the Rust layer.
    pub fn masked_email(&self) -> Option<String> {
        self.identity.email().and_then(mask_email)
    }

    /// Errors only when there is neither a name nor an address, which `verify` already refuses
    /// for a ChatGPT account.
    pub fn default_display_name(&self) -> Result<String> {
        default_display_name(self.display_name.as_deref(), self.identity.email()).ok_or_else(|| {
            TogletError::new(
                ErrorCode::AuthFileConflict,
                Phase::Login,
                false,
                UserAction::ReLogin,
            )
            .with_detail("the credentials carry nothing to name the account after")
        })
    }
}

/// Runs an app server against `secret` in a throwaway home and reports who it belongs to.
///
/// `home`, decrypted credentials included, is removed by its guard on every path.
pub fn verify(
    binary: &CodexBinary,
    home: IsolatedHome,
    secret: Secret,
    phase: Phase,
) -> Result<VerifiedCredentials> {
    let facts = auth_file::read(&secret).ok_or_else(|| {
        TogletError::new(
            ErrorCode::AuthFileConflict,
            phase,
            false,
            UserAction::ReLogin,
        )
        .with_detail("the credential file could not be read")
    })?;

    crate::codex_home::permissions::write_private_file(
        &home.path().join("auth.json"),
        secret.expose(),
    )
    .map_err(|error| {
        TogletError::new(
            ErrorCode::CodexHomeUnwritable,
            phase,
            true,
            UserAction::Retry,
        )
        .with_detail(&error.to_string())
    })?;

    let mut session = AppServerSession::open(AppServerClient::start(binary, home)?)?;
    let account = session.read_account();
    session.close()?;

    // A corrupted file also yields `account: null`, so this is a failure, not "signed out".
    let identity = account?.ok_or_else(|| {
        TogletError::new(ErrorCode::AuthExpired, phase, false, UserAction::ReLogin)
            .with_detail("the credentials do not identify an account")
    })?;

    let kind = match facts.auth_mode.as_deref() {
        Some(mode) => AccountKind::from_auth_mode(mode),
        None => AccountKind::Unknown(String::new()),
    };

    // Prefer the stable id; fall back to the address when the file carries none.
    let fingerprint = facts
        .fingerprint
        .or_else(|| identity.email().map(fingerprint::from_email));
    let fingerprint = fingerprint.ok_or_else(|| {
        TogletError::new(
            ErrorCode::AuthFileConflict,
            phase,
            false,
            UserAction::ReLogin,
        )
        .with_detail("the credentials carry no stable identifier")
    })?;

    Ok(VerifiedCredentials {
        identity,
        kind,
        fingerprint,
        display_name: facts.display_name,
        secret,
    })
}

/// Reads the default home's `auth.json`; nothing in the home is written.
pub fn read_default_credentials(home: &Path, phase: Phase) -> Result<Secret> {
    let bytes = std::fs::read(home.join("auth.json")).map_err(|error| {
        TogletError::new(
            ErrorCode::CodexHomeUnwritable,
            phase,
            false,
            UserAction::InstallRuntime,
        )
        .with_detail(&error.to_string())
    })?;
    Ok(Secret::new(bytes))
}

/// Stores verified credentials and creates a profile.
///
/// Returns [`DuplicateCheck::AlreadyPresent`] without storing anything when the account is
/// already known. `display_name: None` names the account after its own name or address.
pub fn adopt(
    store: &dyn SecretStore,
    document: &mut MetadataDocument,
    verified: &VerifiedCredentials,
    display_name: Option<&str>,
    id: &str,
    now: &str,
) -> Result<DuplicateCheck> {
    let existing = document
        .accounts
        .iter()
        .map(|account| (account.id.as_str(), account.account_fingerprint.as_str()));
    if let DuplicateCheck::AlreadyPresent { existing_id } =
        check_duplicate(&verified.fingerprint, existing)
    {
        return Ok(DuplicateCheck::AlreadyPresent { existing_id });
    }

    let name = match display_name {
        Some(name) => validate_display_name(name)?,
        None => validate_display_name(&verified.default_display_name()?)?,
    };
    let credential_ref = CredentialRef::new(&format!("cred-{id}"))?;

    let profile = AccountProfile {
        id: id.to_owned(),
        display_name: name,
        masked_email: verified.masked_email(),
        account_fingerprint: verified.fingerprint.clone(),
        plan_type: match verified.identity() {
            AccountIdentity::Chatgpt { plan_type, .. } => plan_type.clone(),
            AccountIdentity::ApiKey => None,
        },
        auth_mode: match verified.kind() {
            AccountKind::Chatgpt => "chatgpt".to_owned(),
            AccountKind::ApiKey => "apikey".to_owned(),
            AccountKind::HostManagedTokens => "chatgptAuthTokens".to_owned(),
            AccountKind::Unknown(mode) => mode.clone(),
        },
        credential_ref: credential_ref.as_str().to_owned(),
        status: verified.kind().status(),
        created_at: now.to_owned(),
        updated_at: now.to_owned(),
        last_validated_at: Some(now.to_owned()),
    };

    // Credentials first: a profile pointing at a missing entry is worse than an orphaned entry.
    store.store(&credential_ref, &verified.secret)?;
    match repository::add(document, profile) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            // Most likely the account limit; do not leave the credential behind.
            drop(store.delete(&credential_ref));
            Err(error)
        }
    }
}

/// Replaces an existing account's stored credentials after re-authentication.
///
/// The new credentials must belong to the same account, or the profile would be silently
/// repointed and the next switch would sign in as the wrong person.
pub fn reauthenticate(
    store: &dyn SecretStore,
    document: &mut MetadataDocument,
    id: &str,
    verified: &VerifiedCredentials,
    now: &str,
) -> Result<()> {
    let account = document
        .accounts
        .iter_mut()
        .find(|account| account.id == id)
        .ok_or_else(|| {
            TogletError::new(ErrorCode::Internal, Phase::Login, false, UserAction::None)
                .with_detail("no such account")
        })?;

    if account.account_fingerprint != verified.fingerprint {
        return Err(TogletError::new(
            ErrorCode::SwitchVerificationMismatch,
            Phase::Login,
            false,
            UserAction::ReLogin,
        )
        .with_detail("the new credentials belong to a different account"));
    }

    let credential_ref = CredentialRef::new(&account.credential_ref)?;
    // Only now is the old snapshot replaced. A failed verification above leaves it untouched.
    store.store(&credential_ref, &verified.secret)?;

    account.masked_email = verified.masked_email();
    account.status = verified.kind().status();
    account.updated_at = now.to_owned();
    account.last_validated_at = Some(now.to_owned());
    Ok(())
}

/// Removes an account and its stored credentials.
///
/// The profile goes first: an orphaned credential entry is recoverable, a dangling profile is not.
pub fn forget(
    store: &dyn SecretStore,
    document: &mut MetadataDocument,
    id: &str,
) -> Result<AccountProfile> {
    let removed = repository::remove(document, id)?;
    store.delete(&CredentialRef::new(&removed.credential_ref)?)?;
    Ok(removed)
}

/// A sign-in waiting for the user to finish in their browser.
pub struct PendingLogin {
    session: AppServerSession,
    login_id: String,
    auth_url: String,
    canceled_locally: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginOutcome {
    Completed,
    /// Distinguishable from `Failed` only because Toglet remembers asking to cancel.
    Canceled,
    TimedOut,
    /// The server reported failure and Toglet did not cancel.
    Failed,
}

impl PendingLogin {
    /// Starts a sign-in in a throwaway home; the default home is untouched throughout.
    pub fn start(binary: &CodexBinary, home: IsolatedHome, _phase: Phase) -> Result<Self> {
        let mut session = AppServerSession::open(AppServerClient::start(binary, home)?)?;
        let (login_id, auth_url) = session.login_start()?;

        Ok(Self {
            session,
            login_id,
            auth_url,
            canceled_locally: false,
        })
    }

    /// Never logged: it carries PKCE parameters.
    pub fn auth_url(&self) -> &str {
        &self.auth_url
    }

    /// Asks the server to stop the sign-in and records that Toglet asked.
    pub fn cancel(&mut self) -> Result<()> {
        self.canceled_locally = true;
        self.session.login_cancel(&self.login_id)
    }

    pub fn wait(&mut self, timeout: Duration) -> LoginOutcome {
        match self.session.await_login_completion(&self.login_id, timeout) {
            Ok(true) => LoginOutcome::Completed,
            // The server says the same thing either way; only the local flag tells them apart.
            Ok(false) if self.canceled_locally => LoginOutcome::Canceled,
            Ok(false) => LoginOutcome::Failed,
            Err(error) if error.code() == ErrorCode::AppServerUnresponsive => {
                LoginOutcome::TimedOut
            }
            Err(_) => LoginOutcome::Failed,
        }
    }

    /// Reads the credentials the completed sign-in produced.
    pub fn credentials(&self, phase: Phase) -> Result<Secret> {
        read_default_credentials(&self.session.home_path(), phase)
    }

    /// Cancels first if still running, so no subprocess is left waiting on an unused browser.
    pub fn finish(mut self) -> Result<()> {
        if !self.canceled_locally {
            drop(self.session.login_cancel(&self.login_id));
        }
        self.session.close()
    }
}
