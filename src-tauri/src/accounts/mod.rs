//! `AccountProfile` CRUD, duplicate detection, naming rules and the account state machine.
//!
//! Depends on: `credentials`, `storage`, `diagnostics`.
//!
//! Hard constraints: profiles never carry a token field; the account limit is 12; `displayName`
//! never reaches a command line or environment variable.
//!
//! Implemented so far: `AccountIdentity`, `AccountProfile` with name validation and CRUD,
//! irreversible fingerprints and duplicate detection, the nine state account state machine, the
//! four onboarding paths - import, sign-in, re-authentication and removal - and reconciliation
//! with authentication changes Codex made on its own.

mod auth_file;
pub mod external_change;
pub mod fingerprint;
mod identity;
mod kind;
pub mod onboarding;
mod profile;
pub mod repository;
mod status;

pub use auth_file::AuthFacts;
pub use identity::AccountIdentity;
pub use kind::{AccountKind, UnsupportedReason};
pub use profile::{AccountProfile, default_display_name, mask_email, validate_display_name};
pub use status::AccountStatus;
