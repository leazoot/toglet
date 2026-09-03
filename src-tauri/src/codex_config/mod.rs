//! Managing the one Codex setting Toglet needs, through Codex's own interface.
//!
//! Depends on: `app_server`, `codex_home`, `diagnostics`.
//!
//! This module sits above `app_server` rather than inside `codex_home` because it calls the
//! runtime, and `app_server` already depends on `codex_home` - putting it there would make the
//! dependency graph circular.
//!
//! **Why the runtime and not a TOML editor.** An earlier design called for editing
//! `config.toml` with `toml_edit`. Measurement showed the app server exposes `config/read`,
//! `config/value/write` and `configRequirements/read`, and that they give three things a
//! hand-written editor cannot:
//!
//! * `expectedVersion` - the process that owns the file refuses a stale write itself, so there
//!   is no window between Toglet checking and Toglet writing;
//! * `configRequirements/read` and `configLayerReadonly` - an organisation-enforced
//!   configuration becomes observable, which is required and which nothing else provided;
//! * comment and format preservation as Codex's responsibility, verified rather than
//!   reimplemented.
//!
//! It also adds no dependency. The cost is that changing the setting needs a running app
//! server, and a runtime without these methods fails honestly rather than falling back.
//!
//! **This module never touches `auth.json`.** Starting and stopping management change one
//! configuration key; the credentials Codex is currently using are not Toglet's to delete.
//! Removing an account is a separate, explicitly confirmed action.

mod backup;
mod manage;

pub use backup::is_toglet_backup;
pub use manage::{
    CredentialStoreOutcome, EnabledRecord, RestoreOutcome, enable_file_credential_store,
    restore_credential_store,
};
