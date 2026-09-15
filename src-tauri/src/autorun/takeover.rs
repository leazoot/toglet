//! Taking the bound session over from the desktop app, and giving it back.
//!
//! The desktop app is asked to quit, never force-killed; a CLI or editor session stops the
//! takeover instead of being closed. What was closed is reopened once, on release, if wanted.

use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::process::{
    self, ClientKind, ClientOutcome, ClientPresence, ClientProbe, ClientRestart, RestartPlan,
    SHUTDOWN_TIMEOUT,
};
use crate::switching::{ClientVerdict, verdict};

const PHASE: Phase = Phase::Autorun;

/// What the takeover closed, and therefore owes a reopen.
#[derive(Debug, Default)]
pub struct Takeover {
    plan: Option<RestartPlan>,
}

impl Takeover {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes sure nothing else writes to the session: asks a running desktop app to quit, and
    /// refuses if a CLI or editor session is running or presence is unknown. `exclude` lists
    /// Toglet's own app servers.
    pub fn acquire(
        &mut self,
        probe: &dyn ClientProbe,
        restart: &dyn ClientRestart,
        exclude: &[u32],
    ) -> Result<()> {
        let presence = probe.running_clients(exclude);
        let plan = match verdict(&presence) {
            ClientVerdict::Blocked => {
                // Record which client kinds were seen, never a path or a pid, so a refusal is
                // diagnosable.
                let seen = kinds_seen(&presence);
                log(&LogRecord::new(Level::Warn, "autorun_takeover_refused")
                    .with_phase(PHASE)
                    .with_detail(&seen));
                return Err(refused(&format!(
                    "a CLI or editor session is using Codex ({seen})"
                )));
            }
            ClientVerdict::Unknown => {
                return Err(refused("running clients could not be determined"));
            }
            ClientVerdict::Clear => RestartPlan::NothingRunning,
            ClientVerdict::DesktopOnly => match presence {
                ClientPresence::Known(clients) => process::plan(&clients),
                ClientPresence::Unknown => RestartPlan::NothingRunning,
            },
        };
        // The switch's own graceful quit and timeout; a desktop app that stays reports
        // `client_shutdown_timeout` and nothing is killed.
        process::close(restart, &plan, SHUTDOWN_TIMEOUT)?;
        self.remember(plan);
        Ok(())
    }

    /// Notes what a step closed, so release can reopen it. A step that closed nothing does
    /// not erase what an earlier one did.
    pub fn remember(&mut self, plan: RestartPlan) {
        if matches!(plan, RestartPlan::CloseThenReopen(_)) || self.plan.is_none() {
            self.plan = Some(plan);
        }
    }

    /// The executor has let go: start the desktop app again if this closed it and the user
    /// wants it back. What happened is recorded either way.
    pub fn release(&mut self, restart: &dyn ClientRestart, reopen: bool) -> ClientOutcome {
        let plan = self.plan.take().unwrap_or(RestartPlan::NothingRunning);
        let outcome = if reopen {
            process::reopen(restart, &plan)
        } else if matches!(plan, RestartPlan::CloseThenReopen(_)) {
            ClientOutcome::ClosedByChoice
        } else {
            ClientOutcome::NothingWasRunning
        };
        log(&LogRecord::new(Level::Info, "autorun_session_released")
            .with_phase(PHASE)
            .with_detail(match &outcome {
                ClientOutcome::NothingWasRunning => "nothing was running",
                ClientOutcome::Reopened => "desktop app reopened",
                ClientOutcome::ClosedNotReopened { .. } => "desktop app could not be reopened",
                ClientOutcome::ClosedByChoice => "desktop app left closed by setting",
            }));
        outcome
    }
}

/// `cli=1 desktop_app=1`: the kinds the probe reported, counted. Nothing that identifies a
/// process.
fn kinds_seen(presence: &ClientPresence) -> String {
    let ClientPresence::Known(clients) = presence else {
        return "unknown".to_owned();
    };
    let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    for client in clients {
        *counts.entry(kind_name(client.kind)).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(kind, count)| format!("{kind}={count}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_name(kind: ClientKind) -> &'static str {
    match kind {
        ClientKind::Cli => "cli",
        ClientKind::IdeExtension => "ide_extension",
        ClientKind::ManagedRuntime => "managed_runtime",
        ClientKind::DesktopApp => "desktop_app",
        ClientKind::Unrecognised => "unrecognised",
    }
}

fn refused(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::ClientRunning,
        PHASE,
        true,
        UserAction::CloseCodexClient,
    )
    .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::*;
    use crate::process::{ClientKind, QuitOutcome, RunningClient};

    struct Probe(ClientPresence);

    impl ClientProbe for Probe {
        fn running_clients(&self, _exclude: &[u32]) -> ClientPresence {
            self.0.clone()
        }
    }

    #[derive(Default)]
    struct Restart {
        quit: Option<QuitOutcome>,
        quits: RefCell<Vec<u32>>,
        launches: RefCell<Vec<PathBuf>>,
    }

    impl ClientRestart for Restart {
        fn request_quit(&self, pid: u32, _timeout: Duration) -> QuitOutcome {
            self.quits.borrow_mut().push(pid);
            self.quit.unwrap_or(QuitOutcome::Exited)
        }

        fn launch(&self, executable: &Path) -> Result<()> {
            self.launches.borrow_mut().push(executable.to_path_buf());
            Ok(())
        }
    }

    fn client(pid: u32, kind: ClientKind) -> RunningClient {
        RunningClient {
            pid,
            kind,
            executable: PathBuf::from("/Applications/Codex.app/Contents/MacOS/codex"),
        }
    }

    #[test]
    fn a_running_desktop_app_is_asked_to_quit_and_reopened_on_release() {
        let probe = Probe(ClientPresence::Known(vec![client(
            41,
            ClientKind::DesktopApp,
        )]));
        let restart = Restart::default();
        let mut takeover = Takeover::new();

        takeover
            .acquire(&probe, &restart, &[])
            .expect("the desktop app quits");
        assert_eq!(*restart.quits.borrow(), vec![41]);
        assert!(restart.launches.borrow().is_empty(), "not before release");

        assert_eq!(takeover.release(&restart, true), ClientOutcome::Reopened);
        assert_eq!(restart.launches.borrow().len(), 1);
    }

    #[test]
    fn release_honours_the_reopen_setting() {
        let probe = Probe(ClientPresence::Known(vec![client(
            41,
            ClientKind::DesktopApp,
        )]));
        let restart = Restart::default();
        let mut takeover = Takeover::new();
        takeover.acquire(&probe, &restart, &[]).expect("quits");

        assert_eq!(
            takeover.release(&restart, false),
            ClientOutcome::ClosedByChoice
        );
        assert!(restart.launches.borrow().is_empty());
    }

    // No force-kill: the timeout is reported and the takeover stops.
    #[test]
    fn a_desktop_app_that_will_not_quit_is_reported_not_killed() {
        let probe = Probe(ClientPresence::Known(vec![client(
            41,
            ClientKind::DesktopApp,
        )]));
        let restart = Restart {
            quit: Some(QuitOutcome::StillRunning),
            ..Restart::default()
        };
        let mut takeover = Takeover::new();

        let error = takeover
            .acquire(&probe, &restart, &[])
            .expect_err("the takeover stops");
        assert_eq!(error.code(), ErrorCode::ClientShutdownTimeout);
        assert_eq!(
            takeover.release(&restart, true),
            ClientOutcome::NothingWasRunning
        );
    }

    // A CLI or editor session is never closed for this.
    #[test]
    fn a_cli_or_editor_session_refuses_the_takeover() {
        for kind in [ClientKind::Cli, ClientKind::IdeExtension] {
            let probe = Probe(ClientPresence::Known(vec![client(7, kind)]));
            let restart = Restart::default();
            let mut takeover = Takeover::new();
            let error = takeover
                .acquire(&probe, &restart, &[])
                .expect_err("refused");
            assert_eq!(error.code(), ErrorCode::ClientRunning);
            assert!(
                restart.quits.borrow().is_empty(),
                "{kind:?} was asked to quit"
            );
        }
    }

    #[test]
    fn an_unknown_answer_from_the_probe_refuses_rather_than_guesses() {
        let probe = Probe(ClientPresence::Unknown);
        let restart = Restart::default();
        let mut takeover = Takeover::new();
        let error = takeover
            .acquire(&probe, &restart, &[])
            .expect_err("refused");
        assert_eq!(error.code(), ErrorCode::ClientRunning);
    }

    #[test]
    fn nothing_running_means_nothing_to_reopen() {
        let probe = Probe(ClientPresence::Known(Vec::new()));
        let restart = Restart::default();
        let mut takeover = Takeover::new();
        takeover.acquire(&probe, &restart, &[]).expect("clear");
        assert_eq!(
            takeover.release(&restart, true),
            ClientOutcome::NothingWasRunning
        );
        assert!(restart.launches.borrow().is_empty());
    }

    // A switch closed the desktop app before the takeover looked: the second look finds
    // nothing running and must not forget what the switch closed.
    #[test]
    fn a_plan_from_the_switch_survives_a_later_clear_look() {
        let restart = Restart::default();
        let mut takeover = Takeover::new();
        takeover.remember(process::plan(&[client(41, ClientKind::DesktopApp)]));

        let probe = Probe(ClientPresence::Known(Vec::new()));
        takeover.acquire(&probe, &restart, &[]).expect("clear");

        assert_eq!(takeover.release(&restart, true), ClientOutcome::Reopened);
        assert_eq!(restart.launches.borrow().len(), 1);
    }
}
