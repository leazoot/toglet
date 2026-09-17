//! Automatic account scheduling and task continuation, off by default.
//!
//! Invariants: only ticked accounts and the bound thread; only a machine-readable usage-limit
//! failure triggers; one continuation per exhaustion; stale generations are dropped; every
//! switch goes through `switching`.

pub mod availability;
pub mod driver;
pub mod executor;
pub mod machine;
pub mod plan;
pub mod ports;
pub mod takeover;

pub use availability::{
    AccountFacts, Blocked, Blocker, Candidate, Reason, Selection, Verdict, assess, choose,
};
pub use driver::{
    ACTIVE_SLICE, Clock, DriverConfig, DriverHandle, IDLE_SLICE, Observation, Observer, PlanEdit,
    Ports, Resumed, SystemClock, Verification, WATCH_INTERVAL_SECONDS, parse_rfc3339, rfc3339,
    spawn,
};
pub use executor::{Continuation, Executor};
pub use machine::{
    Action, Dedup, Exhausted, Fact, Ignored, Interruption, LastResult, Machine, Outcome, Restored,
    ResultKind, State, UserEvent, WaitReason,
};
pub use plan::{
    AutoRunPlan, Binding, ExecutionEnvironment, LastResultRecord, PLAN_FILE, Participant, PlanStore,
};
pub use ports::{ActiveAccountRecord, AppPorts, ParticipantRecord, Services};
pub use takeover::Takeover;
