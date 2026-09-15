//! `AccountProfile` CRUD, duplicate detection, naming rules and the account state machine.
//!
//! Profiles never carry a token field; the account limit is 12; `displayName` never reaches a
//! command line or environment variable.

mod auth_file;
pub mod external_change;
pub mod fingerprint;
mod identity;
mod kind;
pub mod onboarding;
mod profile;
pub mod rate_limits;
pub mod repository;
mod status;

pub use auth_file::{AuthFacts, read as read_auth_facts};
pub use identity::AccountIdentity;
pub use kind::{AccountKind, UnsupportedReason};
pub use profile::{AccountProfile, default_display_name, mask_email, validate_display_name};
pub use status::AccountStatus;
