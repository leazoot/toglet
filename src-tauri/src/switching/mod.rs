//! Pre-checks, switch journal, atomic replacement, verification and rollback.
//! The only module that writes the default `auth.json`, always under the global switch lock.
//! Must not depend on `quota`.

mod adopt;
mod journal;
mod orchestrate;
mod preflight;
mod recovery;
mod sign_out;
mod state;
mod swap;
mod verify;

pub use adopt::adopt_current_session;
pub use journal::{JOURNAL_FILE, RecoveryPlan, SwitchJournal, SwitchPhase};
pub use orchestrate::{ActiveRecord, SwitchContext, SwitchReport, perform};
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
pub use verify::{is_same, is_target, mismatch, read_default_identity};
