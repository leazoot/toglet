//! The credential storage interface.

use super::secret::{CredentialRef, Secret};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Where stored credentials live.
///
/// Deliberately has no export or enumeration, so a plaintext export cannot be added by accident.
/// Implementations must fail loudly when unavailable and never fall back to plaintext.
pub trait SecretStore {
    /// Stores `secret`, replacing any existing entry.
    fn store(&self, reference: &CredentialRef, secret: &Secret) -> Result<()>;

    /// Loads the stored entry.
    fn load(&self, reference: &CredentialRef) -> Result<Secret>;

    /// Removes the entry; removing a missing entry succeeds.
    fn delete(&self, reference: &CredentialRef) -> Result<()>;

    fn contains(&self, reference: &CredentialRef) -> Result<bool>;
}

/// The credential store could not be used. Never a reason to store plaintext instead.
pub(crate) fn unavailable(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::CredentialStoreUnavailable,
        Phase::Storage,
        true,
        UserAction::UnlockCredentialStore,
    )
    .with_detail(detail)
}
