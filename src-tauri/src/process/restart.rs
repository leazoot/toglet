//! Asks the Codex desktop client to close, and starts it again afterwards.
//!
//! The only thing done to another process is a close request; a refusal stops the switch
//! rather than escalating. The relaunch path comes only from the running process, because the
//! managed runtime's content-hash directory changes with every version.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::probe::{ClientKind, RunningClient};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// How long a client is given to close before the switch is abandoned; generous so an
/// application saving state is not cut short.
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

pub trait ClientRestart {
    /// Asks the process to close, and waits up to `timeout` for it to.
    fn request_quit(&self, pid: u32, timeout: Duration) -> QuitOutcome;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartTarget {
    pub pid: u32,
    /// Read from the running process; the only permitted source.
    pub executable: PathBuf,
}

/// Decides what to close and reopen from what the probe found.
///
/// Only whole applications qualify; CLI and editor sessions are blocked by the pre-checks
/// because a terminal session cannot be put away and brought back.
pub fn plan(clients: &[RunningClient]) -> RestartPlan {
    let targets: Vec<RestartTarget> = clients
        .iter()
        .filter(|client| {
            matches!(
                client.kind,
                ClientKind::ManagedRuntime | ClientKind::DesktopApp
            )
        })
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

/// Asks every client in the plan to close before anything is replaced.
///
/// Stops at the first that does not; the credentials are still untouched at that point.
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
/// Every variant except [`Self::Reopened`] means the user still has something to do, so none
/// may be collapsed into a plain success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientOutcome {
    /// Nothing had been running, so nothing was closed or started. The new credentials apply
    /// the next time Codex is started.
    NothingWasRunning,
    /// Closed before the switch and started again after it.
    Reopened,
    /// Closed, but could not be started again. The user opens it themselves.
    ClosedNotReopened { reason: ErrorCode },
    /// Closed and left closed because the user turned reopening off; not a failure.
    ClosedByChoice,
}

impl ClientOutcome {
    /// Whether the client is running the new credentials right now.
    ///
    /// `false` does not mean the switch failed; the user has to start Codex to see the change.
    pub fn client_is_up_to_date(&self) -> bool {
        matches!(self, Self::Reopened)
    }
}

/// Starts the clients again after a verified switch.
///
/// Failure is reported, never returned as an error: the switch has already been verified.
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
        // A bundled executable is reopened as its application; starting the helper alone would
        // leave a stray process. Elsewhere the executable is started directly.
        #[cfg(target_os = "macos")]
        if let Some(opened) = platform::launch(executable) {
            return opened.map_err(|reason| {
                TogletError::new(ErrorCode::Internal, Phase::Restart, true, UserAction::None)
                    .with_detail(&reason)
            });
        }

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
            // `WM_CLOSE` is a request the application may honour, prompt on, or refuse.
            // SAFETY: the window handle came from `EnumWindows` in this call.
            unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
        }

        let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        // SAFETY: the handle is live for the duration of the wait.
        match unsafe { WaitForSingleObject(process.0, milliseconds) } {
            WAIT_OBJECT_0 => QuitOutcome::Exited,
            WAIT_TIMEOUT => QuitOutcome::StillRunning,
            // Any other result means the wait cannot be trusted; "still running" stops the switch.
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

/// Asking a macOS application to quit, and starting it again.
///
/// The bundled `codex` is not a Launch Services application and only answers to a signal, so
/// the quit goes to its owning application via `NSRunningApplication terminate`, the ordinary
/// Quit. No Apple Event, which would put a permission prompt in front of a switch.
#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{CString, c_char, c_void};
    use std::path::{Path, PathBuf};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::QuitOutcome;
    use crate::process::probe::executable_of;

    /// How often the process is looked at while it is quitting.
    const POLL: Duration = Duration::from_millis(100);

    type Id = *mut c_void;
    type Sel = *mut c_void;

    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    // Linked explicitly for `NSRunningApplication` rather than relying on another dependency.
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    pub fn request_quit(pid: u32, timeout: Duration) -> QuitOutcome {
        let Some(executable) = executable_of(pid) else {
            // Already gone by the time the request went out.
            return QuitOutcome::NotFound;
        };
        let Some(owner) = owning_application(&executable) else {
            // Not inside an application, e.g. a terminal session: it cannot be asked to close.
            return QuitOutcome::StillRunning;
        };
        if !terminate(owner) {
            return QuitOutcome::StillRunning;
        }

        // Wait for both processes: the helper exits first, and Launch Services refuses (-600) to
        // start an application that is still quitting.
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if executable_of(pid).is_none() && executable_of(owner).is_none() {
                return QuitOutcome::Exited;
            }
            thread::sleep(POLL);
        }
        QuitOutcome::StillRunning
    }

    /// Starts an application again from its bundle rather than running the bundled helper alone.
    pub fn launch(executable: &Path) -> Option<Result<(), String>> {
        let bundle = bundle_root(executable)?;
        Some(open_bundle(&bundle))
    }

    /// The `.app` directory a path lies inside, if any.
    fn bundle_root(executable: &Path) -> Option<PathBuf> {
        executable
            .ancestors()
            .find(|ancestor| {
                ancestor
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
            })
            .map(Path::to_path_buf)
    }

    /// The pid of the process running from `<bundle>/Contents/MacOS/` for `executable`'s bundle.
    ///
    /// Found by bundle rather than parent ids, since a helper may be several hops away.
    fn owning_application(executable: &Path) -> Option<u32> {
        let bundle = bundle_root(executable)?;
        let main = bundle.join("Contents").join("MacOS");
        crate::process::probe::running_pids()?
            .into_iter()
            .find(|pid| executable_of(*pid).is_some_and(|path| path.starts_with(&main)))
    }

    /// `[[NSRunningApplication runningApplicationWithProcessIdentifier: pid] terminate]`.
    ///
    /// `false` means no such application or it declined; never that it was forced.
    fn terminate(pid: u32) -> bool {
        let Ok(class_name) = CString::new("NSRunningApplication") else {
            return false;
        };
        let Ok(by_pid) = CString::new("runningApplicationWithProcessIdentifier:") else {
            return false;
        };
        let Ok(quit) = CString::new("terminate") else {
            return false;
        };
        let Ok(pid) = i32::try_from(pid) else {
            return false;
        };

        // SAFETY: each `objc_msgSend` is called through a signature matching its selector, as the
        // runtime requires; class and selector lookups return null on failure, and null is checked.
        unsafe {
            let class = objc_getClass(class_name.as_ptr());
            if class.is_null() {
                return false;
            }
            let lookup: extern "C" fn(Id, Sel, i32) -> Id =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let application = lookup(class, sel_registerName(by_pid.as_ptr()), pid);
            if application.is_null() {
                return false;
            }
            let send: extern "C" fn(Id, Sel) -> bool =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            send(application, sel_registerName(quit.as_ptr()))
        }
    }

    type CFTypeRef = *const c_void;
    type CFURLRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            buffer: *const u8,
            length: isize,
            is_directory: u8,
        ) -> CFURLRef;
        fn CFRelease(cf: CFTypeRef);
    }

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSOpenCFURLRef(url: CFURLRef, launched: *mut CFURLRef) -> i32;
    }

    /// How long to keep asking for the application back, covering the undocumented gap between
    /// the process exiting and Launch Services releasing it.
    const REOPEN_WINDOW: Duration = Duration::from_secs(5);

    fn open_bundle(bundle: &Path) -> Result<(), String> {
        let deadline = Instant::now() + REOPEN_WINDOW;
        let mut last = ask_to_open(bundle)?;
        while last != 0 && Instant::now() < deadline {
            thread::sleep(POLL);
            last = ask_to_open(bundle)?;
        }
        if last == 0 {
            Ok(())
        } else {
            // -600 means the application is not there to be opened, as a still-quitting one
            // answers.
            Err(format!("the application would not start again ({last})"))
        }
    }

    /// One launch request. `Ok(status)` is what Launch Services answered; `Err` means the path
    /// could not be turned into a request at all.
    fn ask_to_open(bundle: &Path) -> Result<i32, String> {
        use std::os::unix::ffi::OsStrExt;

        let bytes = bundle.as_os_str().as_bytes();
        let Ok(length) = isize::try_from(bytes.len()) else {
            return Err("the application path was not a path".to_owned());
        };
        // SAFETY: the bytes live until after the call, which reads exactly `length` of them; a
        // null allocator means the default one. A bundle is a directory, hence the flag.
        let url = unsafe {
            CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), length, 1)
        };
        if url.is_null() {
            return Err("the application path could not be read".to_owned());
        }
        // SAFETY: `url` is a live object this call owns and releases exactly once. A null
        // launched-application pointer means it is not wanted.
        Ok(unsafe {
            let status = LSOpenCFURLRef(url, std::ptr::null_mut());
            CFRelease(url);
            status
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use std::time::Duration;

    use super::QuitOutcome;

    /// Unsupported platform: `StillRunning` stops the switch rather than replacing credentials
    /// under a client whose state is unknown.
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

    #[test]
    fn a_desktop_application_is_closed_and_reopened_like_the_managed_runtime() {
        let clients = vec![client(
            10,
            ClientKind::DesktopApp,
            "/Applications/ChatGPT.app/Contents/Resources/codex",
        )];

        let plan = plan(&clients);

        let RestartPlan::CloseThenReopen(targets) = plan else {
            panic!("a desktop application is a restart candidate");
        };
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].pid, 10);
    }

    #[test]
    fn a_terminal_session_is_never_planned_for_a_restart() {
        // A terminal session cannot be put away and brought back, so the pre-check blocks instead.
        let clients = vec![
            client(11, ClientKind::Cli, "/opt/homebrew/bin/codex"),
            client(
                12,
                ClientKind::IdeExtension,
                "/x/extensions/openai.chatgpt/bin/codex",
            ),
        ];

        assert_eq!(plan(&clients), RestartPlan::NothingRunning);
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
            // A signal would end a session's turn half way through.
            "SIGTERM",
            ".kill(",
        ] {
            assert!(
                !code.contains(forbidden),
                "`{forbidden}` would make force-killing reachable"
            );
        }
    }
}
