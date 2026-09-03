//! Discovery of running Codex clients and graceful restart requests.
//!
//! Depends on: platform APIs, `diagnostics`.
//!
//! Hard constraints: force-killing an external Codex process is never the default action;
//! only `codex app-server` may be spawned, with compile-time constant arguments; the restart
//! path may only reuse the `ExecutablePath` read from the running process before shutdown,
//! and must never hard-code a content-hash directory.
//!
//! Implemented so far: the client probe, the graceful close-and-reopen path and handing a
//! sign-in address to the user's browser.

mod browser;
mod probe;
mod restart;

pub use browser::open_url;
pub use probe::{
    ClientKind, ClientPresence, ClientProbe, RunningClient, SystemClientProbe, classify,
    is_codex_executable,
};
pub use restart::{
    ClientOutcome, ClientRestart, QuitOutcome, RestartPlan, RestartTarget, SHUTDOWN_TIMEOUT,
    SystemClientRestart, close, plan, reopen,
};
