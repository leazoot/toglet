//! Codex home detection, `auth.json` watching, isolated homes, private-file primitives and the
//! shared atomic write. Every persistence path reuses `atomic_write`: the temporary file sits in
//! the target's directory and permissions are applied before content.

mod atomic;
mod detect;
mod isolated;
pub(crate) mod permissions;
mod watcher;

pub use atomic::{Staged, atomic_write, stage};
pub use detect::{CheckId, CheckStatus, EnvironmentCheck, EnvironmentReport, detect_environment};
pub use isolated::{IsolatedHome, ServerHome, sweep_stale};
// `create_private_dir` is public because startup creates the application data directory.
pub use permissions::{create_private_dir, is_private, open_private_append};
pub use watcher::{AuthChange, AuthWatcher};
