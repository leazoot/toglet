//! JSON-RPC over stdio against `codex app-server`. Never writes the default `auth.json`.
//!
//! - Framing is NDJSON, not LSP `Content-Length`.
//! - Every request needs a timeout: a malformed frame produces no reply and no exit.
//! - The binary must be the native `codex` executable, never a PATH name or shell wrapper.

mod client;
mod dto;
mod process;
mod thread;
mod wire;

pub use client::AppServerClient;
pub use dto::{
    CREDENTIAL_STORE_FILE, CREDENTIAL_STORE_KEY, ConfigWriteOutcome, CredentialStoreSetting,
    RawCredits, RawLimitBucket, RawRateLimits, RawResetCredits, RawWindow, ResetOutcome,
};
pub use process::CodexBinary;
pub use thread::{
    ActiveFlag, ModelInfo, ServerEvent, ThreadPage, ThreadStatus, ThreadSummary, TurnErrorKind,
    TurnRecord, TurnStatus,
};
pub use wire::AppServerSession;
