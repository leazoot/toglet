//! The credential storage interface.

use super::secret::{CredentialRef, Secret};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Where encrypted credentials live.
///
/// The interface is deliberately four operations wide. There is **no export, no enumeration of
/// plaintext and no "give me everything"** - a plaintext export path is forbidden, and an
/// interface that cannot express one cannot grow one by accident.
///
/// Every implementation must fail loudly when the platform store is unavailable. Falling back
/// to writing plaintext is forbidden without exception, so there is no "best effort" variant
/// of `store`.
pub trait SecretStore {
    /// Encrypts and stores `secret`, replacing any existing entry.
    fn store(&self, reference: &CredentialRef, secret: &Secret) -> Result<()>;

    /// Decrypts the stored entry.
    fn load(&self, reference: &CredentialRef) -> Result<Secret>;

    /// Removes the entry. Removing something that is not there succeeds: the caller wanted it
    /// gone and it is gone.
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
