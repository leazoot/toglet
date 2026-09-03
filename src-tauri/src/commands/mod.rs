//! Tauri command boundary: the only surface exposed to the frontend.
//!
//! Depends on: every business module. Nothing depends on it.
//!
//! Hard constraints: no general-purpose "run any command" or "read any path" command may
//! ever be added; return values never contain tokens, `auth.json` content, full e-mail
//! addresses, absolute paths or command lines; input is validated at this boundary.

pub mod accounts;
pub mod environment;
pub mod onboarding;
pub mod settings;
pub mod state;
pub mod switching;
pub mod views;
pub mod window;

pub use state::AppState;
