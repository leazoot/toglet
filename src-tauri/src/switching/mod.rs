//! Pre-checks, switch journal, atomic replacement, verification and rollback.
//!
//! Depends on: `codex_home`, `credentials`, `app_server`, `process`, `accounts`, `diagnostics`.
//! Must not depend on `quota`.
//!
//! Hard constraints: the only module allowed to write the default `auth.json`, and only
//! while holding the global switch mutex; `activeAccountId` is written only after
//! verification confirms the identity equals the target; verification compares the account
//! e-mail, because a corrupt credential and a logged-out state both return `account: null`.
//!
//! Implemented so far: the seven pre-checks with the global switch lock and the four-step
//! progress, the atomic replacement with post-switch verification, the journal with its
//! rollback and start-up recovery, adopting the session the default home already holds, and
//! signing Codex out on request.

mod adopt;
mod journal;
mod preflight;
mod recovery;
mod sign_out;
mod state;
mod swap;
mod verify;

pub use adopt::adopt_current_session;
pub use journal::{JOURNAL_FILE, RecoveryPlan, SwitchJournal, SwitchPhase};
pub use preflight::{
    ClientVerdict, Preflight, PreflightFailure, PreflightPassed, PreflightStep, SwitchGuard,
    SwitchLock, SwitchTarget, verdict,
};
pub use recovery::{RecoveryOutcome, recover};
pub use sign_out::{SignOut, SignOutFailed, SignOutPassed, SignedOut};
pub use state::{NoObserver, StepObserver, SwitchProgress, SwitchStep};
pub use swap::{
    Faults, NoFaults, RollbackReport, Switch, SwitchFailed, SwitchStage, SwitchSucceeded,
};
pub use verify::{is_same, is_target, read_default_identity};
