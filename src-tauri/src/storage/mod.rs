//! Non-sensitive local metadata and settings.
//!
//! No tokens, `auth.json` content, full e-mail addresses or absolute paths; credentials are
//! linked only by `credentialRef`, and every write goes through `atomic_write`.

mod document;
pub mod settings;
mod store;

pub use document::{
    CURRENT_SCHEMA_VERSION, CodexConfigState, LoadProblem, MetadataDocument, migrate,
};
pub use settings::{AppSettings, DockEdge, DockShape, Language, SwitchVerified, Theme};
pub use store::{LoadOutcome, MetadataStore, read_schema_version};
