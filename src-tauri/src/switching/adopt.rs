//! Recording the account Codex is **already** signed in as.
//!
//! Found by the real-machine acceptance on 2026-09-01, not by a test: after importing the
//! account the default home holds, Toglet had no way to record it as the active one. Import
//! cannot do it - `activeAccountId` may only be written with a token that only `switching`
//! issues - so `activeAccountId` stayed `null`, every session in the default home
//! looked like a stranger's, and the pre-checks refused **every first switch** with
//! `ExternalAuthChange`. The manual loop could not be completed at all.
//!
//! The same operation is what resolves an external sign-in that turns out to be an account
//! Toglet already knows.

use std::path::Path;

use super::verify::{is_target, read_default_identity};
use crate::accounts::onboarding::VerifiedCredentials;
use crate::app_server::CodexBinary;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::storage::SwitchVerified;

const PHASE: Phase = Phase::Verify;

/// Confirms that the default Codex home is signed in as `candidate`, and issues the token that
/// lets `activeAccountId` be written.
///
/// **Nothing is written to the default home.** This is not a switch; it is the verification half
/// of one, run on its own.
///
/// Issuing the same token a switch issues is not a loophole. The evidence is identical: a
/// freshly started app server is asked who the **default** home is signed in as, and the answer
/// is compared with an identity that only [`crate::accounts::onboarding::verify`] can have
/// produced. `candidate` is a `VerifiedCredentials` rather than a plain identity precisely so a
/// caller cannot assert an identity it never checked.
pub fn adopt_current_session(
    binary: &CodexBinary,
    default_home: &Path,
    candidate: &VerifiedCredentials,
) -> Result<SwitchVerified> {
    let actual = read_default_identity(binary, default_home, PHASE)?;

    if is_target(actual.as_ref(), candidate.identity()) {
        Ok(SwitchVerified::issue())
    } else {
        // Deliberately not `mismatch()`: nothing was replaced, so there is no backup to restore
        // and telling the user to restore one would be wrong. What they can do is resolve the
        // sign-in that is actually there.
        Err(TogletError::new(
            ErrorCode::SwitchVerificationMismatch,
            PHASE,
            false,
            UserAction::ResolveExternalChange,
        )
        .with_detail("the default home is not signed in as this account"))
    }
}
