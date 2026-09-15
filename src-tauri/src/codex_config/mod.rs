//! Manages the one Codex setting Toglet needs through the app server's `config/*` methods.
//!
//! Used instead of editing TOML: `expectedVersion` makes the file's owner refuse stale writes,
//! `configRequirements/read` exposes organisation-enforced config, and Codex keeps the formatting.
//! Never touches `auth.json`.

mod backup;
mod manage;

pub use backup::is_toglet_backup;
pub use manage::{
    CredentialStoreOutcome, EnabledRecord, RestoreOutcome, enable_file_credential_store,
    restore_credential_store,
};
