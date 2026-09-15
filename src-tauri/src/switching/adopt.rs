//! Records the account Codex is already signed in as, without switching.
//! Import cannot write `activeAccountId`; without this, every first switch would be refused as
//! an external sign-in change.

use std::path::Path;

use super::verify::{is_target, read_default_identity};
use crate::accounts::onboarding::VerifiedCredentials;
use crate::app_server::CodexBinary;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::storage::SwitchVerified;

const PHASE: Phase = Phase::Verify;

/// Confirms the default home is signed in as `candidate` and issues the token that lets
/// `activeAccountId` be written. Nothing is written to the home. `candidate` is
/// `VerifiedCredentials` so a caller cannot assert an identity it never checked.
pub fn adopt_current_session(
    binary: &CodexBinary,
    default_home: &Path,
    candidate: &VerifiedCredentials,
) -> Result<SwitchVerified> {
    let actual = read_default_identity(binary, default_home, PHASE)?;

    if is_target(actual.as_ref(), candidate.identity()) {
        Ok(SwitchVerified::issue())
    } else {
        // Not `mismatch()`: nothing was replaced, so there is no backup to restore.
        Err(TogletError::new(
            ErrorCode::SwitchVerificationMismatch,
            PHASE,
            false,
            UserAction::ResolveExternalChange,
        )
        .with_detail("the default home is not signed in as this account"))
    }
}
