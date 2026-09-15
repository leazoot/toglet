//! Tauri command boundary: the only surface exposed to the frontend.
//! No general "run any command" or "read any path" command; return values never carry tokens,
//! `auth.json` content, full e-mail addresses, absolute paths or command lines.

pub mod accounts;
pub mod autorun;
pub mod environment;
pub mod notify;
pub mod onboarding;
pub mod remote;
pub mod remote_poll;
pub mod settings;
pub mod state;
pub mod switching;
pub mod views;
pub mod window;

pub use state::AppState;
