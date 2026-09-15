//! Credential stores (user-only files on macOS, DPAPI on Windows) and the short-lived decryption
//! home. No plaintext fallback when a store is unavailable; permissions precede content; cleanup
//! of temporary material is guaranteed by `Drop`.

mod file;
mod memory;
mod refresh;
mod secret;
mod session;
mod store;

#[cfg(windows)]
mod windows;

pub use file::FileSecretStore;
pub use memory::MemorySecretStore;
pub use refresh::{CredentialLock, WriteBack, write_back_if_refreshed};
pub use secret::{CredentialRef, Secret};
pub use session::CredentialSession;
pub use store::SecretStore;

#[cfg(windows)]
pub use windows::WindowsSecretStore;
