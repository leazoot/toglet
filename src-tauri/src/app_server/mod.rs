//! JSON-RPC over stdio against `codex app-server`: process lifecycle, handshake, method
//! calls and capability detection.
//!
//! Depends on: `credentials` (decrypt to an isolated home only), `codex_home`, `diagnostics`.
//! Must never write the default `auth.json`.
//!
//! Measured protocol constraints that the implementation must honour:
//! - Framing is NDJSON, not LSP `Content-Length`.
//! - Every request needs a timeout: a malformed frame produces no reply and no exit.
//! - The binary must be the native `codex` executable, never a PATH name or shell wrapper.
//!
//! Implemented so far: binary resolution and subprocess lifecycle, the NDJSON line transport,
//! JSON-RPC framing, DTOs and the handshake. Environment detection is not implemented yet.

mod client;
mod dto;
mod process;
mod wire;

pub use client::AppServerClient;
pub use dto::{
    CREDENTIAL_STORE_FILE, CREDENTIAL_STORE_KEY, ConfigWriteOutcome, CredentialStoreSetting,
    RawRateLimits, RawWindow,
};
pub use process::CodexBinary;
pub use wire::AppServerSession;
