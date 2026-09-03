//! Locating the Codex executable and running one `codex app-server` subprocess.
//!
//! Two hard constraints shape this module:
//!
//! * The command line is fixed at compile time. [`APP_SERVER_ARG`] is the only argument and
//!   nothing user-editable ever reaches the argument list or the environment.
//! * No shell. On Windows the `codex` name on `PATH` is an npm shim, and resolving it would
//!   need `cmd.exe`; the native executable is located directly instead.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// The only argument Toglet ever passes to the Codex executable.
const APP_SERVER_ARG: &str = "app-server";

/// How long a closed stdin is given to end the process before it is terminated. A clean exit
/// was measured at 8-9 ms, so this is three orders of magnitude of headroom rather than a
/// guess at typical timing.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Poll interval while waiting for the subprocess to exit.
const EXIT_POLL: Duration = Duration::from_millis(10);

/// A verified path to the native Codex executable.
///
/// Held as a type rather than a bare `PathBuf` so a path that was never checked cannot reach
/// [`AppServerProcess::spawn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexBinary {
    path: PathBuf,
}

impl CodexBinary {
    /// Finds the native executable by walking `PATH`.
    ///
    /// Each entry is checked twice: for a native executable sitting directly in it, and for
    /// the npm layout `node_modules/@openai/codex/vendor/<triple>/codex/`. A directory holding
    /// only a shim (`codex.cmd`, a `#!` script) matches neither and is skipped, which is what
    /// keeps a third-party wrapper on `PATH` from being started instead of Codex itself.
    pub fn resolve(phase: Phase) -> Result<Self> {
        let path = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .unwrap_or_default();
        resolve_in(&path).ok_or_else(|| not_installed(phase))
    }

    /// Uses an explicitly configured path.
    ///
    /// Automatic resolution cannot cover every install layout (Homebrew, standalone archives,
    /// a managed runtime), so a configured path is the escape hatch. It is validated the same
    /// way, and it comes from settings - never from anything typed on a command line.
    pub fn at(path: PathBuf, phase: Phase) -> Result<Self> {
        if is_executable_file(&path) {
            Ok(Self { path })
        } else {
            Err(not_installed(phase).with_detail("the configured Codex path is not a file"))
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn resolve_in(directories: &[PathBuf]) -> Option<CodexBinary> {
    directories.iter().find_map(|directory| {
        native_in(directory)
            .or_else(|| vendored_under(directory))
            .map(|path| CodexBinary { path })
    })
}

/// A Codex executable sitting directly in `directory`.
fn native_in(directory: &Path) -> Option<PathBuf> {
    let candidate = directory.join(executable_name("codex"));
    (is_executable_file(&candidate) && !is_script(&candidate)).then_some(candidate)
}

/// The executable the npm package vendors for this platform.
///
/// Two package shapes are recognised. Current releases ship the binary in a platform-specific
/// package (`@openai/codex-darwin-arm64`); earlier ones vendored it inside `@openai/codex`.
/// An installed Codex is not necessarily a freshly installed one, so both are tried.
///
/// Two module roots are searched for the same reason: npm keeps its packages beside the
/// executables on Windows and one level up in `lib` everywhere else, and only the directory
/// holding the executables is ever on `PATH`.
fn vendored_under(directory: &Path) -> Option<PathBuf> {
    let mut bases = vec![directory.join("node_modules")];
    if let Some(parent) = directory.parent() {
        bases.push(parent.join("lib").join("node_modules"));
    }
    bases
        .iter()
        .flat_map(|base| candidates_under(&base.join("@openai")))
        .find(|candidate| is_executable_file(candidate))
}

/// Every place the executable has been found under an `@openai` directory.
///
/// The platform package is reached two ways because npm may hoist it beside `@openai/codex` or
/// leave it nested inside it, and which one happens depends on the install.
fn candidates_under(scope: &Path) -> Vec<PathBuf> {
    let (Some(triple), Some(package)) = (target_triple(), platform_package()) else {
        return Vec::new();
    };
    let executable = executable_name("codex");
    let platform = |root: &Path| {
        root.join(format!("codex-{package}"))
            .join("vendor")
            .join(triple)
            .join("bin")
            .join(&executable)
    };

    vec![
        platform(scope),
        platform(&scope.join("codex").join("node_modules").join("@openai")),
        scope
            .join("codex")
            .join("vendor")
            .join(triple)
            .join("codex")
            .join(&executable),
    ]
}

/// The platform-specific package name current npm releases install the binary into. `None` on
/// platforms Toglet does not target.
fn platform_package() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("win32-x64"),
        ("windows", "aarch64") => Some("win32-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("macos", "aarch64") => Some("darwin-arm64"),
        _ => None,
    }
}

/// The vendor directory name for the running platform. `None` on platforms Toglet does not
/// target, where only a native executable on `PATH` can be used.
fn target_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Some("aarch64-pc-windows-msvc"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

fn executable_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Whether the file starts with a `#!` line.
///
/// On Unix the npm shim is a shell script with the same name as the real executable, and
/// running it would need an interpreter. Unreadable files are treated as scripts: refusing to
/// start something Toglet could not inspect is the safe direction.
fn is_script(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return true;
    };
    let mut magic = [0u8; 2];
    match file.read_exact(&mut magic) {
        Ok(()) => magic == *b"#!",
        Err(_) => true,
    }
}

/// One running `codex app-server`.
///
/// Shutting down is idempotent, so the explicit path and the `Drop` guard can both call it.
pub(crate) struct AppServerProcess {
    child: Child,
    /// Taken on shutdown: closing stdin is what asks the app server to exit.
    stdin: Option<ChildStdin>,
    phase: Phase,
    reaped: bool,
}

impl AppServerProcess {
    /// Starts the app server against `home`, returning the process and its output stream.
    ///
    /// `home` is a directory Toglet generated; no user-editable value is passed here.
    pub(crate) fn spawn(
        binary: &CodexBinary,
        home: &Path,
        phase: Phase,
    ) -> Result<(Self, ChildStdout)> {
        let mut child = Command::new(binary.path())
            .arg(APP_SERVER_ARG)
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Discarded rather than captured: the app server's diagnostics may contain paths
            // and credential material, and there is no redacting sink to send them to yet.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| not_installed(phase).with_detail(&error.to_string()))?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let mut process = Self {
            child,
            stdin,
            phase,
            reaped: false,
        };

        match (process.stdin.is_some(), stdout) {
            (true, Some(stdout)) => Ok((process, stdout)),
            // A piped handle is missing, which leaves the process unusable. Reap it here
            // instead of leaking it back to the caller.
            (_, stdout) => {
                drop(stdout);
                let shutdown = process.finish();
                Err(
                    crashed(phase, "the app server did not expose its standard streams")
                        .with_detail(&format!("shutdown: {shutdown:?}")),
                )
            }
        }
    }

    pub(crate) fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }

    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    /// Closes stdin, waits for the process to exit and reports how it went.
    ///
    /// Calling this more than once is a no-op, so the explicit path and the guard agree.
    pub(crate) fn finish(&mut self) -> Result<()> {
        if self.reaped {
            return Ok(());
        }
        self.reaped = true;

        // The documented shutdown: exit code 0 was measured within 8-9 ms of stdin closing,
        // so the normal path never terminates anything.
        drop(self.stdin.take());

        match self.wait_for_exit(SHUTDOWN_GRACE)? {
            Some(status) if status.success() => Ok(()),
            Some(status) => Err(crashed(self.phase, "the app server exited abnormally")
                .with_detail(&format!("exit status: {status}"))),
            None => {
                // Only ever this process, only after asking it to leave, and never an external
                // Codex client.
                let terminated = self.terminate();
                Err(
                    crashed(self.phase, "the app server did not exit after stdin closed")
                        .with_detail(&format!("termination: {terminated:?}")),
                )
            }
        }
    }

    fn wait_for_exit(&mut self, grace: Duration) -> Result<Option<ExitStatus>> {
        let deadline = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Ok(Some(status)),
                Ok(None) if Instant::now() >= deadline => return Ok(None),
                Ok(None) => std::thread::sleep(EXIT_POLL),
                Err(error) => {
                    return Err(crashed(self.phase, "could not wait for the app server")
                        .with_detail(&error.to_string()));
                }
            }
        }
    }

    fn terminate(&mut self) -> io::Result<()> {
        self.child.kill()?;
        self.child.wait().map(|_| ())
    }
}

impl Drop for AppServerProcess {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            // A guard cannot return this, and leaving a subprocess unreported would hide
            // exactly the subprocess leak this reporting exists to prevent.
            crate::diagnostics::record_background_failure(error);
        }
    }
}

fn not_installed(phase: Phase) -> TogletError {
    TogletError::new(
        ErrorCode::RuntimeNotInstalled,
        phase,
        false,
        UserAction::InstallRuntime,
    )
}

fn crashed(phase: Phase, detail: &str) -> TogletError {
    TogletError::new(ErrorCode::AppServerCrashed, phase, true, UserAction::Retry)
        .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::IsolatedHome;

    /// `IsolatedHome` is used as the scratch directory throughout these tests: it is exactly a
    /// private temporary directory that removes itself even if a test panics, and reusing it
    /// beats a second temporary-directory helper.
    ///
    /// Where an npm install keeps the executable.
    enum Layout {
        /// Current releases: a package of its own, beside the directory that is on `PATH`.
        Platform,
        /// Earlier releases: vendored inside `@openai/codex`.
        Bundled,
        /// The Unix npm prefix: `prefix/bin` is on `PATH`, packages live in `prefix/lib`.
        BesideBin,
        /// The platform package left inside `@openai/codex` rather than hoisted beside it.
        Nested,
    }

    /// Builds a directory tree that mirrors a real npm install and returns its `PATH` entry.
    fn npm_layout(root: &Path, name: &str, layout: Layout) -> PathBuf {
        let bin = root.join(name);
        let triple = target_triple().expect("Toglet only targets Windows and macOS");
        let package = platform_package().expect("Toglet only targets Windows and macOS");
        let vendor = match layout {
            Layout::Platform => bin
                .join("node_modules")
                .join("@openai")
                .join(format!("codex-{package}"))
                .join("vendor")
                .join(triple)
                .join("bin"),
            Layout::Bundled => bin
                .join("node_modules")
                .join("@openai")
                .join("codex")
                .join("vendor")
                .join(triple)
                .join("codex"),
            Layout::BesideBin => root
                .join("lib")
                .join("node_modules")
                .join("@openai")
                .join(format!("codex-{package}"))
                .join("vendor")
                .join(triple)
                .join("bin"),
            Layout::Nested => bin
                .join("node_modules")
                .join("@openai")
                .join("codex")
                .join("node_modules")
                .join("@openai")
                .join(format!("codex-{package}"))
                .join("vendor")
                .join(triple)
                .join("bin"),
        };
        std::fs::create_dir_all(&vendor).expect("vendor directory is created");
        std::fs::create_dir_all(&bin).expect("bin directory is created");
        // The shim, which must not be selected.
        std::fs::write(bin.join("codex.cmd"), b"@echo off\n").expect("shim is written");
        std::fs::write(vendor.join(executable_name("codex")), b"MZ")
            .expect("vendored executable is written");
        bin
    }

    #[test]
    fn resolution_prefers_the_vendored_executable_over_a_shim_directory() {
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let empty = scratch.path().join("empty");
        std::fs::create_dir_all(&empty).expect("empty directory is created");
        // A directory holding nothing but a wrapper `.cmd`, like the third-party wrapper found
        // on the development machine.
        let wrapper = scratch.path().join("wrapper");
        std::fs::create_dir_all(&wrapper).expect("wrapper directory is created");
        std::fs::write(wrapper.join("codex.cmd"), b"@echo off\n").expect("wrapper is written");
        let npm = npm_layout(scratch.path(), "npm", Layout::Platform);

        let resolved = resolve_in(&[empty, wrapper, npm.clone()]).expect("a binary is resolved");

        assert!(resolved.path().starts_with(npm.join("node_modules")));
        assert_eq!(
            resolved
                .path()
                .file_name()
                .and_then(std::ffi::OsStr::to_str),
            Some(executable_name("codex").as_str())
        );
    }

    #[test]
    fn an_install_that_still_bundles_the_executable_resolves() {
        // Codex moved the binary into a package of its own. A machine that has not updated
        // keeps the older tree, and reporting "not installed" there would be false.
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let npm = npm_layout(scratch.path(), "npm", Layout::Bundled);

        let resolved = std::slice::from_ref(&npm);
        let resolved = resolve_in(resolved).expect("a binary is resolved");

        assert!(resolved.path().starts_with(npm.join("node_modules")));
    }

    #[test]
    fn a_platform_package_left_nested_resolves() {
        // npm hoists the platform package beside `@openai/codex` in some installs and leaves it
        // inside in others. Both are a working install, so both have to resolve.
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let npm = npm_layout(scratch.path(), "npm", Layout::Nested);

        let resolved = std::slice::from_ref(&npm);
        let resolved = resolve_in(resolved).expect("a binary is resolved");

        assert!(resolved.path().starts_with(npm.join("node_modules")));
    }

    #[test]
    fn packages_kept_beside_the_bin_directory_resolve() {
        // Only `prefix/bin` is ever on `PATH`, and everywhere but Windows npm puts the
        // packages in `prefix/lib` instead of inside it.
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let prefix = scratch.path().join("prefix");
        let bin = npm_layout(&prefix, "bin", Layout::BesideBin);

        let resolved = resolve_in(&[bin]).expect("a binary is resolved");

        assert!(resolved.path().starts_with(prefix.join("lib")));
    }

    #[test]
    fn resolution_reports_runtime_not_installed_when_nothing_matches() {
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");

        assert!(resolve_in(&[scratch.path().to_path_buf()]).is_none());
    }

    #[test]
    fn a_configured_path_must_point_at_a_file() {
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");

        let error = CodexBinary::at(scratch.path().join("absent"), Phase::Detect)
            .expect_err("a missing path is rejected");

        assert_eq!(error.code(), ErrorCode::RuntimeNotInstalled);
        // The directory a user configured must not come back out in the error.
        assert!(!error.detail().unwrap_or_default().contains("absent"));
    }

    #[test]
    fn a_shell_script_is_never_treated_as_the_executable() {
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let shim = scratch.path().join(executable_name("codex"));
        std::fs::write(&shim, b"#!/usr/bin/env node\n").expect("shim is written");

        assert!(is_script(&shim));
        assert!(native_in(scratch.path()).is_none());
    }

    #[test]
    fn the_argument_list_is_a_single_compile_time_constant() {
        assert_eq!(APP_SERVER_ARG, "app-server");
    }
}
