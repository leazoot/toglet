//! Codex home path resolution, `config.toml` editing, `auth.json` reading and watching, and
//! the shared atomic write primitive.
//!
//! Depends on: platform APIs, `diagnostics`.
//!
//! Hard constraints: `atomic_write` lives here and every persistence path in the app reuses
//! it; the temporary file is always in the target's directory; permissions precede content;
//! only `switching` may use the default-`auth.json` write path.
//!
//! Implemented so far: the isolated `CODEX_HOME`, the private-file primitives, `atomic_write`,
//! environment detection and the authentication watcher. `config.toml` is managed by
//! `codex_config`, which sits above `app_server`.

mod atomic;
mod detect;
mod isolated;
pub(crate) mod permissions;
mod watcher;

pub use atomic::{Staged, atomic_write, stage};
pub use detect::{CheckId, CheckStatus, EnvironmentCheck, EnvironmentReport, detect_environment};
pub use isolated::{IsolatedHome, ServerHome, sweep_stale};
// `create_private_dir` is public because creating the application data directory belongs to
// application startup, which lives outside this module (see `storage::MetadataStore`).
pub use permissions::{create_private_dir, is_private, open_private_append};
pub use watcher::{AuthChange, AuthWatcher};
