//! The real [`Ports`]: each driver action goes through the same code as the manual path
//! (`accounts::rate_limits`, `switching::perform` under the shared switch lock).
//! Application state comes through [`Services`] so tests can run against fakes.

use std::time::Duration;

use super::availability::{AccountFacts, Candidate, Selection, assess, choose};
use super::driver::{Clock, Ports, Resumed, Verification, parse_rfc3339, rfc3339};
use super::executor::{Continuation, Executor};
use super::machine::{Exhausted, Fact, State, WaitReason};
use super::plan::AutoRunPlan;
use super::takeover::Takeover;
use crate::accounts::{AccountStatus, rate_limits};
use crate::app_server::{CodexBinary, RawRateLimits, TurnErrorKind};
use crate::credentials::{CredentialLock, CredentialRef, SecretStore};
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::process::{ClientProbe, ClientRestart};
use crate::quota::{NormalisedQuota, QuotaSnapshot};
use crate::storage::SwitchVerified;
use crate::switching::{
    ActiveRecord, Faults, NoObserver, SwitchContext, SwitchLock, SwitchReport, SwitchTarget,
    perform,
};

const PHASE: Phase = Phase::Autorun;

/// One participating account, as Toglet has it on record.
#[derive(Debug, Clone)]
pub struct ParticipantRecord {
    pub credential_ref: String,
    /// `accountFingerprint`: an account identifier, never logged or sent to the frontend.
    pub fingerprint: String,
    pub status: AccountStatus,
    /// RFC 3339, as the profile stores it.
    pub created_at: String,
}

/// The account Codex is signed in as, as Toglet has it on record.
#[derive(Debug, Clone)]
pub struct ActiveAccountRecord {
    pub account_id: String,
    pub credential_ref: String,
    pub fingerprint: String,
}

/// What the ports need from the application; implemented over app state, and over fakes in tests.
pub trait Services: Send + 'static {
    fn secrets(&self) -> &dyn SecretStore;
    fn switch_lock(&self) -> &SwitchLock;
    fn credential_lock(&self) -> &CredentialLock;
    /// The application data directory, where the switch journal lives.
    fn journal_directory(&self) -> &std::path::Path;
    /// The user's real Codex home.
    fn default_home(&self) -> Result<std::path::PathBuf>;
    /// The Codex binary to run: the user's chosen path when set.
    fn binary(&self) -> Result<CodexBinary>;
    fn participant(&self, account_id: &str) -> Option<ParticipantRecord>;
    fn active(&self) -> Option<ActiveAccountRecord>;
    /// Records the active account. Only a verified switch produces the token.
    fn record_active(&self, account_id: &str, verified: &SwitchVerified) -> Result<()>;
    fn reopen_after_switch(&self) -> bool;
}

pub struct AppPorts {
    services: Box<dyn Services>,
    probe: Box<dyn ClientProbe + Send>,
    restart: Box<dyn ClientRestart + Send>,
    faults: Box<dyn Faults + Send>,
    clock: Box<dyn Clock>,
    executor: Option<Executor>,
    takeover: Takeover,
    /// Shared with `AutoRun` so `remote` can seal the agent's last message into a receipt.
    ///
    /// Deliberately not the plan and not the view: the plan is written to disk and the view
    /// crosses to the desktop interface, and this is session content.
    excerpt: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl AppPorts {
    pub fn new(
        services: Box<dyn Services>,
        probe: Box<dyn ClientProbe + Send>,
        restart: Box<dyn ClientRestart + Send>,
        faults: Box<dyn Faults + Send>,
        clock: Box<dyn Clock>,
        excerpt: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            services,
            probe,
            restart,
            faults,
            clock,
            executor: None,
            takeover: Takeover::new(),
            excerpt,
        }
    }

    /// Copies whatever the executor last saw into the shared slot.
    fn publish_excerpt(&mut self) {
        let latest = self.executor.as_ref().and_then(Executor::last_excerpt);
        if latest.is_none() {
            return;
        }
        let mut slot = self
            .excerpt
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *slot = latest;
    }

    fn executor(&mut self) -> Result<&mut Executor> {
        if self.executor.is_none() {
            let binary = self.services.binary()?;
            let home = self.services.default_home()?;
            self.executor = Some(Executor::new(binary, &home));
        }
        self.executor
            .as_mut()
            .ok_or_else(|| internal("the executor was not created"))
    }

    /// Toglet's own app servers, which the client probe must leave alone.
    fn own_processes(&self) -> Vec<u32> {
        self.executor
            .as_ref()
            .and_then(Executor::pid)
            .into_iter()
            .collect()
    }

    /// One account's quota, read fresh: in place for the active account, through a throwaway
    /// home otherwise.
    fn read_quota(&self, account_id: &str, record: &ParticipantRecord) -> Result<RawRateLimits> {
        let binary = self.services.binary()?;
        let is_active = self
            .services
            .active()
            .is_some_and(|active| active.account_id == account_id);
        if is_active {
            rate_limits::read_active(&binary, &self.services.default_home()?)
        } else {
            let reference = CredentialRef::new(&record.credential_ref)?;
            rate_limits::read_stored(
                self.services.credential_lock(),
                self.services.secrets(),
                &binary,
                &reference,
            )
        }
    }

    /// Judges one account on a fresh reading. A failed reading is a `quota_unknown` blocker,
    /// not an error.
    fn judge(
        &self,
        account_id: &str,
        record: &ParticipantRecord,
        exhausted_here: bool,
        now: i64,
    ) -> super::availability::Verdict {
        let reading = self.read_quota(account_id, record);
        let snapshot = match &reading {
            Ok(raw) => Some(QuotaSnapshot::fresh(
                account_id,
                NormalisedQuota::from_raw(raw),
                now,
            )),
            Err(error) => {
                log(
                    &LogRecord::new(Level::Warn, "autorun_participant_unreadable")
                        .with_phase(PHASE)
                        .with_code(error.code()),
                );
                None
            }
        };
        let facts = AccountFacts {
            status: record.status,
            quota: snapshot.as_ref(),
            last_turn_error: exhausted_here.then_some(TurnErrorKind::UsageLimitExceeded),
        };
        assess(&facts, now)
    }
}

/// Resolves the machine's evidence to an account id: the named one, or whoever Codex is signed
/// in as, or nobody.
fn exhausted_account(exhausted: Option<&Exhausted>, active: Option<&str>) -> Option<String> {
    match exhausted {
        Some(Exhausted::Account(id)) => Some(id.clone()),
        Some(Exhausted::ActiveAccount) => active.map(str::to_owned),
        None => None,
    }
}

impl Ports for AppPorts {
    fn read_thread(&mut self, plan: &AutoRunPlan) -> Result<Fact> {
        let thread_id = bound_thread(plan)?.to_owned();
        let fact = self.executor()?.read_thread(&thread_id);
        self.publish_excerpt();
        fact
    }

    fn select(&mut self, plan: &AutoRunPlan, exhausted: Option<&Exhausted>) -> Selection {
        let now = self.clock.now();
        let active = self.services.active().map(|active| active.account_id);
        // The executing account keeps its place at the front; whose turn ran out is decided by
        // the machine, since after a switch it is a different account.
        let executing = plan.executing_account_id.clone().or_else(|| active.clone());
        let exhausted = exhausted_account(exhausted, active.as_deref());

        let mut participants: Vec<_> = plan.participants.iter().collect();
        participants.sort_by_key(|participant| participant.order);
        let candidates: Vec<Candidate> = participants
            .iter()
            .enumerate()
            .filter_map(|(position, participant)| {
                let Some(record) = self.services.participant(&participant.account_id) else {
                    // An account that has since been removed takes no part.
                    log(&LogRecord::new(Level::Warn, "autorun_participant_missing")
                        .with_phase(PHASE));
                    return None;
                };
                let exhausted_here = exhausted.as_deref() == Some(participant.account_id.as_str());
                Some(Candidate {
                    account_id: participant.account_id.clone(),
                    verdict: self.judge(&participant.account_id, &record, exhausted_here, now),
                    list_position: position,
                    created_at: parse_rfc3339(&record.created_at).unwrap_or(i64::MAX),
                })
            })
            .collect();

        // The list order is the user's priority among the rest; the account already executing
        // still comes first, so a recovered account does not cause a switch for nothing.
        choose(&candidates, executing.as_deref(), false)
    }

    fn verify_quota(&mut self, account_id: &str) -> Result<Verification> {
        let record = self
            .services
            .participant(account_id)
            .ok_or_else(unknown_account)?;
        let now = self.clock.now();
        // A read that fails here is a failed read, not a blocker: the machine backs off.
        let raw = self.read_quota(account_id, &record)?;
        let snapshot = QuotaSnapshot::fresh(account_id, NormalisedQuota::from_raw(&raw), now);
        let facts = AccountFacts {
            status: record.status,
            quota: Some(&snapshot),
            last_turn_error: None,
        };
        Ok(Verification {
            verdict: assess(&facts, now),
            active_account_id: self.services.active().map(|active| active.account_id),
        })
    }

    fn switch(&mut self, account_id: &str) -> Result<()> {
        let target_record = self
            .services
            .participant(account_id)
            .ok_or_else(unknown_account)?;
        let target_reference = CredentialRef::new(&target_record.credential_ref)?;
        let active = self.services.active();
        let active_reference = match &active {
            Some(active) => Some(CredentialRef::new(&active.credential_ref)?),
            None => None,
        };
        let binary = self.services.binary()?;
        let home = self.services.default_home()?;
        let own = self.own_processes();
        let now = self.clock.now();

        let context = SwitchContext {
            lock: self.services.switch_lock(),
            credential_lock: self.services.credential_lock(),
            store: self.services.secrets(),
            probe: self.probe.as_ref(),
            restart: self.restart.as_ref(),
            binary: &binary,
            default_home: &home,
            journal_directory: self.services.journal_directory(),
            own_processes: &own,
            faults: self.faults.as_ref(),
            observer: &NoObserver,
        };
        let report = perform(
            &context,
            match (&active, &active_reference) {
                (Some(active), Some(reference)) => Some(ActiveRecord {
                    account_id: &active.account_id,
                    credentials: reference,
                    fingerprint: &active.fingerprint,
                }),
                _ => None,
            },
            SwitchTarget {
                account_id,
                credentials: &target_reference,
            },
            &format!("autorun-{now}"),
            &rfc3339(now),
        )?;

        match report {
            SwitchReport::Switched { verified, plan, .. } => {
                // Verified by Codex; a failed record write is logged and repaired by the next sync.
                // The executor re-confirms the identity before starting anything.
                if let Err(error) = self.services.record_active(account_id, &verified) {
                    log(&LogRecord::from_error(
                        "active_account_not_recorded",
                        &error,
                    ));
                }
                // Kept closed: the session is about to be taken over. Reopened on release.
                self.takeover.remember(plan);
                Ok(())
            }
            SwitchReport::Failed {
                error, rollback, ..
            } => {
                log(&LogRecord::new(Level::Error, "autorun_switch_rolled_back")
                    .with_phase(PHASE)
                    .with_code(error.code())
                    .with_detail(rollback.as_str()));
                Err(error)
            }
        }
    }

    fn resume(
        &mut self,
        plan: &AutoRunPlan,
        account_id: &str,
        after_turn: Option<&str>,
        instruction: Option<&str>,
    ) -> Result<Resumed> {
        let binding = plan
            .binding
            .as_ref()
            .ok_or_else(|| internal("the plan is not bound"))?;
        let record = self
            .services
            .participant(account_id)
            .ok_or_else(unknown_account)?;

        // Single writer: nobody else writes to the session from here on.
        let own = self.own_processes();
        self.takeover
            .acquire(self.probe.as_ref(), self.restart.as_ref(), &own)?;

        let thread_id = binding.thread_id.clone();
        // The phone's sentence wins for this turn; the stored instruction is left untouched,
        // because automatic continuation still needs it when quota comes back. Which of the two
        // it is also decides what may be done to a thread that is parked.
        let text = instruction
            .unwrap_or(&binding.resume_instruction)
            .to_owned();
        let continuation = match instruction {
            Some(_) => Continuation::Steered(&text),
            None => Continuation::Automatic(&text),
        };
        self.executor()?
            .resume(&thread_id, continuation, &record.fingerprint, after_turn)
    }

    fn poll_turn(&mut self, wait: Duration) -> Option<Fact> {
        let fact = self
            .executor
            .as_mut()
            .and_then(|executor| executor.poll(wait));
        self.publish_excerpt();
        fact
    }

    fn stop_executing(&mut self) {
        if let Some(executor) = self.executor.as_mut() {
            executor.stop();
        }
        let reopen = self.services.reopen_after_switch();
        self.takeover.release(self.restart.as_ref(), reopen);
    }

    fn notify(&mut self, state: State, reason: Option<WaitReason>) {
        // Only the fact that the user is owed a notification is recorded, with the code.
        log(&LogRecord::new(Level::Info, "autorun_notify")
            .with_phase(PHASE)
            .with_detail(&match reason {
                Some(reason) => format!("{} {}", state.as_str(), reason.as_str()),
                None => state.as_str().to_owned(),
            }));
    }
}

fn bound_thread(plan: &AutoRunPlan) -> Result<&str> {
    plan.binding
        .as_ref()
        .map(|binding| binding.thread_id.as_str())
        .ok_or_else(|| internal("the plan is not bound"))
}

fn unknown_account() -> TogletError {
    internal("no participant with that id")
}

fn internal(detail: &str) -> TogletError {
    TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None).with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_evidence_names_an_account_or_the_active_one_or_nobody() {
        assert_eq!(
            exhausted_account(Some(&Exhausted::Account("a".to_owned())), Some("b")),
            Some("a".to_owned())
        );
        assert_eq!(
            exhausted_account(Some(&Exhausted::ActiveAccount), Some("b")),
            Some("b".to_owned())
        );
        assert_eq!(
            exhausted_account(Some(&Exhausted::ActiveAccount), None),
            None
        );
        assert_eq!(exhausted_account(None, Some("b")), None);
    }
}
