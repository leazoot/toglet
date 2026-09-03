//! Asking the Codex desktop client to close, and starting it again afterwards.
//!
//! **There is no force-kill here, and there is no code path that can grow one**: the only
//! operation this module performs on another process is a close *request*. If
//! the application declines or is busy, the switch stops - it does not escalate.
//!
//! The order matters: close, then switch, then reopen. A client that is still running would
//! keep using the credentials it started with, so replacing them underneath it produces exactly
//! the disagreement between Toglet and Codex that the honest-state rule exists to prevent.
//!
//! **The executable to relaunch comes only from a process that was running.** There is no disk
//! search and no hard-coded path - `%LOCALAPPDATA%\OpenAI\Codex\bin\<content hash>\` changes
//! with every version, so depending on it is forbidden.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::probe::{ClientKind, RunningClient};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// How long a client is given to close itself before the switch is abandoned.
///
/// Generous on purpose: a desktop application that is saving state should not be cut short, and
/// the consequence of waiting is a slower switch, while the consequence of hurrying is the
/// force-kill this module refuses to perform.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(20);

/// What a close request achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitOutcome {
    /// The process is gone.
    Exited,
    /// It is still running. Never followed by a force-kill.
    StillRunning,
    /// There was no such process by the time the request went out.
    NotFound,
}

/// Closing and starting a client. Implemented per platform; faked in tests.
pub trait ClientRestart {
    /// Asks the process to close, and waits up to `timeout` for it to.
    fn request_quit(&self, pid: u32, timeout: Duration) -> QuitOutcome;

    /// Starts `executable` again.
    fn launch(&self, executable: &Path) -> Result<()>;
}

/// What to do about running clients around a switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartPlan {
    /// Nothing to close, and so nothing to reopen. The new credentials apply the next time
    /// Codex is started.
    NothingRunning,
    /// Close these processes, then start these executables again.
    CloseThenReopen(Vec<RestartTarget>),
}

/// One client to close and start again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartTarget {
    pub pid: u32,
    /// Read from the running process. The only permitted source.
    pub executable: PathBuf,
}

/// Decides what to close and what to reopen, from what the probe found.
///
/// Only the shared managed runtime is a candidate: a CLI or an editor session blocks the switch
/// in the pre-checks and never reaches this point.
pub fn plan(clients: &[RunningClient]) -> RestartPlan {
    let targets: Vec<RestartTarget> = clients
        .iter()
        .filter(|client| client.kind == ClientKind::ManagedRuntime)
        .map(|client| RestartTarget {
            pid: client.pid,
            executable: client.executable.clone(),
        })
        .collect();

    if targets.is_empty() {
        RestartPlan::NothingRunning
    } else {
        RestartPlan::CloseThenReopen(targets)
    }
}

/// Asks every client in the plan to close, before anything is replaced.
///
/// Returns a shutdown-timeout error on the first one that does not, and the caller stops
/// there: the credentials have not been touched at that point, so stopping costs nothing.
pub fn close(restart: &dyn ClientRestart, plan: &RestartPlan, timeout: Duration) -> Result<()> {
    let RestartPlan::CloseThenReopen(targets) = plan else {
        return Ok(());
    };

    for target in targets {
        match restart.request_quit(target.pid, timeout) {
            // Already gone is the state the caller wanted.
            QuitOutcome::Exited | QuitOutcome::NotFound => {}
            QuitOutcome::StillRunning => {
                return Err(TogletError::new(
                    ErrorCode::ClientShutdownTimeout,
                    Phase::Restart,
                    true,
                    UserAction::CloseCodexClient,
                )
                .with_detail("the client did not close within the timeout"));
            }
        }
    }
    Ok(())
}

/// What became of the client after a switch that already succeeded.
///
/// Every variant except [`Self::Reopened`] means the account changed **and** the user still has
/// something to do. None of them may be collapsed into a plain success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientOutcome {
    /// Nothing had been running, so nothing was closed or started. The new credentials apply
    /// the next time Codex is started.
    NothingWasRunning,
    /// Closed before the switch and started again after it.
    Reopened,
    /// Closed, but could not be started again. The user opens it themselves.
    ClosedNotReopened { reason: ErrorCode },
    /// Closed and deliberately left closed, because the user turned the reopen off. Not a
    /// failure, and reported as its own thing so the interface does not have to describe a
    /// choice as a problem.
    ClosedByChoice,
}

impl ClientOutcome {
    /// Whether the client is running the new credentials right now.
    ///
    /// `false` does not mean the switch failed - the account did change. It means the user has
    /// to start Codex before the change is visible, which is the second half of the two-part
    /// result.
    pub fn client_is_up_to_date(&self) -> bool {
        matches!(self, Self::Reopened)
    }
}

/// Starts the clients again after a verified switch.
///
/// Failure is reported, never thrown: the switch has already happened and been verified, and
/// turning "could not reopen your editor" into a failed switch would misdescribe what is on
/// disk.
pub fn reopen(restart: &dyn ClientRestart, plan: &RestartPlan) -> ClientOutcome {
    let RestartPlan::CloseThenReopen(targets) = plan else {
        return ClientOutcome::NothingWasRunning;
    };

    for target in targets {
        if let Err(error) = restart.launch(&target.executable) {
            return ClientOutcome::ClosedNotReopened {
                reason: error.code(),
            };
        }
    }
    ClientOutcome::Reopened
}

/// The platform implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClientRestart;

impl SystemClientRestart {
    pub fn new() -> Self {
        Self
    }
}

impl ClientRestart for SystemClientRestart {
    fn request_quit(&self, pid: u32, timeout: Duration) -> QuitOutcome {
        platform::request_quit(pid, timeout)
    }

    fn launch(&self, executable: &Path) -> Result<()> {
        // No shell, no arguments, no environment: the path came from a running process and is
        // passed to the OS as one argument.
        std::process::Command::new(executable)
            .spawn()
            .map(drop)
            .map_err(|error| {
                TogletError::new(ErrorCode::Internal, Phase::Restart, true, UserAction::None)
                    .with_detail(&error.to_string())
            })
    }
}

#[cfg(windows)]
mod platform {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };

    use super::QuitOutcome;

    /// Collects the windows belonging to one process while `EnumWindows` walks them all.
    struct Search {
        pid: u32,
        windows: Vec<HWND>,
    }

    pub fn request_quit(pid: u32, timeout: Duration) -> QuitOutcome {
        // SAFETY: the handle is checked and closed below; `PROCESS_SYNCHRONIZE` is the least
        // access that allows waiting, and grants nothing that could terminate the process.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return QuitOutcome::NotFound;
        }
        let process = OwnedHandle(handle);

        for window in windows_of(pid) {
            // `WM_CLOSE` is a request. The application decides what to do with it - saving,
            // prompting, or refusing - which is exactly the behaviour that is wanted.
            // SAFETY: the window handle came from `EnumWindows` in this call.
            unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
        }

        let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: the handle is live for the duration of the wait.
        match unsafe { WaitForSingleObject(process.0, milliseconds) } {
            WAIT_OBJECT_0 => QuitOutcome::Exited,
            WAIT_TIMEOUT => QuitOutcome::StillRunning,
            // Any other result means the wait itself could not be trusted. Reporting "still
            // running" is the conservative answer: it stops the switch instead of replacing
            // credentials under a process whose state is unknown.
            _ => QuitOutcome::StillRunning,
        }
    }

    fn windows_of(pid: u32) -> Vec<HWND> {
        let mut search = Search {
            pid,
            windows: Vec::new(),
        };
        // SAFETY: the pointer is valid for the duration of the call, and the callback only
        // dereferences it while `EnumWindows` is on the stack.
        unsafe {
            EnumWindows(
                Some(collect),
                std::ptr::from_mut(&mut search) as isize as LPARAM,
            )
        };
        search.windows
    }

    unsafe extern "system" fn collect(window: HWND, argument: LPARAM) -> i32 {
        let mut owner = 0u32;
        // SAFETY: `window` is supplied by `EnumWindows`, and `owner` is a live local.
        unsafe { GetWindowThreadProcessId(window, &mut owner) };
        // SAFETY: `argument` is the pointer passed to `EnumWindows` just above.
        let search = unsafe { &mut *(argument as *mut Search) };
        if owner == search.pid {
            search.windows.push(window);
        }
        1
    }

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful `OpenProcess` and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::time::Duration;

    use super::QuitOutcome;

    /// Not implemented outside Windows.
    ///
    /// **Unverified platform.** Reporting `StillRunning` stops the switch rather than
    /// replacing credentials under a client whose state is unknown - the same conservative
    /// answer the probe gives.
    pub fn request_quit(_pid: u32, _timeout: Duration) -> QuitOutcome {
        QuitOutcome::StillRunning
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn client(pid: u32, kind: ClientKind, executable: &str) -> RunningClient {
        RunningClient {
            pid,
            kind,
            executable: PathBuf::from(executable),
        }
    }

    /// Records what it was asked to do, and answers however the test says.
    struct FakeRestart {
        quit: QuitOutcome,
        launch_fails: bool,
        launched: RefCell<Vec<PathBuf>>,
        quit_requests: RefCell<Vec<u32>>,
    }

    impl FakeRestart {
        fn new(quit: QuitOutcome) -> Self {
            Self {
                quit,
                launch_fails: false,
                launched: RefCell::new(Vec::new()),
                quit_requests: RefCell::new(Vec::new()),
            }
        }
    }

    impl ClientRestart for FakeRestart {
        fn request_quit(&self, pid: u32, _timeout: Duration) -> QuitOutcome {
            self.quit_requests.borrow_mut().push(pid);
            self.quit
        }

        fn launch(&self, executable: &Path) -> Result<()> {
            self.launched.borrow_mut().push(executable.to_path_buf());
            if self.launch_fails {
                return Err(TogletError::new(
                    ErrorCode::Internal,
                    Phase::Restart,
                    true,
                    UserAction::None,
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn nothing_running_means_nothing_to_close_and_nothing_to_search_for() {
        // No disk search when no process was found.
        assert_eq!(plan(&[]), RestartPlan::NothingRunning);
    }

    #[test]
    fn only_the_desktop_runtime_is_a_restart_candidate() {
        let clients = [
            client(1, ClientKind::ManagedRuntime, "codex.exe"),
            client(2, ClientKind::Cli, "cli-codex.exe"),
            client(3, ClientKind::IdeExtension, "ext-codex.exe"),
        ];

        let RestartPlan::CloseThenReopen(targets) = plan(&clients) else {
            panic!("the desktop runtime must be a candidate");
        };
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].pid, 1);
    }

    #[test]
    fn the_executable_comes_from_the_running_process_and_nowhere_else() {
        let clients = [client(
            7,
            ClientKind::ManagedRuntime,
            r"C:\Users\x\AppData\Local\OpenAI\Codex\bin\8fffe694\codex.exe",
        )];

        let RestartPlan::CloseThenReopen(targets) = plan(&clients) else {
            panic!("expected a candidate");
        };
        assert_eq!(targets[0].executable, clients[0].executable);
    }

    #[test]
    fn a_client_that_closes_lets_the_switch_continue() {
        let restart = FakeRestart::new(QuitOutcome::Exited);
        let plan = plan(&[client(1, ClientKind::ManagedRuntime, "codex.exe")]);

        close(&restart, &plan, SHUTDOWN_TIMEOUT).expect("a closed client is not an error");

        assert_eq!(*restart.quit_requests.borrow(), vec![1]);
    }

    #[test]
    fn a_client_that_had_already_exited_is_not_an_error() {
        let restart = FakeRestart::new(QuitOutcome::NotFound);
        let plan = plan(&[client(1, ClientKind::ManagedRuntime, "codex.exe")]);

        close(&restart, &plan, SHUTDOWN_TIMEOUT).expect("gone is the state that was wanted");
    }

    #[test]
    fn a_client_that_will_not_close_stops_the_switch_rather_than_being_killed() {
        let restart = FakeRestart::new(QuitOutcome::StillRunning);
        let plan = plan(&[client(1, ClientKind::ManagedRuntime, "codex.exe")]);

        let error = close(&restart, &plan, SHUTDOWN_TIMEOUT)
            .expect_err("a client that stayed up must stop the switch");

        assert_eq!(error.code(), ErrorCode::ClientShutdownTimeout);
        assert!(
            restart.launched.borrow().is_empty(),
            "nothing may be started when the close failed"
        );
    }

    #[test]
    fn reopening_starts_exactly_what_was_closed() {
        let restart = FakeRestart::new(QuitOutcome::Exited);
        let clients = [client(1, ClientKind::ManagedRuntime, "codex.exe")];
        let plan = plan(&clients);

        assert_eq!(reopen(&restart, &plan), ClientOutcome::Reopened);
        assert_eq!(*restart.launched.borrow(), vec![PathBuf::from("codex.exe")]);
    }

    #[test]
    fn a_client_that_cannot_be_started_again_is_reported_and_not_hidden() {
        let mut restart = FakeRestart::new(QuitOutcome::Exited);
        restart.launch_fails = true;
        let plan = plan(&[client(1, ClientKind::ManagedRuntime, "codex.exe")]);

        let outcome = reopen(&restart, &plan);

        assert!(matches!(outcome, ClientOutcome::ClosedNotReopened { .. }));
        assert!(
            !outcome.client_is_up_to_date(),
            "the account changed, but Codex is not running it yet"
        );
    }

    #[test]
    fn nothing_running_reports_that_the_client_still_has_to_be_started() {
        let restart = FakeRestart::new(QuitOutcome::Exited);

        let outcome = reopen(&restart, &RestartPlan::NothingRunning);

        assert_eq!(outcome, ClientOutcome::NothingWasRunning);
        assert!(
            !outcome.client_is_up_to_date(),
            "only an actually reopened client counts as up to date"
        );
    }

    /// No content-hash directory may be compiled in.
    #[test]
    fn no_client_path_is_hard_coded_in_the_restart_path() {
        let source = include_str!("restart.rs");
        // Comments are stripped: the module documentation names the directory in order to
        // explain why it must never be compiled in.
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();

        for forbidden in ["openai\\codex\\bin", "openai/codex/bin", "programs\\codex"] {
            assert!(
                !implementation.contains(forbidden),
                "`{forbidden}` is a path that changes with every Codex version"
            );
        }
    }

    /// There is no force-kill, and no way for one to appear unnoticed.
    #[test]
    fn nothing_in_the_restart_path_terminates_a_process() {
        let source = include_str!("restart.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part");
        let code = implementation
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        for forbidden in [
            "TerminateProcess",
            "PROCESS_TERMINATE",
            "taskkill",
            "SIGKILL",
            ".kill(",
        ] {
            assert!(
                !code.contains(forbidden),
                "`{forbidden}` would make force-killing reachable"
            );
        }
    }
}
