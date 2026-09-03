//! Non-sensitive local metadata and settings.
//!
//! Depends on: `codex_home` (for `atomic_write`), `diagnostics`.
//!
//! Hard constraints: no tokens, no `auth.json` content, no full e-mail addresses, no
//! absolute paths, no keys - the credential store is linked only by `credentialRef`;
//! every write carries a `schemaVersion` and goes through `atomic_write`.
//!
//! Implemented so far: the single-JSON metadata document with schema versioning and rebuild
//! on corruption, and `AppSettings`.

mod document;
pub mod settings;
mod store;

pub use document::{
    CURRENT_SCHEMA_VERSION, CodexConfigState, LoadProblem, MetadataDocument, migrate,
};
pub use settings::{AppSettings, DockEdge, Language, SwitchVerified, Theme};
pub use store::{LoadOutcome, MetadataStore};
