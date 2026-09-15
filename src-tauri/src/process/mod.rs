//! Discovery of running Codex clients and graceful restart requests.
//!
//! External Codex processes are never force-killed by default, and a restart reuses only the
//! executable path read from the running process, never a hard-coded directory.

mod browser;
mod power;
mod probe;
mod restart;

pub use browser::open_url;
pub use power::{FakePowerAssertion, PowerAssertion, PowerHold, SystemPowerAssertion};
pub use probe::{
    ClientKind, ClientPresence, ClientProbe, RunningClient, SystemClientProbe, classify,
    is_codex_executable,
};
pub use restart::{
    ClientOutcome, ClientRestart, QuitOutcome, RestartPlan, RestartTarget, SHUTDOWN_TIMEOUT,
    SystemClientRestart, close, plan, reopen,
};
