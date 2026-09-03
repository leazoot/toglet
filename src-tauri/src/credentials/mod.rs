//! Credential store abstraction (macOS Keychain / Windows DPAPI), encryption and the
//! short-lived decryption directory.
//!
//! Depends on: platform APIs, `diagnostics`. One of the four modules allowed to contain
//! `#[cfg(target_os)]` branches, and only inside trait implementations.
//!
//! Hard constraints: no plaintext fallback when the store is unavailable; permissions are
//! set before content; cleanup of temporary material is guaranteed by `Drop`, not by the
//! caller remembering.
//!
//! Implemented so far: the `SecretStore` interface, the `Secret` and `CredentialRef` types,
//! the Windows DPAPI store and an in-memory store for tests, the macOS Keychain store (compiles
//! for the target but unverified on a real machine), and the temporary decryption session.

mod memory;
mod refresh;
mod secret;
mod session;
mod store;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

pub use memory::MemorySecretStore;
pub use refresh::{CredentialLock, WriteBack, write_back_if_refreshed};
pub use secret::{CredentialRef, Secret};
pub use session::CredentialSession;
pub use store::SecretStore;

#[cfg(target_os = "macos")]
pub use macos::MacosSecretStore;
#[cfg(windows)]
pub use windows::WindowsSecretStore;
