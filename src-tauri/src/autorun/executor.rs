//! The executor session: Toglet's own app server on the user's Codex home, continuing the
//! bound thread. It confirms the identity, resumes the thread, refuses a thread that is busy or
//! waiting on a person, and starts one turn. The instruction is only ever a `turn/start` payload,
//! never logged. A session lost under a running turn is reported as a network interruption.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::driver::Resumed;
use super::machine::Fact;
use crate::accounts::{AccountIdentity, fingerprint, onboarding, read_auth_facts};
use crate::app_server::{
    AppServerClient, AppServerSession, CodexBinary, ServerEvent, ThreadSummary, TurnStatus,
};
use crate::codex_home::ServerHome;
use crate::diagnostics::{ErrorCode, LogRecord, Phase, Result, TogletError, UserAction, log};
use crate::switching::mismatch;

const PHASE: Phase = Phase::Autorun;

/// How long a turn is given to actually end after being asked to stop.
const INTERRUPT_WAIT: Duration = Duration::from_secs(10);

/// One look at the server while waiting for that.
const INTERRUPT_SLICE: Duration = Duration::from_secs(1);

/// Why a turn is being started, which decides what happens to a thread that is parked.
///
/// Deliberately not `Debug`: both forms carry text a person wrote, which must not be able to
/// quote itself into a log line or an error detail.
#[derive(Clone, Copy)]
pub enum Continuation<'a> {
    /// The plan's stored instruction, started because quota came back. A thread that is busy or
    /// waiting on a person is left exactly as it is - nobody asked for this.
    Automatic(&'a str),
    /// A sentence the user typed on their phone, answering what is on their screen. The turn
    /// holding the question is stopped first, so the sentence can start one of its own.
    Steered(&'a str),
}

impl<'a> Continuation<'a> {
    fn text(self) -> &'a str {
        match self {
            Self::Automatic(text) | Self::Steered(text) => text,
        }
    }
}

pub struct Executor {
    binary: CodexBinary,
    default_home: PathBuf,
    /// Open while a turn is being run or waited for.
    session: Option<AppServerSession>,
    /// The turn the open session listens to, so a lost session can say which turn it lost.
    turn_id: Option<String>,
    /// The agent's last message in the bound session, already cut to an excerpt.
    ///
    /// Kept here and nowhere else: it is session content, so it must not reach the plan (which
    /// is written to disk) or the view (which crosses to the desktop interface).
    last_excerpt: Option<String>,
}

impl Executor {
    pub fn new(binary: CodexBinary, default_home: &Path) -> Self {
        Self {
            binary,
            default_home: default_home.to_path_buf(),
            session: None,
            turn_id: None,
            last_excerpt: None,
        }
    }

    /// The agent's last message, for the one caller allowed to send it out (`remote`), sealed.
    pub fn last_excerpt(&self) -> Option<String> {
        self.last_excerpt.clone()
    }

    /// The open session's subprocess, for the client probe's exclusion list.
    pub fn pid(&self) -> Option<u32> {
        self.session.as_ref().map(AppServerSession::pid)
    }

    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// The bound thread's last turn, as a fact, via the open session or a short-lived one.
    pub fn read_thread(&mut self, thread_id: &str) -> Result<Fact> {
        if let Some(session) = self.session.as_mut() {
            let summary = session.read_thread(thread_id)?;
            self.remember_excerpt(
                summary
                    .last_turn()
                    .and_then(|turn| turn.agent_excerpt.clone()),
            );
            return Ok(fact_of(&summary));
        }
        let mut session = self.open()?;
        let summary = session.read_thread(thread_id);
        // Closed on both paths, so a failed read still leaves no subprocess behind.
        let closed = session.close();
        let summary = summary?;
        closed?;
        self.remember_excerpt(
            summary
                .last_turn()
                .and_then(|turn| turn.agent_excerpt.clone()),
        );
        Ok(fact_of(&summary))
    }

    fn remember_excerpt(&mut self, excerpt: Option<String>) {
        remember(&mut self.last_excerpt, excerpt);
    }

    /// Confirms the identity, resumes the thread, checks its last turn and starts the one
    /// continuation turn. `after_turn` is the exhausted turn being continued from.
    ///
    /// What happens to a thread that is busy or parked depends on why the turn is being
    /// started; see [`Continuation`].
    pub fn resume(
        &mut self,
        thread_id: &str,
        continuation: Continuation<'_>,
        expected_fingerprint: &str,
        after_turn: Option<&str>,
    ) -> Result<Resumed> {
        if self.session.is_none() {
            self.session = Some(self.open()?);
        }
        // Checked fresh on every attempt, never cached: this is the second identity check.
        self.confirm_identity(expected_fingerprint)?;

        let session = self
            .session
            .as_mut()
            .ok_or_else(|| internal("the executor session vanished"))?;
        let thread = session.resume_thread(thread_id)?;

        let running_turn = thread
            .last_turn()
            .filter(|last| last.status == TurnStatus::InProgress)
            .map(|last| last.id.clone());
        let moved_on = thread
            .last_turn()
            .is_some_and(|last| after_turn.is_some_and(|expected| expected != last.id));

        match continuation {
            Continuation::Automatic(_) => {
                if thread.status.is_waiting_on_human() {
                    return Ok(Resumed::WaitingOnHuman);
                }
                if let Some(turn_id) = running_turn {
                    self.turn_id = Some(turn_id);
                    return Ok(Resumed::TurnInProgress);
                }
                if moved_on {
                    return Ok(Resumed::ThreadChanged);
                }
            }
            Continuation::Steered(_) => {
                // Checked before anything is stopped: a thread somebody else has moved on with
                // must not be interrupted.
                if moved_on {
                    return Ok(Resumed::ThreadChanged);
                }
                // The turn holding the question cannot be steered - `turn/start` against it
                // fails with `activeTurnNotSteerable` - and nothing in Toglet may answer the
                // blocking request on the user's behalf. So it is stopped, and the sentence
                // starts a turn of its own.
                if let Some(turn_id) = running_turn
                    && !stop_turn(session, thread_id, &turn_id)?
                {
                    // It did not end in time. Nothing was started, and that is what the caller
                    // is told; the sentence is kept for the retry.
                    self.turn_id = Some(turn_id);
                    return Ok(Resumed::TurnInProgress);
                }
            }
        }

        let started = session.start_turn(thread_id, continuation.text())?;
        self.turn_id = Some(started.id.clone());
        Ok(Resumed::Started {
            turn_id: started.id,
        })
    }

    /// Waits up to `wait` for the running turn to report something; `None` is "nothing yet".
    /// A dead session is dropped and reported as the turn ending on a network failure.
    pub fn poll(&mut self, wait: Duration) -> Option<Fact> {
        let session = self.session.as_mut()?;
        match session.next_event(wait) {
            Ok(ServerEvent::TurnCompleted { thread_id: _, turn }) => {
                let excerpt = turn.agent_excerpt.clone();
                let fact = Fact::from_turn(&turn);
                self.remember_excerpt(excerpt);
                Some(fact)
            }
            Ok(ServerEvent::ThreadStatusChanged { status, .. }) => {
                status.is_waiting_on_human().then_some(Fact::WaitingOnHuman)
            }
            // A request only a person can answer. Toglet never answers it.
            Ok(ServerEvent::ServerRequest { .. }) => Some(Fact::WaitingOnHuman),
            Ok(ServerEvent::Other { .. }) => None,
            Err(error) if error.code() == ErrorCode::AppServerUnresponsive => None,
            Err(error) => {
                log(&LogRecord::from_error(
                    "autorun_executor_session_lost",
                    &error,
                ));
                self.session = None;
                let turn_id = self.turn_id.take().unwrap_or_default();
                Some(Fact::TurnEnded {
                    turn_id,
                    interruption: super::machine::Interruption::Network,
                })
            }
        }
    }

    /// Closes the open session. A failed close is logged; the guard never leaves the process
    /// behind.
    pub fn stop(&mut self) {
        self.turn_id = None;
        if let Some(session) = self.session.take()
            && let Err(error) = session.close()
        {
            log(&LogRecord::from_error(
                "autorun_executor_close_failed",
                &error,
            ));
        }
    }

    fn open(&self) -> Result<AppServerSession> {
        let home = ServerHome::Default {
            path: self.default_home.clone(),
            phase: PHASE,
        };
        AppServerSession::open(AppServerClient::start(&self.binary, home)?)
    }

    /// `account/read` on the executor session must identify the chosen account.
    ///
    /// Both the server's live answer and the credential file must match the plan's choice; a file
    /// fingerprinted by address (older sign-ins) is matched by the address the server reports.
    fn confirm_identity(&mut self, expected_fingerprint: &str) -> Result<()> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| internal("no executor session to confirm"))?;
        let identity = session.read_account()?.ok_or_else(|| {
            TogletError::new(ErrorCode::AuthExpired, PHASE, false, UserAction::ReLogin)
                .with_detail("the executor session identifies no account")
        })?;

        let credentials = onboarding::read_default_credentials(&self.default_home, PHASE)?;
        let facts = read_auth_facts(&credentials);
        if identity_matches(
            &identity,
            facts.and_then(|f| f.fingerprint),
            expected_fingerprint,
        ) {
            Ok(())
        } else {
            Err(mismatch(PHASE))
        }
    }
}

/// Asks the running turn to stop and waits for it to actually end, reporting whether it did.
///
/// Both halves are load-bearing. `turn/start` is refused while the old turn still runs, and the
/// interrupted turn's own `turn/completed` would otherwise arrive later and be read as the user
/// stopping the plan - pausing the very continuation the sentence just asked for.
fn stop_turn(session: &mut AppServerSession, thread_id: &str, turn_id: &str) -> Result<bool> {
    session.interrupt_turn(thread_id, turn_id)?;
    let deadline = Instant::now() + INTERRUPT_WAIT;
    while Instant::now() < deadline {
        match session.next_event(INTERRUPT_SLICE) {
            Ok(ServerEvent::TurnCompleted { turn, .. }) if turn.id == turn_id => return Ok(true),
            // Anything else the server says while it winds the turn down, including the
            // question's own blocking request, which is simply dropped with the turn.
            Ok(_) => {}
            Err(error) if error.code() == ErrorCode::AppServerUnresponsive => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn identity_matches(
    identity: &AccountIdentity,
    file_fingerprint: Option<String>,
    expected: &str,
) -> bool {
    if file_fingerprint.as_deref() == Some(expected) {
        return true;
    }
    identity
        .email()
        .map(fingerprint::from_email)
        .is_some_and(|by_email| by_email == expected)
}

/// The thread's state as a fact for the machine: waiting on a person beats everything, then
/// the last turn says what happened.
/// Keeps the latest excerpt, and keeps the previous one when a read carried none.
///
/// A turn that ran a command and said nothing must not blank what the agent last said: the
/// phone would then show an empty preview for a session that has plenty to answer.
fn remember(slot: &mut Option<String>, excerpt: Option<String>) {
    if excerpt.is_some() {
        *slot = excerpt;
    }
}

fn fact_of(summary: &ThreadSummary) -> Fact {
    if summary.status.is_waiting_on_human() {
        return Fact::WaitingOnHuman;
    }
    match summary.last_turn() {
        Some(turn) => Fact::from_turn(turn),
        // No turn has ever run: reported as in progress, meaning nothing to act on.
        None => Fact::TurnInProgress,
    }
}

fn internal(detail: &str) -> TogletError {
    TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None).with_detail(detail)
}

impl Drop for Executor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_thing_the_agent_said_replaces_the_one_before_it() {
        let mut slot = Some("first".to_owned());
        remember(&mut slot, Some("second".to_owned()));
        assert_eq!(slot.as_deref(), Some("second"));
    }

    /// A turn can run a command and say nothing. That must not blank the preview.
    #[test]
    fn a_turn_that_said_nothing_keeps_what_the_agent_last_said() {
        let mut slot = Some("the plan looks right".to_owned());
        remember(&mut slot, None);
        assert_eq!(slot.as_deref(), Some("the plan looks right"));
    }

    #[test]
    fn nothing_said_yet_stays_nothing() {
        let mut slot = None;
        remember(&mut slot, None);
        assert_eq!(slot, None);
    }

    /// The excerpt is session content. It may live in memory and go out sealed, but it must
    /// never be a **field** of the plan (written to disk) or of the view (crosses to the
    /// desktop interface).
    ///
    /// The scan is of the two struct declarations rather than of whole files. Scanning the
    /// files was this test's first form and it failed: `AutoRun` keeps the excerpt in memory
    /// and hands it to `remote` through an accessor, which is the sanctioned path, not a leak.
    /// The keyword is `excerpt` rather than `agent_excerpt`, so any spelling is caught.
    #[test]
    fn the_excerpt_is_a_field_of_neither_the_plan_nor_the_view() {
        for (what, source, declaration) in [
            (
                "the plan",
                include_str!("plan.rs"),
                "pub struct AutoRunPlan {",
            ),
            (
                "the view",
                include_str!("../commands/autorun.rs"),
                "pub struct AutoRunView {",
            ),
        ] {
            let block = source
                .split(declaration)
                .nth(1)
                .unwrap_or_else(|| panic!("{declaration} should exist"))
                .split("\n}")
                .next()
                .expect("the struct ends");
            assert!(
                !block.contains("excerpt"),
                "{what} must not carry the agent excerpt:\n{block}"
            );
        }
    }
    use crate::app_server::{ActiveFlag, ThreadStatus, TurnErrorKind, TurnRecord};
    use crate::autorun::machine::Interruption;

    fn chatgpt(email: &str) -> AccountIdentity {
        AccountIdentity::Chatgpt {
            email: email.to_owned(),
            plan_type: None,
        }
    }

    #[test]
    fn the_file_fingerprint_decides_when_it_is_there() {
        let expected = fingerprint::from_account_id("acct-1");
        assert!(identity_matches(
            &chatgpt("someone@example.com"),
            Some(expected.clone()),
            &expected
        ));
        assert!(!identity_matches(
            &chatgpt("someone@example.com"),
            Some(fingerprint::from_account_id("acct-2")),
            &expected
        ));
    }

    #[test]
    fn an_address_fingerprint_is_matched_by_the_reported_address() {
        let expected = fingerprint::from_email("someone@example.com");
        assert!(identity_matches(
            &chatgpt("Someone@Example.com"),
            None,
            &expected
        ));
        assert!(!identity_matches(
            &chatgpt("other@example.com"),
            None,
            &expected
        ));
        assert!(!identity_matches(&AccountIdentity::ApiKey, None, &expected));
    }

    fn thread_with(status: ThreadStatus, turns: Vec<TurnRecord>) -> ThreadSummary {
        ThreadSummary {
            id: "thread".to_owned(),
            cwd: PathBuf::from("/x/project"),
            cli_version: "0.153.4".to_owned(),
            created_at: 0,
            updated_at: 0,
            title: None,
            preview: None,
            status,
            turns,
        }
    }

    fn turn(id: &str, status: TurnStatus, error: Option<TurnErrorKind>) -> TurnRecord {
        TurnRecord {
            id: id.to_owned(),
            status,
            error,
            started_at: None,
            completed_at: None,
            agent_excerpt: None,
        }
    }

    #[test]
    fn waiting_on_a_person_outranks_the_last_turn() {
        let summary = thread_with(
            ThreadStatus::Active(vec![ActiveFlag::WaitingOnApproval]),
            vec![turn("t1", TurnStatus::Completed, None)],
        );
        assert_eq!(fact_of(&summary), Fact::WaitingOnHuman);
    }

    #[test]
    fn the_last_turn_becomes_its_class() {
        let summary = thread_with(
            ThreadStatus::Idle,
            vec![
                turn("t1", TurnStatus::Completed, None),
                turn(
                    "t2",
                    TurnStatus::Failed,
                    Some(TurnErrorKind::UsageLimitExceeded),
                ),
            ],
        );
        assert_eq!(
            fact_of(&summary),
            Fact::TurnEnded {
                turn_id: "t2".to_owned(),
                interruption: Interruption::Exhausted
            }
        );
        assert_eq!(
            fact_of(&thread_with(ThreadStatus::Idle, Vec::new())),
            Fact::TurnInProgress
        );
    }
}
