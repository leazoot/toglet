//! The executor session: Toglet's own app server on the user's Codex home, continuing the
//! bound thread. It confirms the identity, resumes the thread, refuses a thread that is busy or
//! waiting on a person, and starts one turn. The instruction is only ever a `turn/start` payload,
//! never logged. A session lost under a running turn is reported as a network interruption.

use std::path::{Path, PathBuf};
use std::time::Duration;

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

pub struct Executor {
    binary: CodexBinary,
    default_home: PathBuf,
    /// Open while a turn is being run or waited for.
    session: Option<AppServerSession>,
    /// The turn the open session listens to, so a lost session can say which turn it lost.
    turn_id: Option<String>,
}

impl Executor {
    pub fn new(binary: CodexBinary, default_home: &Path) -> Self {
        Self {
            binary,
            default_home: default_home.to_path_buf(),
            session: None,
            turn_id: None,
        }
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
            return session
                .read_thread(thread_id)
                .map(|summary| fact_of(&summary));
        }
        let mut session = self.open()?;
        let summary = session.read_thread(thread_id);
        // Closed on both paths, so a failed read still leaves no subprocess behind.
        let closed = session.close();
        let summary = summary?;
        closed?;
        Ok(fact_of(&summary))
    }

    /// Confirms the identity, resumes the thread, checks its last turn and starts the one
    /// continuation turn. `after_turn` is the exhausted turn being continued from.
    pub fn resume(
        &mut self,
        thread_id: &str,
        instruction: &str,
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

        if thread.status.is_waiting_on_human() {
            return Ok(Resumed::WaitingOnHuman);
        }
        if let Some(last) = thread.last_turn() {
            if last.status == TurnStatus::InProgress {
                self.turn_id = Some(last.id.clone());
                return Ok(Resumed::TurnInProgress);
            }
            if after_turn.is_some_and(|expected| expected != last.id) {
                return Ok(Resumed::ThreadChanged);
            }
        }

        let started = session.start_turn(thread_id, instruction)?;
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
            Ok(ServerEvent::TurnCompleted { thread_id: _, turn }) => Some(Fact::from_turn(&turn)),
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
