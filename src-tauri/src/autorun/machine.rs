//! The automatic-continuation state machine: a pure transition function from facts to the next
//! state and one driver action. It never reads a clock.
//!
//! Every accepted event bumps `generation`; a fact from an older generation is dropped, so a late
//! result cannot restart anything. User events carry no generation. One exhaustion, one
//! continuation.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::app_server::{TurnErrorKind, TurnRecord, TurnStatus};
use crate::diagnostics::ErrorCode;
use crate::quota::Backoff;

use super::availability::{Blocker, Selection, Verdict};

/// Added to an expected recovery time before the account is re-read: a limit that "resets at
/// 03:00" is not reliably open at 03:00:00.
pub const WAIT_BUFFER_SECONDS: i64 = 90;

/// Default cap on continuations per run.
pub const DEFAULT_MAX_RESUMES: u32 = 8;

/// Delay before re-checking accounts blocked only by things that lift with time but without a
/// server-given recovery time. Not a failure, so it does not count against
/// [`MAX_CONSECUTIVE_QUERY_FAILURES`].
pub const BLOCKED_RECHECK_SECONDS: i64 = 900;

/// Initial retry delay when the session was busy. Doubles, capped, and counts against
/// [`MAX_CONSECUTIVE_QUERY_FAILURES`] so a CLI session left open overnight reaches a person.
pub const SESSION_BUSY_BASE: Duration = Duration::from_secs(30);

/// Consecutive failed queries (quota or thread) before the machine asks a person; with
/// `Backoff` this is about two hours of continuous failure.
pub const MAX_CONSECUTIVE_QUERY_FAILURES: u32 = 12;

/// The states, spelled as the plan file spells them: the serde form and
/// [`as_str`](Self::as_str) agree, and a test keeps them agreeing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Disabled,
    Armed,
    Selecting,
    WaitingQuota,
    Verifying,
    Switching,
    Resuming,
    Running,
    WaitingNetwork,
    RoundCompleted,
    NeedsHuman,
    Paused,
    Stopped,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Armed => "armed",
            Self::Selecting => "selecting",
            Self::WaitingQuota => "waiting_quota",
            Self::Verifying => "verifying",
            Self::Switching => "switching",
            Self::Resuming => "resuming",
            Self::Running => "running",
            Self::WaitingNetwork => "waiting_network",
            Self::RoundCompleted => "round_completed",
            Self::NeedsHuman => "needs_human",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }

    /// States in which the driver has something scheduled or in flight. The others wait for
    /// the user.
    pub fn is_active(self) -> bool {
        !matches!(
            self,
            Self::Disabled | Self::Paused | Self::Stopped | Self::RoundCompleted | Self::NeedsHuman
        )
    }
}

/// How a turn ended. Only `Exhausted` triggers anything; waiting on a person is a thread
/// status, see [`Fact::WaitingOnHuman`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interruption {
    /// `failed` with `usageLimitExceeded`.
    Exhausted,
    /// A transport failure or an overloaded server. Retried on the same account.
    Network,
    /// The credentials were refused.
    AuthExpired,
    /// `interrupted`: somebody stopped it.
    UserStopped,
    /// `completed`. Whether the task is done is for the user to judge.
    Completed,
    /// Failed for a reason nobody retries automatically.
    Failed(TurnErrorKind),
    /// `failed` without a machine-readable reason.
    FailedUnknownReason,
    /// A status this build cannot read.
    StatusUnknown,
}

impl Interruption {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exhausted => "usage_limit_exceeded",
            Self::Network => "network",
            Self::AuthExpired => "unauthorized",
            Self::UserStopped => "turn_interrupted",
            Self::Completed => "completed",
            Self::Failed(kind) => kind.as_str(),
            Self::FailedUnknownReason => "turn_failed_unknown_reason",
            Self::StatusUnknown => "turn_status_unknown",
        }
    }

    fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "usage_limit_exceeded" => Self::Exhausted,
            "network" => Self::Network,
            "unauthorized" => Self::AuthExpired,
            "turn_interrupted" => Self::UserStopped,
            "completed" => Self::Completed,
            "turn_failed_unknown_reason" => Self::FailedUnknownReason,
            "turn_status_unknown" => Self::StatusUnknown,
            other => Self::Failed(TurnErrorKind::parse(other)?),
        })
    }
}

/// Why the machine is waiting, paused, stopped, or asking for a person: a stable code, never prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    /// Waiting on a quota blocker of the target account.
    Blocked(Blocker),
    /// A switch or a resume failed with this code.
    Error(ErrorCode),
    /// The bound turn ended this way.
    Turn(Interruption),
    /// The thread is waiting on an approval or an answer only a person can give.
    WaitingOnHuman,
    /// No participant is usable and none has an expected time.
    NoAccountAvailable,
    /// Consecutive queries failed up to the cap.
    QueryFailures,
    /// The user switched accounts by hand while automatic mode was waiting.
    ManualSwitch,
    /// The user opened the desktop app while automatic mode was waiting.
    DesktopReopened,
    MaxResumes,
    Deadline,
    /// Toglet was restarted while a cycle was in flight. No longer produced (a restart re-arms,
    /// see [`Machine::restore`]); kept so older plan files still read back and can be cleared.
    AppRestarted,
}

impl WaitReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blocked(blocker) => blocker.as_str(),
            Self::Error(code) => code.as_str(),
            Self::Turn(interruption) => interruption.as_str(),
            Self::WaitingOnHuman => "waiting_on_human",
            Self::NoAccountAvailable => "no_account_available",
            Self::QueryFailures => "query_failures",
            Self::ManualSwitch => "manual_switch",
            Self::DesktopReopened => "desktop_reopened",
            Self::MaxResumes => "max_resumes",
            Self::Deadline => "deadline",
            Self::AppRestarted => "app_restarted",
        }
    }

    /// The inverse of [`as_str`](Self::as_str), for a reason read back from the plan file.
    /// Ambiguity is resolved the way the codes are produced: a fixed word first, then a
    /// blocker, then an error code, then a turn class.
    pub fn parse(code: &str) -> Option<Self> {
        let fixed = match code {
            "waiting_on_human" => Some(Self::WaitingOnHuman),
            "no_account_available" => Some(Self::NoAccountAvailable),
            "query_failures" => Some(Self::QueryFailures),
            "manual_switch" => Some(Self::ManualSwitch),
            "desktop_reopened" => Some(Self::DesktopReopened),
            "max_resumes" => Some(Self::MaxResumes),
            "deadline" => Some(Self::Deadline),
            "app_restarted" => Some(Self::AppRestarted),
            _ => None,
        };
        fixed
            .or_else(|| Blocker::parse(code).map(Self::Blocked))
            .or_else(|| ErrorCode::parse(code).map(Self::Error))
            .or_else(|| Interruption::parse(code).map(Self::Turn))
    }
}

/// Something that happened, reported by the driver with the generation it was issued under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    /// The bound thread's last turn has ended. From `thread/read` while armed or checking, from
    /// `turn/completed` while running.
    TurnEnded {
        turn_id: String,
        interruption: Interruption,
    },
    /// The bound thread's last turn is still in progress.
    TurnInProgress,
    /// The thread is waiting on an approval or user input.
    WaitingOnHuman,
    /// `choose` ran over fresh facts of every participant.
    Chosen(Selection),
    /// The target account's quota was read again, fresh.
    Verified {
        verdict: Verdict,
        /// The account Codex is signed in as right now, if any.
        active_account_id: Option<String>,
    },
    /// A quota or thread read failed.
    QueryFailed,
    /// The timer the driver set for this generation fired.
    WaitElapsed,
    /// `switching` verified the switch to this account.
    Switched {
        account_id: String,
    },
    SwitchFailed {
        code: ErrorCode,
    },
    /// `turn/start` was accepted; this is the new turn.
    Resumed {
        turn_id: String,
    },
    ResumeFailed {
        code: ErrorCode,
    },
    /// The user switched accounts by hand.
    ManualSwitchObserved,
    /// The user opened the desktop app.
    DesktopReopened,
    /// The user's deadline has passed.
    DeadlineReached,
}

impl Fact {
    /// What a thread's last turn says, as a fact. `None` for the reason on a failed turn is
    /// reported as unknown, not guessed.
    pub fn from_turn(turn: &TurnRecord) -> Self {
        let interruption = match turn.status {
            TurnStatus::InProgress => return Self::TurnInProgress,
            TurnStatus::Completed => Interruption::Completed,
            TurnStatus::Interrupted => Interruption::UserStopped,
            TurnStatus::Unknown => Interruption::StatusUnknown,
            TurnStatus::Failed => match turn.error {
                Some(TurnErrorKind::UsageLimitExceeded) => Interruption::Exhausted,
                Some(TurnErrorKind::Network { .. } | TurnErrorKind::ServerOverloaded) => {
                    Interruption::Network
                }
                Some(TurnErrorKind::Unauthorized) => Interruption::AuthExpired,
                Some(kind) => Interruption::Failed(kind),
                None => Interruption::FailedUnknownReason,
            },
        };
        Self::TurnEnded {
            turn_id: turn.id.clone(),
            interruption,
        }
    }
}

/// What the user did. Carries no generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserEvent {
    /// Enable and confirm (the plan is complete; the command layer checked that).
    Enable,
    Pause,
    /// Continue after a pause, a completed round, a stop or a `needs_human`.
    Resume,
    Cancel,
}

/// Whose turn the thread reported as run out, for the selection that follows.
///
/// The turn record does not say which account ran it, so this is only claimed where Toglet
/// knows: a turn it started, or before the first continuation, the signed-in account. Otherwise
/// the quota windows decide alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exhausted {
    /// The account Codex is signed in as when the selection is made.
    ActiveAccount,
    Account(String),
}

/// The one thing the driver does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing to start; wait for the next fact.
    Wait,
    /// Watch the bound thread's last turn and report how it ends.
    WatchThread,
    /// Gather every participant's facts, `choose`, report the selection. `exhausted` is whose
    /// turn the thread reported as run out, when that is known - see [`Exhausted`].
    Select { exhausted: Option<Exhausted> },
    /// Read this account's quota fresh and report the verdict.
    VerifyQuota { account_id: String },
    /// Set a timer for this unix time (buffer included) and report `WaitElapsed`.
    WaitUntil { at: i64 },
    /// Wait this long and report `WaitElapsed`.
    WaitFor { delay: Duration },
    /// Read the bound thread and report its last turn.
    ReadThread,
    /// Switch to this account through `switching`.
    Switch { account_id: String },
    /// Resume the bound thread on this account and start the one continuation turn.
    Resume { account_id: String },
    /// Tell the user; nothing is scheduled.
    Notify,
}

/// Answers from a switch or a continuation that mean "something else is using the session
/// right now", as opposed to "this will not work". Each one lifts by itself: a CLI or editor
/// session somebody has open ends, a desktop app that ignored the quit request is quit, a
/// switch already running finishes. Everything else still goes in front of a person.
fn session_busy(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::ClientRunning | ErrorCode::ClientShutdownTimeout | ErrorCode::SwitchInProgress
    )
}

/// Why an event was not acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// Not meaningful in the current state.
    NotApplicable,
    /// A turn end that is not an exhaustion while armed: nothing to do.
    NotATrigger,
    /// This exhausted turn was already continued once.
    AlreadyResumed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Applied(Action),
    /// Issued for a generation that has passed. Dropped.
    Stale {
        current: u64,
    },
    Ignored(Ignored),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultKind {
    Resumed,
    Switched,
    Waited,
    Failed,
    Completed,
}

impl ResultKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resumed => "resumed",
            Self::Switched => "switched",
            Self::Waited => "waited",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }
}

/// The last notable thing that happened: the plan's `lastResult`, minus the timestamp the plan
/// adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastResult {
    pub kind: ResultKind,
    pub account_id: Option<String>,
    pub turn_id: Option<String>,
    /// A stable code, never a message.
    pub code: Option<String>,
}

/// What the plan file holds of a machine, handed back to [`Machine::restore`] on start-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub state: State,
    pub generation: u64,
    pub resume_count: u32,
    pub max_resumes: Option<u32>,
    pub executing_account_id: Option<String>,
    pub wait_reason: Option<WaitReason>,
    pub dedup: Dedup,
    pub last_result: Option<LastResult>,
}

/// The plan file's `dedup` record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dedup {
    /// The exhausted turn the current cycle is about.
    pub last_observed_turn_id: Option<String>,
    /// The exhausted turn a continuation was already started for.
    pub last_resumed_turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    state: State,
    generation: u64,
    resume_count: u32,
    max_resumes: Option<u32>,
    executing_account_id: Option<String>,
    target_account_id: Option<String>,
    /// The turn the last continuation started, and the account it ran as: what lets an
    /// exhaustion of that turn be pinned on the right account. Not persisted; after a restart
    /// the windows decide.
    last_resumed_by: Option<(String, String)>,
    /// Whose turn ran out in the cycle under way, or `None` when that is not known.
    exhausted: Option<Exhausted>,
    wait_reason: Option<WaitReason>,
    expected_available_at: Option<i64>,
    backoff: Backoff,
    dedup: Dedup,
    last_result: Option<LastResult>,
}

impl Machine {
    /// Disabled, generation 0. `None` for `max_resumes` means no cap.
    pub fn new(max_resumes: Option<u32>) -> Self {
        Self {
            state: State::Disabled,
            generation: 0,
            resume_count: 0,
            max_resumes,
            executing_account_id: None,
            target_account_id: None,
            last_resumed_by: None,
            exhausted: None,
            wait_reason: None,
            expected_available_at: None,
            backoff: Backoff::new(),
            dedup: Dedup::default(),
            last_result: None,
        }
    }

    /// A machine as the plan file last saw it.
    ///
    /// A state with something in flight comes back **armed**, which re-reads the bound thread and
    /// the accounts before acting; a state waiting for the user comes back as it was. `dedup`,
    /// `resume_count` and the deadline survive, so no turn is continued twice.
    pub fn restore(restored: Restored) -> Self {
        let mut machine = Self {
            state: restored.state,
            generation: restored.generation,
            resume_count: restored.resume_count,
            max_resumes: restored.max_resumes,
            executing_account_id: restored.executing_account_id,
            target_account_id: None,
            last_resumed_by: None,
            exhausted: None,
            wait_reason: restored.wait_reason,
            expected_available_at: None,
            backoff: Backoff::new(),
            dedup: restored.dedup,
            last_result: restored.last_result,
        };
        // `paused` + `app_restarted` is what older builds wrote on every restart; re-arm that too.
        let restarted =
            machine.state == State::Paused && machine.wait_reason == Some(WaitReason::AppRestarted);
        if machine.state.is_active() || restarted {
            machine.arm();
        }
        machine
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn resume_count(&self) -> u32 {
        self.resume_count
    }

    pub fn max_resumes(&self) -> Option<u32> {
        self.max_resumes
    }

    /// The user changed the cap. Takes effect at the next exhaustion; a run already past the
    /// new cap stops then, not now.
    pub fn set_max_resumes(&mut self, max_resumes: Option<u32>) {
        self.max_resumes = max_resumes;
    }

    /// The account the bound thread is running or about to run on.
    pub fn executing_account_id(&self) -> Option<&str> {
        self.executing_account_id.as_deref()
    }

    /// The account being waited for or verified.
    pub fn target_account_id(&self) -> Option<&str> {
        self.target_account_id.as_deref()
    }

    pub fn wait_reason(&self) -> Option<WaitReason> {
        self.wait_reason
    }

    /// When the target is expected to be usable. `None` is unknown, never 0.
    pub fn expected_available_at(&self) -> Option<i64> {
        self.expected_available_at
    }

    /// When the target is next read: the expected time plus the buffer, only while waiting
    /// for a known time.
    pub fn next_check_at(&self) -> Option<i64> {
        match self.state {
            State::WaitingQuota => self
                .expected_available_at
                .map(|at| at.saturating_add(WAIT_BUFFER_SECONDS)),
            _ => None,
        }
    }

    pub fn dedup(&self) -> &Dedup {
        &self.dedup
    }

    pub fn last_result(&self) -> Option<&LastResult> {
        self.last_result.as_ref()
    }

    /// Applies a fact issued under `generation`.
    pub fn apply_fact(&mut self, generation: u64, fact: Fact) -> Outcome {
        if generation != self.generation {
            return Outcome::Stale {
                current: self.generation,
            };
        }
        // Facts that mean the same thing in every state come first.
        match fact {
            Fact::DeadlineReached => {
                return if matches!(self.state, State::Disabled | State::Stopped) {
                    Outcome::Ignored(Ignored::NotApplicable)
                } else {
                    self.stop(WaitReason::Deadline)
                };
            }
            Fact::ManualSwitchObserved if self.state.is_active() => {
                return self.pause(Some(WaitReason::ManualSwitch));
            }
            Fact::DesktopReopened if self.state.is_active() => {
                return self.pause(Some(WaitReason::DesktopReopened));
            }
            _ => {}
        }
        match self.state {
            State::Armed => self.armed(fact),
            State::Selecting => self.selecting(fact),
            State::WaitingQuota => self.waiting_quota(fact),
            State::Verifying => self.verifying(fact),
            State::Switching => self.switching(fact),
            State::Resuming => self.resuming(fact),
            State::Running => self.running(fact),
            State::WaitingNetwork => self.waiting_network(fact),
            State::Disabled
            | State::Paused
            | State::Stopped
            | State::RoundCompleted
            | State::NeedsHuman => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    pub fn apply_user(&mut self, event: UserEvent) -> Outcome {
        match (event, self.state) {
            (UserEvent::Enable, State::Disabled) => {
                // Enabling starts a run: the count belongs to the run.
                self.resume_count = 0;
                self.dedup = Dedup::default();
                self.last_result = None;
                self.arm()
            }
            (UserEvent::Enable, _) => Outcome::Ignored(Ignored::NotApplicable),
            (UserEvent::Pause, State::Disabled | State::Paused | State::Stopped) => {
                Outcome::Ignored(Ignored::NotApplicable)
            }
            (UserEvent::Pause, _) => self.pause(None),
            (
                UserEvent::Resume,
                State::Paused | State::RoundCompleted | State::Stopped | State::NeedsHuman,
            ) => self.arm(),
            (UserEvent::Resume, _) => Outcome::Ignored(Ignored::NotApplicable),
            (UserEvent::Cancel, State::Disabled) => Outcome::Ignored(Ignored::NotApplicable),
            (UserEvent::Cancel, _) => {
                self.clear_cycle();
                self.executing_account_id = None;
                self.last_resumed_by = None;
                self.exhausted = None;
                self.transition(State::Disabled, None, Action::Wait)
            }
        }
    }

    fn armed(&mut self, fact: Fact) -> Outcome {
        match fact {
            Fact::TurnEnded {
                turn_id,
                interruption: Interruption::Exhausted,
            } => self.exhausted(turn_id),
            Fact::TurnEnded { .. } | Fact::TurnInProgress => Outcome::Ignored(Ignored::NotATrigger),
            Fact::WaitingOnHuman => self.needs_human(WaitReason::WaitingOnHuman, None),
            _ => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    fn selecting(&mut self, fact: Fact) -> Outcome {
        let Fact::Chosen(selection) = fact else {
            return Outcome::Ignored(Ignored::NotApplicable);
        };
        match selection {
            Selection::Now { account_id, .. } => {
                self.backoff = self.backoff.after_success();
                self.target_account_id = Some(account_id.clone());
                self.transition(State::Verifying, None, Action::VerifyQuota { account_id })
            }
            Selection::Later {
                account_id,
                available_at,
                blockers,
                ..
            } => {
                self.backoff = self.backoff.after_success();
                self.target_account_id = Some(account_id.clone());
                self.expected_available_at = Some(available_at);
                self.last_result = Some(LastResult {
                    kind: ResultKind::Waited,
                    account_id: Some(account_id),
                    turn_id: None,
                    code: blockers.first().map(|blocker| blocker.as_str().to_owned()),
                });
                self.transition(
                    State::WaitingQuota,
                    blockers.first().copied().map(WaitReason::Blocked),
                    Action::WaitUntil {
                        at: available_at.saturating_add(WAIT_BUFFER_SECONDS),
                    },
                )
            }
            Selection::Recheck { .. } => {
                self.target_account_id = None;
                self.expected_available_at = None;
                self.after_query_failure(State::WaitingQuota, Some(Blocker::QuotaUnknown))
            }
            Selection::LaterUnknown {
                account_id,
                blockers,
                ..
            } => {
                self.backoff = self.backoff.after_success();
                // No target: the wait is not for this account in particular, it is for the
                // next look at all of them. `waiting_quota` with no target selects again.
                self.target_account_id = None;
                self.expected_available_at = None;
                self.last_result = Some(LastResult {
                    kind: ResultKind::Waited,
                    account_id: Some(account_id),
                    turn_id: None,
                    code: blockers.first().map(|blocker| blocker.as_str().to_owned()),
                });
                self.transition(
                    State::WaitingQuota,
                    blockers.first().copied().map(WaitReason::Blocked),
                    Action::WaitFor {
                        delay: Duration::from_secs(
                            u64::try_from(BLOCKED_RECHECK_SECONDS).unwrap_or(900),
                        ),
                    },
                )
            }
            Selection::NeedsHuman => self.needs_human(WaitReason::NoAccountAvailable, None),
        }
    }

    fn waiting_quota(&mut self, fact: Fact) -> Outcome {
        match fact {
            Fact::WaitElapsed => match self.target_account_id.clone() {
                Some(account_id) => {
                    self.transition(State::Verifying, None, Action::VerifyQuota { account_id })
                }
                None => self.transition(
                    State::Selecting,
                    None,
                    Action::Select {
                        exhausted: self.exhausted.clone(),
                    },
                ),
            },
            _ => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    fn verifying(&mut self, fact: Fact) -> Outcome {
        let Some(target) = self.target_account_id.clone() else {
            return Outcome::Ignored(Ignored::NotApplicable);
        };
        match fact {
            Fact::Verified {
                verdict: Verdict::Available,
                active_account_id,
            } => {
                self.backoff = self.backoff.after_success();
                self.expected_available_at = None;
                if active_account_id.as_deref() == Some(target.as_str()) {
                    self.executing_account_id = Some(target.clone());
                    self.transition(State::Resuming, None, Action::Resume { account_id: target })
                } else {
                    self.transition(
                        State::Switching,
                        None,
                        Action::Switch { account_id: target },
                    )
                }
            }
            Fact::Verified {
                verdict: Verdict::Blocked(blocked),
                ..
            } => {
                let first = blocked.blockers.first().copied();
                match blocked.expected_available_at {
                    Some(at) => {
                        self.backoff = self.backoff.after_success();
                        self.expected_available_at = Some(at);
                        self.transition(
                            State::WaitingQuota,
                            first.map(WaitReason::Blocked),
                            Action::WaitUntil {
                                at: at.saturating_add(WAIT_BUFFER_SECONDS),
                            },
                        )
                    }
                    None if blocked.is_recheckable() => {
                        self.expected_available_at = None;
                        self.after_query_failure(State::WaitingQuota, first)
                    }
                    // Blocked for good with no time: this account is out; somebody else may
                    // do. Straight back to choosing rather than waiting on nothing.
                    None => {
                        self.backoff = self.backoff.after_success();
                        self.target_account_id = None;
                        self.expected_available_at = None;
                        self.transition(
                            State::Selecting,
                            None,
                            Action::Select {
                                exhausted: self.exhausted.clone(),
                            },
                        )
                    }
                }
            }
            // A fresh read cannot be stale; treat a claim that it is as a failed read.
            Fact::Verified {
                verdict: Verdict::Recheck,
                ..
            }
            | Fact::QueryFailed => self.after_query_failure(State::Verifying, None),
            Fact::WaitElapsed => self.transition(
                State::Verifying,
                None,
                Action::VerifyQuota { account_id: target },
            ),
            _ => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    fn switching(&mut self, fact: Fact) -> Outcome {
        match fact {
            Fact::Switched { account_id } => {
                self.executing_account_id = Some(account_id.clone());
                self.last_result = Some(LastResult {
                    kind: ResultKind::Switched,
                    account_id: Some(account_id.clone()),
                    turn_id: None,
                    code: None,
                });
                self.transition(State::Resuming, None, Action::Resume { account_id })
            }
            Fact::SwitchFailed { code } if session_busy(code) => {
                self.retry_busy(State::Switching, code)
            }
            Fact::SwitchFailed { code } => {
                self.needs_human(WaitReason::Error(code), Some(code.as_str()))
            }
            Fact::WaitElapsed => match self.target_account_id.clone() {
                Some(account_id) => self.transition(
                    State::Switching,
                    self.wait_reason,
                    Action::Switch { account_id },
                ),
                // The target was cleared under us; choose again rather than switch to nobody.
                None => self.transition(
                    State::Selecting,
                    None,
                    Action::Select {
                        exhausted: self.exhausted.clone(),
                    },
                ),
            },
            _ => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    fn resuming(&mut self, fact: Fact) -> Outcome {
        match fact {
            Fact::Resumed { turn_id } => {
                self.resume_count = self.resume_count.saturating_add(1);
                self.dedup.last_resumed_turn_id = self.dedup.last_observed_turn_id.clone();
                self.last_resumed_by = self
                    .executing_account_id
                    .clone()
                    .map(|account| (turn_id.clone(), account));
                self.exhausted = None;
                self.backoff = self.backoff.after_success();
                self.target_account_id = None;
                self.last_result = Some(LastResult {
                    kind: ResultKind::Resumed,
                    account_id: self.executing_account_id.clone(),
                    turn_id: Some(turn_id),
                    code: None,
                });
                self.transition(State::Running, None, Action::Wait)
            }
            Fact::ResumeFailed { code } if session_busy(code) => {
                self.retry_busy(State::Resuming, code)
            }
            Fact::ResumeFailed { code } => {
                self.needs_human(WaitReason::Error(code), Some(code.as_str()))
            }
            Fact::WaitElapsed => match self.executing_account_id.clone() {
                Some(account_id) => self.transition(
                    State::Resuming,
                    self.wait_reason,
                    Action::Resume { account_id },
                ),
                None => self.transition(
                    State::Selecting,
                    None,
                    Action::Select {
                        exhausted: self.exhausted.clone(),
                    },
                ),
            },
            // Somebody else's turn is running on the thread. Nothing is started; the executor
            // session listens for how it ends, and the machine acts on that.
            Fact::TurnInProgress => {
                self.backoff = self.backoff.after_success();
                self.transition(State::Running, None, Action::Wait)
            }
            Fact::WaitingOnHuman => self.needs_human(WaitReason::WaitingOnHuman, None),
            _ => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    fn running(&mut self, fact: Fact) -> Outcome {
        match fact {
            Fact::TurnEnded {
                turn_id,
                interruption,
            } => self.turn_ended(turn_id, interruption),
            Fact::WaitingOnHuman => self.needs_human(WaitReason::WaitingOnHuman, None),
            _ => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    fn waiting_network(&mut self, fact: Fact) -> Outcome {
        match fact {
            Fact::WaitElapsed => self.transition(State::WaitingNetwork, None, Action::ReadThread),
            Fact::QueryFailed => self.after_query_failure(State::WaitingNetwork, None),
            Fact::TurnInProgress => {
                self.backoff = self.backoff.after_success();
                self.transition(State::Running, None, Action::Wait)
            }
            Fact::TurnEnded {
                turn_id,
                interruption: Interruption::Network,
            } => {
                // The continuation itself failed on the network: retry on the same account,
                // bounded by the query failure cap.
                if self.backoff.failures() >= MAX_CONSECUTIVE_QUERY_FAILURES {
                    return self.needs_human(WaitReason::QueryFailures, Some("network"));
                }
                self.dedup.last_observed_turn_id = Some(turn_id);
                match self.executing_account_id.clone() {
                    Some(account_id) => {
                        self.transition(State::Resuming, None, Action::Resume { account_id })
                    }
                    None => self.needs_human(WaitReason::Turn(Interruption::Network), None),
                }
            }
            Fact::TurnEnded {
                turn_id,
                interruption,
            } => self.turn_ended(turn_id, interruption),
            Fact::WaitingOnHuman => self.needs_human(WaitReason::WaitingOnHuman, None),
            _ => Outcome::Ignored(Ignored::NotApplicable),
        }
    }

    /// A turn of the bound thread ended while it was ours (running or being checked).
    fn turn_ended(&mut self, turn_id: String, interruption: Interruption) -> Outcome {
        match interruption {
            Interruption::Exhausted => self.exhausted(turn_id),
            Interruption::Completed => {
                self.last_result = Some(LastResult {
                    kind: ResultKind::Completed,
                    account_id: self.executing_account_id.clone(),
                    turn_id: Some(turn_id),
                    code: None,
                });
                self.transition(State::RoundCompleted, None, Action::Notify)
            }
            Interruption::Network => {
                self.backoff = self.backoff.after_failure();
                if self.backoff.failures() >= MAX_CONSECUTIVE_QUERY_FAILURES {
                    return self.needs_human(WaitReason::QueryFailures, Some("network"));
                }
                self.transition(
                    State::WaitingNetwork,
                    Some(WaitReason::Turn(Interruption::Network)),
                    Action::WaitFor {
                        delay: self.backoff.delay(),
                    },
                )
            }
            Interruption::UserStopped => {
                self.pause(Some(WaitReason::Turn(Interruption::UserStopped)))
            }
            Interruption::AuthExpired
            | Interruption::Failed(_)
            | Interruption::FailedUnknownReason
            | Interruption::StatusUnknown => {
                self.needs_human(WaitReason::Turn(interruption), Some(interruption.as_str()))
            }
        }
    }

    /// The one trigger.
    fn exhausted(&mut self, turn_id: String) -> Outcome {
        if self.dedup.last_resumed_turn_id.as_deref() == Some(turn_id.as_str()) {
            return Outcome::Ignored(Ignored::AlreadyResumed);
        }
        if let Some(max) = self.max_resumes
            && self.resume_count >= max
        {
            return self.stop(WaitReason::MaxResumes);
        }
        self.exhausted = match (&self.last_resumed_by, &self.executing_account_id) {
            (Some((started, account)), _) if *started == turn_id => {
                Some(Exhausted::Account(account.clone()))
            }
            (_, None) => Some(Exhausted::ActiveAccount),
            _ => None,
        };
        self.dedup.last_observed_turn_id = Some(turn_id);
        self.backoff = self.backoff.after_success();
        self.target_account_id = None;
        self.expected_available_at = None;
        self.transition(
            State::Selecting,
            None,
            Action::Select {
                exhausted: self.exhausted.clone(),
            },
        )
    }

    /// One more consecutive failed query: back off in `next`, or give up at the cap.
    fn after_query_failure(&mut self, next: State, blocker: Option<Blocker>) -> Outcome {
        self.backoff = self.backoff.after_failure();
        if self.backoff.failures() >= MAX_CONSECUTIVE_QUERY_FAILURES {
            return self.needs_human(WaitReason::QueryFailures, blocker.map(Blocker::as_str));
        }
        self.transition(
            next,
            blocker.map(WaitReason::Blocked),
            Action::WaitFor {
                delay: self.backoff.delay(),
            },
        )
    }

    fn arm(&mut self) -> Outcome {
        self.clear_cycle();
        self.transition(State::Armed, None, Action::WatchThread)
    }

    /// The session was busy, not broken: retry the same step after an escalating wait, giving up
    /// at the query failure cap.
    fn retry_busy(&mut self, state: State, code: ErrorCode) -> Outcome {
        self.backoff = self.backoff.after_failure();
        if self.backoff.failures() >= MAX_CONSECUTIVE_QUERY_FAILURES {
            return self.needs_human(WaitReason::Error(code), Some(code.as_str()));
        }
        self.transition(
            state,
            Some(WaitReason::Error(code)),
            Action::WaitFor {
                delay: self.backoff.delay().max(SESSION_BUSY_BASE),
            },
        )
    }

    fn pause(&mut self, reason: Option<WaitReason>) -> Outcome {
        self.clear_cycle();
        self.transition(State::Paused, reason, Action::Wait)
    }

    fn stop(&mut self, reason: WaitReason) -> Outcome {
        self.clear_cycle();
        self.transition(State::Stopped, Some(reason), Action::Notify)
    }

    fn needs_human(&mut self, reason: WaitReason, code: Option<&'static str>) -> Outcome {
        self.clear_cycle();
        if let Some(code) = code {
            self.last_result = Some(LastResult {
                kind: ResultKind::Failed,
                account_id: self.executing_account_id.clone(),
                turn_id: None,
                code: Some(code.to_owned()),
            });
        }
        self.transition(State::NeedsHuman, Some(reason), Action::Notify)
    }

    /// Drops everything that belonged to the cycle in flight. The executing account and the
    /// dedup record are kept: they describe the thread, not the cycle.
    fn clear_cycle(&mut self) {
        self.target_account_id = None;
        self.expected_available_at = None;
        self.backoff = Backoff::new();
    }

    fn transition(&mut self, state: State, reason: Option<WaitReason>, action: Action) -> Outcome {
        self.state = state;
        self.wait_reason = reason;
        self.generation = self.generation.saturating_add(1);
        Outcome::Applied(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autorun::availability::{Blocked, Reason};

    const NOW: i64 = 1_800_000_000;

    fn exhausted(turn: &str) -> Fact {
        Fact::TurnEnded {
            turn_id: turn.to_owned(),
            interruption: Interruption::Exhausted,
        }
    }

    fn ended(turn: &str, interruption: Interruption) -> Fact {
        Fact::TurnEnded {
            turn_id: turn.to_owned(),
            interruption,
        }
    }

    fn now(account: &str) -> Fact {
        Fact::Chosen(Selection::Now {
            account_id: account.to_owned(),
            reason: Reason::ListPosition,
        })
    }

    fn later(account: &str, at: i64) -> Fact {
        Fact::Chosen(Selection::Later {
            account_id: account.to_owned(),
            available_at: at,
            blockers: vec![Blocker::WeeklyExhausted {
                resets_at: Some(at),
            }],
            reason: Reason::EarliestAvailable,
        })
    }

    fn available(active: &str) -> Fact {
        Fact::Verified {
            verdict: Verdict::Available,
            active_account_id: Some(active.to_owned()),
        }
    }

    fn blocked(blockers: Vec<Blocker>, at: Option<i64>) -> Fact {
        Fact::Verified {
            verdict: Verdict::Blocked(Blocked {
                blockers,
                expected_available_at: at,
            }),
            active_account_id: Some("a".to_owned()),
        }
    }

    /// Applies a current-generation fact and returns the action.
    fn step(machine: &mut Machine, fact: Fact) -> Action {
        match machine.apply_fact(machine.generation(), fact) {
            Outcome::Applied(action) => action,
            other => panic!("expected the fact to apply, got {other:?}"),
        }
    }

    fn armed() -> Machine {
        let mut machine = Machine::new(Some(DEFAULT_MAX_RESUMES));
        assert_eq!(
            machine.apply_user(UserEvent::Enable),
            Outcome::Applied(Action::WatchThread)
        );
        machine
    }

    /// Armed, exhausted on turn `t1`, account `a` chosen and verified as the active one.
    fn resuming_on_a() -> Machine {
        let mut machine = armed();
        assert!(matches!(
            step(&mut machine, exhausted("t1")),
            Action::Select { .. }
        ));
        assert_eq!(
            step(&mut machine, now("a")),
            Action::VerifyQuota {
                account_id: "a".to_owned()
            }
        );
        assert_eq!(
            step(&mut machine, available("a")),
            Action::Resume {
                account_id: "a".to_owned()
            }
        );
        machine
    }

    fn running_on_a() -> Machine {
        let mut machine = resuming_on_a();
        assert_eq!(
            step(
                &mut machine,
                Fact::Resumed {
                    turn_id: "t2".to_owned()
                }
            ),
            Action::Wait
        );
        assert_eq!(machine.state(), State::Running);
        machine
    }

    #[test]
    fn a_new_machine_is_disabled_at_generation_zero() {
        let machine = Machine::new(None);
        assert_eq!(machine.state(), State::Disabled);
        assert_eq!(machine.generation(), 0);
        assert_eq!(machine.resume_count(), 0);
        assert_eq!(machine.expected_available_at(), None);
        assert_eq!(machine.next_check_at(), None);
    }

    #[test]
    fn enabling_arms_and_watches_the_thread() {
        let machine = armed();
        assert_eq!(machine.state(), State::Armed);
        assert_eq!(machine.generation(), 1);
    }

    #[test]
    fn enabling_twice_is_ignored() {
        let mut machine = armed();
        assert_eq!(
            machine.apply_user(UserEvent::Enable),
            Outcome::Ignored(Ignored::NotApplicable)
        );
    }

    // Every accepted event moves the generation, and a fact issued before it is dropped.
    #[test]
    fn a_fact_from_a_previous_generation_is_dropped() {
        let mut machine = armed();
        let stale = machine.generation();
        step(&mut machine, exhausted("t1"));
        assert_eq!(
            machine.apply_fact(stale, now("a")),
            Outcome::Stale {
                current: machine.generation()
            }
        );
        assert_eq!(machine.state(), State::Selecting);
    }

    #[test]
    fn the_generation_moves_on_every_accepted_event_and_not_on_a_dropped_one() {
        let mut machine = armed();
        let before = machine.generation();
        step(&mut machine, exhausted("t1"));
        assert_eq!(machine.generation(), before + 1);
        assert!(matches!(
            machine.apply_fact(machine.generation(), Fact::WaitElapsed),
            Outcome::Ignored(_)
        ));
        assert_eq!(machine.generation(), before + 1);
    }

    // A late result after a cancel restarts nothing.
    #[test]
    fn a_result_arriving_after_cancel_cannot_restart_anything() {
        let mut machine = resuming_on_a();
        let issued = machine.generation();
        assert_eq!(
            machine.apply_user(UserEvent::Cancel),
            Outcome::Applied(Action::Wait)
        );
        assert_eq!(machine.state(), State::Disabled);
        assert!(matches!(
            machine.apply_fact(
                issued,
                Fact::Resumed {
                    turn_id: "t2".to_owned()
                }
            ),
            Outcome::Stale { .. }
        ));
        assert_eq!(machine.state(), State::Disabled);
        assert_eq!(machine.resume_count(), 0);
    }

    #[test]
    fn armed_ignores_a_turn_that_ended_for_any_other_reason() {
        for interruption in [
            Interruption::Completed,
            Interruption::Network,
            Interruption::UserStopped,
            Interruption::Failed(TurnErrorKind::ContextWindowExceeded),
        ] {
            let mut machine = armed();
            assert_eq!(
                machine.apply_fact(machine.generation(), ended("t1", interruption)),
                Outcome::Ignored(Ignored::NotATrigger)
            );
            assert_eq!(machine.state(), State::Armed);
        }
    }

    #[test]
    fn before_the_first_continuation_the_exhaustion_is_the_active_accounts() {
        let mut machine = armed();

        assert_eq!(
            step(&mut machine, exhausted("t1")),
            Action::Select {
                exhausted: Some(Exhausted::ActiveAccount)
            }
        );
    }

    #[test]
    fn a_turn_toglet_started_is_pinned_on_the_account_it_resumed_with() {
        // `running_on_a` resumed turn `t1` as `a`, and the continuation started turn `t2`.
        let mut machine = running_on_a();

        assert_eq!(
            step(&mut machine, exhausted("t2")),
            Action::Select {
                exhausted: Some(Exhausted::Account("a".to_owned()))
            }
        );
    }

    #[test]
    fn an_exhaustion_seen_again_after_a_switch_is_pinned_on_nobody() {
        // `a` ran out, `b` was switched to, the resume was refused, the user pressed
        // resume. The thread now ends with a turn Toglet did not start - the desktop app ran it,
        // as whoever was signed in at the time - and `b`, which has just been switched to and
        // has run nothing, must not be judged as if it had. Nobody is named; the windows decide.
        let mut machine = running_on_a();
        step(&mut machine, exhausted("t2"));
        step(&mut machine, now("b"));
        step(&mut machine, available("a"));
        step(
            &mut machine,
            Fact::Switched {
                account_id: "b".to_owned(),
            },
        );
        step(
            &mut machine,
            Fact::ResumeFailed {
                code: ErrorCode::ThreadUnavailable,
            },
        );
        assert_eq!(machine.state(), State::NeedsHuman);
        machine.apply_user(UserEvent::Resume);

        assert_eq!(
            step(&mut machine, exhausted("t3")),
            Action::Select { exhausted: None }
        );
    }

    #[test]
    fn exhaustion_while_armed_starts_selection() {
        let mut machine = armed();
        assert!(matches!(
            step(&mut machine, exhausted("t1")),
            Action::Select { .. }
        ));
        assert_eq!(machine.state(), State::Selecting);
        assert_eq!(machine.dedup().last_observed_turn_id.as_deref(), Some("t1"));
    }

    #[test]
    fn an_available_account_is_verified_before_anything_else() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        assert_eq!(
            step(&mut machine, now("b")),
            Action::VerifyQuota {
                account_id: "b".to_owned()
            }
        );
        assert_eq!(machine.state(), State::Verifying);
        assert_eq!(machine.target_account_id(), Some("b"));
    }

    #[test]
    fn a_later_account_is_waited_for_with_the_buffer() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        assert_eq!(
            step(&mut machine, later("b", NOW)),
            Action::WaitUntil {
                at: NOW + WAIT_BUFFER_SECONDS
            }
        );
        assert_eq!(machine.state(), State::WaitingQuota);
        assert_eq!(machine.expected_available_at(), Some(NOW));
        assert_eq!(machine.next_check_at(), Some(NOW + WAIT_BUFFER_SECONDS));
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Blocked(Blocker::WeeklyExhausted {
                resets_at: Some(NOW)
            }))
        );
        assert_eq!(
            machine.last_result().map(|r| r.kind),
            Some(ResultKind::Waited)
        );
    }

    #[test]
    fn the_wait_elapsing_verifies_the_waited_for_account() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, later("b", NOW));
        assert_eq!(
            step(&mut machine, Fact::WaitElapsed),
            Action::VerifyQuota {
                account_id: "b".to_owned()
            }
        );
        assert_eq!(machine.state(), State::Verifying);
    }

    #[test]
    fn a_recheck_backs_off_then_selects_again() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        let action = step(
            &mut machine,
            Fact::Chosen(Selection::Recheck {
                account_ids: vec!["a".to_owned()],
            }),
        );
        assert_eq!(
            action,
            Action::WaitFor {
                delay: Backoff::new().after_failure().delay()
            }
        );
        assert_eq!(machine.state(), State::WaitingQuota);
        assert_eq!(machine.expected_available_at(), None);
        assert_eq!(machine.next_check_at(), None);
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Blocked(Blocker::QuotaUnknown))
        );
        assert!(matches!(
            step(&mut machine, Fact::WaitElapsed),
            Action::Select { .. }
        ));
        assert_eq!(machine.state(), State::Selecting);
    }

    #[test]
    fn nobody_usable_and_no_time_needs_a_human() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        assert_eq!(
            step(&mut machine, Fact::Chosen(Selection::NeedsHuman)),
            Action::Notify
        );
        assert_eq!(machine.state(), State::NeedsHuman);
        assert_eq!(machine.wait_reason(), Some(WaitReason::NoAccountAvailable));
    }

    #[test]
    fn verified_available_on_the_active_account_resumes_without_switching() {
        let machine = resuming_on_a();
        assert_eq!(machine.state(), State::Resuming);
        assert_eq!(machine.executing_account_id(), Some("a"));
    }

    #[test]
    fn verified_available_on_another_account_switches_first() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("b"));
        assert_eq!(
            step(&mut machine, available("a")),
            Action::Switch {
                account_id: "b".to_owned()
            }
        );
        assert_eq!(machine.state(), State::Switching);
        assert_eq!(machine.executing_account_id(), None);
    }

    #[test]
    fn a_verified_switch_resumes_on_the_new_account() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("b"));
        step(&mut machine, available("a"));
        assert_eq!(
            step(
                &mut machine,
                Fact::Switched {
                    account_id: "b".to_owned()
                }
            ),
            Action::Resume {
                account_id: "b".to_owned()
            }
        );
        assert_eq!(machine.state(), State::Resuming);
        assert_eq!(machine.executing_account_id(), Some("b"));
        assert_eq!(
            machine.last_result().map(|r| r.kind),
            Some(ResultKind::Switched)
        );
    }

    /// Switched to `b` and about to continue on it.
    fn resuming_on_b() -> Machine {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("b"));
        step(&mut machine, available("a"));
        step(
            &mut machine,
            Fact::Switched {
                account_id: "b".to_owned(),
            },
        );
        assert_eq!(machine.state(), State::Resuming);
        machine
    }

    // A switch that could not be made stops with the code.
    #[test]
    fn a_failed_switch_needs_a_human_with_the_code() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("b"));
        step(&mut machine, available("a"));
        assert_eq!(
            step(
                &mut machine,
                Fact::SwitchFailed {
                    code: ErrorCode::AuthExpired
                }
            ),
            Action::Notify
        );
        assert_eq!(machine.state(), State::NeedsHuman);
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Error(ErrorCode::AuthExpired))
        );
        assert_eq!(
            machine.last_result().and_then(|r| r.code.as_deref()),
            Some("auth_expired")
        );
    }

    /// A switch refused because somebody else is using Codex is a "not now", not a "no": the
    /// same switch is tried again after a wait, on the same target.
    #[test]
    fn a_switch_refused_by_a_busy_session_is_tried_again() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("b"));
        step(&mut machine, available("a"));
        assert_eq!(
            step(
                &mut machine,
                Fact::SwitchFailed {
                    code: ErrorCode::ClientRunning
                }
            ),
            Action::WaitFor {
                delay: SESSION_BUSY_BASE
            }
        );
        assert_eq!(machine.state(), State::Switching);
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Error(ErrorCode::ClientRunning))
        );
        assert_eq!(
            step(&mut machine, Fact::WaitElapsed),
            Action::Switch {
                account_id: "b".to_owned()
            }
        );
    }

    /// The same, one step later: a continuation refused because the session was busy waits
    /// and continues on the account already switched to.
    #[test]
    fn a_continuation_refused_by_a_busy_session_is_tried_again() {
        let mut machine = resuming_on_b();
        assert_eq!(
            step(
                &mut machine,
                Fact::ResumeFailed {
                    code: ErrorCode::ClientRunning
                }
            ),
            Action::WaitFor {
                delay: SESSION_BUSY_BASE
            }
        );
        assert_eq!(machine.state(), State::Resuming);
        assert_eq!(
            step(&mut machine, Fact::WaitElapsed),
            Action::Resume {
                account_id: "b".to_owned()
            }
        );
    }

    /// Bounded: a session busy for hours ends up in front of a person, with the code that
    /// says why, rather than being retried until the deadline.
    #[test]
    fn a_session_busy_for_long_enough_still_asks_a_person() {
        let mut machine = resuming_on_b();
        for _ in 1..MAX_CONSECUTIVE_QUERY_FAILURES {
            step(
                &mut machine,
                Fact::ResumeFailed {
                    code: ErrorCode::ClientRunning,
                },
            );
            assert_eq!(machine.state(), State::Resuming);
            step(&mut machine, Fact::WaitElapsed);
        }
        assert_eq!(
            step(
                &mut machine,
                Fact::ResumeFailed {
                    code: ErrorCode::ClientRunning
                }
            ),
            Action::Notify
        );
        assert_eq!(machine.state(), State::NeedsHuman);
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Error(ErrorCode::ClientRunning))
        );
    }

    /// A continuation refused for a reason that will not lift still stops at once.
    #[test]
    fn a_continuation_refused_for_good_still_needs_a_person() {
        let mut machine = resuming_on_b();
        assert_eq!(
            step(
                &mut machine,
                Fact::ResumeFailed {
                    code: ErrorCode::ThreadUnavailable
                }
            ),
            Action::Notify
        );
        assert_eq!(machine.state(), State::NeedsHuman);
    }

    // Still exhausted at the expected time: wait for the next time, never send.
    #[test]
    fn still_blocked_with_a_time_waits_for_that_time() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("a"));
        let action = step(
            &mut machine,
            blocked(
                vec![Blocker::FiveHourExhausted {
                    resets_at: Some(NOW),
                }],
                Some(NOW),
            ),
        );
        assert_eq!(
            action,
            Action::WaitUntil {
                at: NOW + WAIT_BUFFER_SECONDS
            }
        );
        assert_eq!(machine.state(), State::WaitingQuota);
        assert_eq!(machine.target_account_id(), Some("a"));
    }

    #[test]
    fn still_unknown_backs_off_and_verifies_the_same_account_again() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("a"));
        let action = step(&mut machine, blocked(vec![Blocker::QuotaUnknown], None));
        assert_eq!(
            action,
            Action::WaitFor {
                delay: Backoff::new().after_failure().delay()
            }
        );
        assert_eq!(
            step(&mut machine, Fact::WaitElapsed),
            Action::VerifyQuota {
                account_id: "a".to_owned()
            }
        );
    }

    #[test]
    fn blocked_for_good_without_a_time_goes_back_to_choosing() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("a"));
        assert!(matches!(
            step(&mut machine, blocked(vec![Blocker::ReauthRequired], None)),
            Action::Select { .. }
        ));
        assert_eq!(machine.state(), State::Selecting);
        assert_eq!(machine.target_account_id(), None);
    }

    #[test]
    fn a_failed_query_while_verifying_backs_off_and_retries() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("a"));
        let first = step(&mut machine, Fact::QueryFailed);
        let second = step(&mut machine, Fact::QueryFailed);
        assert_eq!(
            first,
            Action::WaitFor {
                delay: Duration::from_secs(30)
            }
        );
        assert_eq!(
            second,
            Action::WaitFor {
                delay: Duration::from_secs(60)
            }
        );
        assert_eq!(machine.state(), State::Verifying);
        assert_eq!(
            step(&mut machine, Fact::WaitElapsed),
            Action::VerifyQuota {
                account_id: "a".to_owned()
            }
        );
    }

    #[test]
    fn query_failures_at_the_cap_need_a_human() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, now("a"));
        for _ in 1..MAX_CONSECUTIVE_QUERY_FAILURES {
            step(&mut machine, Fact::QueryFailed);
            assert_eq!(machine.state(), State::Verifying);
        }
        assert_eq!(step(&mut machine, Fact::QueryFailed), Action::Notify);
        assert_eq!(machine.state(), State::NeedsHuman);
        assert_eq!(machine.wait_reason(), Some(WaitReason::QueryFailures));
    }

    // One exhaustion, one continuation.
    #[test]
    fn one_exhaustion_produces_exactly_one_resume() {
        let machine = running_on_a();
        assert_eq!(machine.resume_count(), 1);
        assert_eq!(machine.dedup().last_resumed_turn_id.as_deref(), Some("t1"));
        assert_eq!(
            machine.last_result().map(|r| r.kind),
            Some(ResultKind::Resumed)
        );
        assert_eq!(
            machine.last_result().and_then(|r| r.turn_id.as_deref()),
            Some("t2")
        );
    }

    // The same exhausted turn is never continued twice.
    #[test]
    fn the_same_exhausted_turn_seen_again_is_ignored() {
        let mut machine = running_on_a();
        machine.apply_user(UserEvent::Pause);
        machine.apply_user(UserEvent::Resume);
        assert_eq!(machine.state(), State::Armed);
        assert_eq!(
            machine.apply_fact(machine.generation(), exhausted("t1")),
            Outcome::Ignored(Ignored::AlreadyResumed)
        );
        assert_eq!(machine.state(), State::Armed);
    }

    // A completed continuation ends the round; nothing else is sent.
    #[test]
    fn a_completed_turn_ends_the_round() {
        let mut machine = running_on_a();
        assert_eq!(
            step(&mut machine, ended("t2", Interruption::Completed)),
            Action::Notify
        );
        assert_eq!(machine.state(), State::RoundCompleted);
        assert_eq!(
            machine.last_result().map(|r| r.kind),
            Some(ResultKind::Completed)
        );
        assert!(matches!(
            machine.apply_fact(machine.generation(), Fact::WaitElapsed),
            Outcome::Ignored(_)
        ));
    }

    // While running, other accounts recovering produce no action.
    #[test]
    fn while_running_nothing_but_the_turn_is_acted_on() {
        let mut machine = running_on_a();
        let generation = machine.generation();
        for fact in [
            now("b"),
            available("b"),
            Fact::WaitElapsed,
            Fact::TurnInProgress,
            Fact::Switched {
                account_id: "b".to_owned(),
            },
        ] {
            assert_eq!(
                machine.apply_fact(generation, fact),
                Outcome::Ignored(Ignored::NotApplicable)
            );
        }
        assert_eq!(machine.state(), State::Running);
        assert_eq!(machine.generation(), generation);
    }

    #[test]
    fn a_second_exhaustion_starts_the_next_cycle_and_counts() {
        let mut machine = running_on_a();
        assert!(matches!(
            step(&mut machine, exhausted("t2")),
            Action::Select { .. }
        ));
        assert_eq!(machine.state(), State::Selecting);
        assert_eq!(machine.resume_count(), 1);
        step(&mut machine, now("a"));
        step(&mut machine, available("a"));
        step(
            &mut machine,
            Fact::Resumed {
                turn_id: "t3".to_owned(),
            },
        );
        assert_eq!(machine.resume_count(), 2);
        assert_eq!(machine.dedup().last_resumed_turn_id.as_deref(), Some("t2"));
    }

    // The resume cap.
    #[test]
    fn reaching_the_resume_cap_stops() {
        let mut machine = Machine::new(Some(2));
        machine.apply_user(UserEvent::Enable);
        for (exhausted_turn, new_turn) in [("t1", "t2"), ("t2", "t3")] {
            step(&mut machine, exhausted(exhausted_turn));
            step(&mut machine, now("a"));
            step(&mut machine, available("a"));
            step(
                &mut machine,
                Fact::Resumed {
                    turn_id: new_turn.to_owned(),
                },
            );
        }
        assert_eq!(machine.resume_count(), 2);
        assert_eq!(step(&mut machine, exhausted("t3")), Action::Notify);
        assert_eq!(machine.state(), State::Stopped);
        assert_eq!(machine.wait_reason(), Some(WaitReason::MaxResumes));
    }

    #[test]
    fn no_cap_means_no_stop() {
        let mut machine = Machine::new(None);
        machine.apply_user(UserEvent::Enable);
        for i in 0..20 {
            step(&mut machine, exhausted(&format!("t{i}")));
            step(&mut machine, now("a"));
            step(&mut machine, available("a"));
            step(
                &mut machine,
                Fact::Resumed {
                    turn_id: format!("t{}", i + 1),
                },
            );
        }
        assert_eq!(machine.state(), State::Running);
        assert_eq!(machine.resume_count(), 20);
    }

    // The deadline.
    #[test]
    fn the_deadline_stops_from_any_state_but_disabled_and_stopped() {
        for build in [armed, resuming_on_a, running_on_a, || {
            let mut machine = running_on_a();
            machine.apply_user(UserEvent::Pause);
            machine
        }] {
            let mut machine = build();
            assert_eq!(step(&mut machine, Fact::DeadlineReached), Action::Notify);
            assert_eq!(machine.state(), State::Stopped);
            assert_eq!(machine.wait_reason(), Some(WaitReason::Deadline));
            assert_eq!(
                machine.apply_fact(machine.generation(), Fact::DeadlineReached),
                Outcome::Ignored(Ignored::NotApplicable)
            );
        }
        let mut machine = Machine::new(None);
        assert_eq!(
            machine.apply_fact(0, Fact::DeadlineReached),
            Outcome::Ignored(Ignored::NotApplicable)
        );
    }

    #[test]
    fn a_network_failure_of_the_continuation_backs_off_on_the_same_account() {
        let mut machine = running_on_a();
        assert_eq!(
            step(&mut machine, ended("t2", Interruption::Network)),
            Action::WaitFor {
                delay: Duration::from_secs(30)
            }
        );
        assert_eq!(machine.state(), State::WaitingNetwork);
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Turn(Interruption::Network))
        );
        assert_eq!(step(&mut machine, Fact::WaitElapsed), Action::ReadThread);
        assert_eq!(machine.state(), State::WaitingNetwork);
    }

    #[test]
    fn after_a_network_wait_the_thread_decides_what_happens_next() {
        let mut waiting = running_on_a();
        step(&mut waiting, ended("t2", Interruption::Network));
        step(&mut waiting, Fact::WaitElapsed);

        let mut machine = waiting.clone();
        assert_eq!(step(&mut machine, Fact::TurnInProgress), Action::Wait);
        assert_eq!(machine.state(), State::Running);

        let mut machine = waiting.clone();
        assert_eq!(
            step(&mut machine, ended("t2", Interruption::Completed)),
            Action::Notify
        );
        assert_eq!(machine.state(), State::RoundCompleted);

        let mut machine = waiting.clone();
        assert_eq!(
            step(&mut machine, ended("t2", Interruption::Network)),
            Action::Resume {
                account_id: "a".to_owned()
            }
        );
        assert_eq!(machine.state(), State::Resuming);

        let mut machine = waiting.clone();
        assert!(matches!(
            step(&mut machine, exhausted("t2")),
            Action::Select { .. }
        ));
        assert_eq!(machine.state(), State::Selecting);

        let mut machine = waiting;
        assert_eq!(
            step(&mut machine, Fact::QueryFailed),
            Action::WaitFor {
                delay: Duration::from_secs(60)
            }
        );
        assert_eq!(machine.state(), State::WaitingNetwork);
    }

    #[test]
    fn an_expired_login_needs_a_human() {
        let mut machine = running_on_a();
        assert_eq!(
            step(&mut machine, ended("t2", Interruption::AuthExpired)),
            Action::Notify
        );
        assert_eq!(machine.state(), State::NeedsHuman);
        assert_eq!(
            machine.wait_reason().map(WaitReason::as_str),
            Some("unauthorized")
        );
        assert_eq!(
            machine.last_result().map(|r| r.kind),
            Some(ResultKind::Failed)
        );
    }

    #[test]
    fn other_failures_need_a_human_with_their_code() {
        for (interruption, code) in [
            (
                Interruption::Failed(TurnErrorKind::ContextWindowExceeded),
                "context_window_exceeded",
            ),
            (
                Interruption::FailedUnknownReason,
                "turn_failed_unknown_reason",
            ),
            (Interruption::StatusUnknown, "turn_status_unknown"),
        ] {
            let mut machine = running_on_a();
            step(&mut machine, ended("t2", interruption));
            assert_eq!(machine.state(), State::NeedsHuman);
            assert_eq!(machine.wait_reason().map(WaitReason::as_str), Some(code));
        }
    }

    // Waiting on a person.
    #[test]
    fn waiting_on_a_person_needs_a_human_and_sends_nothing() {
        let mut machine = running_on_a();
        assert_eq!(step(&mut machine, Fact::WaitingOnHuman), Action::Notify);
        assert_eq!(machine.state(), State::NeedsHuman);
        assert_eq!(machine.wait_reason(), Some(WaitReason::WaitingOnHuman));
    }

    // The user interrupted the turn.
    #[test]
    fn an_interrupted_turn_pauses() {
        let mut machine = running_on_a();
        assert_eq!(
            step(&mut machine, ended("t2", Interruption::UserStopped)),
            Action::Wait
        );
        assert_eq!(machine.state(), State::Paused);
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Turn(Interruption::UserStopped))
        );
    }

    #[test]
    fn a_turn_already_running_when_resuming_is_waited_for_not_restarted() {
        let mut machine = resuming_on_a();
        assert_eq!(step(&mut machine, Fact::TurnInProgress), Action::Wait);
        assert_eq!(machine.state(), State::Running);
        assert_eq!(machine.resume_count(), 0);
        assert_eq!(
            step(&mut machine, ended("t9", Interruption::Completed)),
            Action::Notify
        );
        assert_eq!(machine.state(), State::RoundCompleted);
    }

    #[test]
    fn a_failed_resume_needs_a_human() {
        let mut machine = resuming_on_a();
        assert_eq!(
            step(
                &mut machine,
                Fact::ResumeFailed {
                    code: ErrorCode::ThreadUnavailable
                }
            ),
            Action::Notify
        );
        assert_eq!(machine.state(), State::NeedsHuman);
        assert_eq!(
            machine.wait_reason(),
            Some(WaitReason::Error(ErrorCode::ThreadUnavailable))
        );
        assert_eq!(machine.resume_count(), 0);
    }

    #[test]
    fn a_manual_switch_pauses_while_waiting() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        step(&mut machine, later("b", NOW));
        assert_eq!(step(&mut machine, Fact::ManualSwitchObserved), Action::Wait);
        assert_eq!(machine.state(), State::Paused);
        assert_eq!(machine.wait_reason(), Some(WaitReason::ManualSwitch));
        assert_eq!(machine.expected_available_at(), None);
    }

    #[test]
    fn the_desktop_app_reopening_pauses_while_waiting() {
        let mut machine = armed();
        step(&mut machine, exhausted("t1"));
        assert_eq!(step(&mut machine, Fact::DesktopReopened), Action::Wait);
        assert_eq!(machine.state(), State::Paused);
        assert_eq!(machine.wait_reason(), Some(WaitReason::DesktopReopened));
    }

    #[test]
    fn outside_events_are_ignored_once_nothing_is_in_flight() {
        let mut machine = running_on_a();
        machine.apply_user(UserEvent::Pause);
        let generation = machine.generation();
        assert_eq!(
            machine.apply_fact(generation, Fact::ManualSwitchObserved),
            Outcome::Ignored(Ignored::NotApplicable)
        );
        assert_eq!(
            machine.apply_fact(generation, Fact::DesktopReopened),
            Outcome::Ignored(Ignored::NotApplicable)
        );
    }

    #[test]
    fn pause_and_resume_go_through_armed_and_recheck_the_thread() {
        let mut machine = running_on_a();
        assert_eq!(
            machine.apply_user(UserEvent::Pause),
            Outcome::Applied(Action::Wait)
        );
        assert_eq!(machine.state(), State::Paused);
        assert_eq!(machine.wait_reason(), None);
        assert_eq!(
            machine.apply_user(UserEvent::Resume),
            Outcome::Applied(Action::WatchThread)
        );
        assert_eq!(machine.state(), State::Armed);
        assert_eq!(machine.resume_count(), 1);
        assert_eq!(machine.executing_account_id(), Some("a"));
    }

    #[test]
    fn a_completed_round_and_a_stop_never_rearm_on_their_own() {
        let mut completed = running_on_a();
        step(&mut completed, ended("t2", Interruption::Completed));
        assert!(matches!(
            completed.apply_fact(completed.generation(), exhausted("t2")),
            Outcome::Ignored(_)
        ));
        assert_eq!(completed.state(), State::RoundCompleted);
        assert_eq!(
            completed.apply_user(UserEvent::Resume),
            Outcome::Applied(Action::WatchThread)
        );

        let mut stopped = armed();
        step(&mut stopped, Fact::DeadlineReached);
        assert!(matches!(
            stopped.apply_fact(stopped.generation(), exhausted("t1")),
            Outcome::Ignored(_)
        ));
        assert_eq!(stopped.state(), State::Stopped);
    }

    #[test]
    fn user_events_that_make_no_sense_are_ignored() {
        let mut machine = Machine::new(None);
        for event in [UserEvent::Pause, UserEvent::Resume, UserEvent::Cancel] {
            assert_eq!(
                machine.apply_user(event),
                Outcome::Ignored(Ignored::NotApplicable)
            );
        }
        let mut machine = armed();
        assert_eq!(
            machine.apply_user(UserEvent::Resume),
            Outcome::Ignored(Ignored::NotApplicable)
        );
    }

    #[test]
    fn cancel_disables_and_enabling_again_starts_a_fresh_run() {
        let mut machine = running_on_a();
        machine.apply_user(UserEvent::Cancel);
        assert_eq!(machine.state(), State::Disabled);
        assert_eq!(machine.executing_account_id(), None);
        machine.apply_user(UserEvent::Enable);
        assert_eq!(machine.resume_count(), 0);
        assert_eq!(machine.dedup(), &Dedup::default());
        assert_eq!(machine.last_result(), None);
    }

    #[test]
    fn a_turn_record_maps_to_its_class() {
        fn turn(status: TurnStatus, error: Option<TurnErrorKind>) -> TurnRecord {
            TurnRecord {
                id: "t".to_owned(),
                status,
                error,
                started_at: None,
                completed_at: None,
            }
        }
        let cases = [
            (
                TurnStatus::Failed,
                Some(TurnErrorKind::UsageLimitExceeded),
                Interruption::Exhausted,
            ),
            (
                TurnStatus::Failed,
                Some(TurnErrorKind::Network { http_status: None }),
                Interruption::Network,
            ),
            (
                TurnStatus::Failed,
                Some(TurnErrorKind::ServerOverloaded),
                Interruption::Network,
            ),
            (
                TurnStatus::Failed,
                Some(TurnErrorKind::Unauthorized),
                Interruption::AuthExpired,
            ),
            (
                TurnStatus::Failed,
                Some(TurnErrorKind::RateLimitExceeded),
                Interruption::Failed(TurnErrorKind::RateLimitExceeded),
            ),
            (TurnStatus::Failed, None, Interruption::FailedUnknownReason),
            (TurnStatus::Completed, None, Interruption::Completed),
            (TurnStatus::Interrupted, None, Interruption::UserStopped),
            (TurnStatus::Unknown, None, Interruption::StatusUnknown),
        ];
        for (status, error, expected) in cases {
            assert_eq!(
                Fact::from_turn(&turn(status, error)),
                Fact::TurnEnded {
                    turn_id: "t".to_owned(),
                    interruption: expected
                }
            );
        }
        assert_eq!(
            Fact::from_turn(&turn(TurnStatus::InProgress, None)),
            Fact::TurnInProgress
        );
    }

    fn restored(state: State) -> Restored {
        Restored {
            state,
            generation: 7,
            resume_count: 3,
            max_resumes: Some(8),
            executing_account_id: Some("a".to_owned()),
            wait_reason: Some(WaitReason::MaxResumes),
            dedup: Dedup {
                last_observed_turn_id: Some("t3".to_owned()),
                last_resumed_turn_id: Some("t3".to_owned()),
            },
            last_result: Some(LastResult {
                kind: ResultKind::Resumed,
                account_id: Some("a".to_owned()),
                turn_id: Some("t4".to_owned()),
                code: None,
            }),
        }
    }

    // A restart never resumes a cycle on its own.
    #[test]
    fn a_restored_active_state_comes_back_armed() {
        for state in [
            State::Armed,
            State::Selecting,
            State::WaitingQuota,
            State::Verifying,
            State::Switching,
            State::Resuming,
            State::Running,
            State::WaitingNetwork,
        ] {
            let machine = Machine::restore(restored(state));
            assert_eq!(machine.state(), State::Armed, "{state:?}");
            // Armed carries no reason: nothing is waiting on anything.
            assert_eq!(machine.wait_reason(), None, "{state:?}");
            assert_eq!(machine.generation(), 8);
            // The count and the memory of what was already continued survive a restart.
            assert_eq!(machine.resume_count(), 3);
            assert_eq!(machine.executing_account_id(), Some("a"));
            assert_eq!(machine.expected_available_at(), None);
            assert_eq!(machine.target_account_id(), None);
            assert_eq!(machine.dedup().last_resumed_turn_id.as_deref(), Some("t3"));
        }
    }

    /// A plan written by a build that paused on restart is re-armed.
    #[test]
    fn a_plan_paused_only_by_an_older_builds_restart_comes_back_armed() {
        let mut restored = restored(State::Paused);
        restored.wait_reason = Some(WaitReason::AppRestarted);
        let machine = Machine::restore(restored);
        assert_eq!(machine.state(), State::Armed);
        assert_eq!(machine.wait_reason(), None);
    }

    /// A restart does not undo a turn the user continued: the turn already continued is still
    /// refused, so the round is not sent twice.
    #[test]
    fn a_restart_does_not_continue_the_same_turn_twice() {
        let mut machine = Machine::restore(restored(State::Running));
        assert_eq!(machine.state(), State::Armed);
        assert_eq!(
            machine.apply_fact(machine.generation(), exhausted("t3")),
            Outcome::Ignored(Ignored::AlreadyResumed)
        );
    }

    #[test]
    fn a_restored_waiting_state_comes_back_as_it_was() {
        for state in [
            State::Disabled,
            State::Paused,
            State::Stopped,
            State::RoundCompleted,
            State::NeedsHuman,
        ] {
            let machine = Machine::restore(restored(state));
            assert_eq!(machine.state(), state);
            assert_eq!(machine.wait_reason(), Some(WaitReason::MaxResumes));
            assert_eq!(machine.generation(), 7);
        }
    }

    // Across a restart, the exhaustion already continued is still remembered.
    #[test]
    fn a_restored_machine_still_refuses_the_turn_it_already_continued() {
        let mut machine = Machine::restore(restored(State::WaitingQuota));
        machine.apply_user(UserEvent::Resume);
        assert_eq!(
            machine.apply_fact(machine.generation(), exhausted("t3")),
            Outcome::Ignored(Ignored::AlreadyResumed)
        );
        assert!(matches!(
            step(&mut machine, exhausted("t4")),
            Action::Select { .. }
        ));
    }

    #[test]
    fn the_serde_form_of_a_state_is_its_code() {
        for state in ALL_STATES {
            let json = serde_json::to_string(&state).expect("serialises");
            assert_eq!(json, format!("\"{}\"", state.as_str()));
            let back: State = serde_json::from_str(&json).expect("parses");
            assert_eq!(back, state);
        }
    }

    #[test]
    fn every_wait_reason_round_trips_through_its_code() {
        let reasons = [
            WaitReason::Blocked(Blocker::WeeklyExhausted { resets_at: None }),
            WaitReason::Blocked(Blocker::QuotaUnknown),
            WaitReason::Error(ErrorCode::ThreadUnavailable),
            WaitReason::Error(ErrorCode::ClientRunning),
            WaitReason::Turn(Interruption::Network),
            WaitReason::Turn(Interruption::AuthExpired),
            WaitReason::Turn(Interruption::UserStopped),
            WaitReason::Turn(Interruption::Failed(TurnErrorKind::ContextWindowExceeded)),
            WaitReason::Turn(Interruption::FailedUnknownReason),
            WaitReason::Turn(Interruption::StatusUnknown),
            WaitReason::WaitingOnHuman,
            WaitReason::NoAccountAvailable,
            WaitReason::QueryFailures,
            WaitReason::ManualSwitch,
            WaitReason::DesktopReopened,
            WaitReason::MaxResumes,
            WaitReason::Deadline,
            WaitReason::AppRestarted,
        ];
        for reason in reasons {
            assert_eq!(
                WaitReason::parse(reason.as_str()),
                Some(reason),
                "{reason:?}"
            );
        }
        // A blocker's reset time is not part of its code; the expected time lives elsewhere.
        assert_eq!(
            WaitReason::parse(
                WaitReason::Blocked(Blocker::FiveHourExhausted {
                    resets_at: Some(NOW)
                })
                .as_str()
            ),
            Some(WaitReason::Blocked(Blocker::FiveHourExhausted {
                resets_at: None
            }))
        );
        assert_eq!(WaitReason::parse(""), None);
        assert_eq!(WaitReason::parse("Some prose"), None);
    }

    const ALL_STATES: [State; 13] = [
        State::Disabled,
        State::Armed,
        State::Selecting,
        State::WaitingQuota,
        State::Verifying,
        State::Switching,
        State::Resuming,
        State::Running,
        State::WaitingNetwork,
        State::RoundCompleted,
        State::NeedsHuman,
        State::Paused,
        State::Stopped,
    ];

    #[test]
    fn every_code_is_a_stable_snake_case_word() {
        let states = ALL_STATES;
        let reasons = [
            WaitReason::Blocked(Blocker::QuotaUnknown),
            WaitReason::Error(ErrorCode::ThreadUnavailable),
            WaitReason::Turn(Interruption::Failed(TurnErrorKind::Other)),
            WaitReason::WaitingOnHuman,
            WaitReason::NoAccountAvailable,
            WaitReason::QueryFailures,
            WaitReason::ManualSwitch,
            WaitReason::DesktopReopened,
            WaitReason::MaxResumes,
            WaitReason::Deadline,
            WaitReason::AppRestarted,
        ];
        let kinds = [
            ResultKind::Resumed,
            ResultKind::Switched,
            ResultKind::Waited,
            ResultKind::Failed,
            ResultKind::Completed,
        ];
        let codes = states
            .iter()
            .map(|s| s.as_str())
            .chain(reasons.iter().map(|r| r.as_str()))
            .chain(kinds.iter().map(|k| k.as_str()));
        for code in codes {
            assert!(
                code.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{code}"
            );
        }
        assert_eq!(states.len(), 13);
    }
}
