//! Reading one account's rate limits without touching the default authentication.
//!
//! Shared by the refresh command and automatic continuation; do not duplicate it.

use std::path::Path;

use crate::app_server::{AppServerClient, AppServerSession, CodexBinary, RawRateLimits};
use crate::codex_home::{IsolatedHome, ServerHome, atomic_write};
use crate::credentials::{CredentialLock, CredentialRef, SecretStore, write_back_if_refreshed};
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};

/// Reads the account in use in the default home, the way Codex reads it.
///
/// A copy could receive a refreshed token Codex never sees, possibly signing Codex out. Here a
/// refreshed token lands in Codex's own file, where `external_change` picks it up.
pub fn read_active(binary: &CodexBinary, default_home: &Path) -> Result<RawRateLimits> {
    let home = ServerHome::Default {
        path: default_home.to_path_buf(),
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

/// Reads any other account through a throwaway home holding a copy of its snapshot.
pub fn read_stored(
    lock: &CredentialLock,
    store: &dyn SecretStore,
    binary: &CodexBinary,
    reference: &CredentialRef,
) -> Result<RawRateLimits> {
    let home = IsolatedHome::create(Phase::ReadQuota)?;
    let secret = store.load(reference)?;
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
    // A token refreshed during the read exists only in this throwaway home, so it is written
    // back whether or not the read succeeded. A failed write-back is logged, not fatal: the next
    // read compares again.
    if let Err(error) = write_back_if_refreshed(
        lock,
        store,
        reference,
        &session.home_path(),
        &secret,
        Phase::ReadQuota,
    ) {
        log(&LogRecord::new(Level::Warn, "refreshed_token_not_stored")
            .with_phase(Phase::ReadQuota)
            .with_code(error.code()));
    }
    // Closed on both paths, so a failed read still leaves no subprocess behind.
    let closed = session.close();
    let raw = raw?;
    closed?;
    Ok(raw)
}
