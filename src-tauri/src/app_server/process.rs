//! Locates the Codex executable and runs one `codex app-server` subprocess.
//!
//! The command line is a compile-time constant with nothing user-editable in its arguments or
//! environment, and there is no shell: on Windows the `codex` on `PATH` is an npm shim, so the
//! native executable is located directly.

use std::io;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// The only argument Toglet ever passes to the Codex executable.
const APP_SERVER_ARG: &str = "app-server";

/// How long a closed stdin is given to end the process before it is terminated; a clean exit
/// takes about 10 ms.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

const EXIT_POLL: Duration = Duration::from_millis(10);

/// Keeps Windows from opening a console window for the child.
///
/// Toglet has no console of its own, and the Codex executable is a console program, so Windows
/// would create one window per subprocess - several per quota refresh. The flag has no effect on
/// the pipes this process talks over.
#[cfg(windows)]
fn without_console(command: &mut Command) {
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn without_console(_command: &mut Command) {}

/// A verified path to the native Codex executable, so an unchecked path cannot reach
/// [`AppServerProcess::spawn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexBinary {
    path: PathBuf,
}

impl CodexBinary {
    /// Finds the native executable on `PATH`, then in well-known macOS install directories.
    ///
    /// Each entry is checked for a native executable and for the npm vendored layout; a directory
    /// holding only a shim (`codex.cmd`, a `#!` script) is skipped so a wrapper is never started.
    pub fn resolve(phase: Phase) -> Result<Self> {
        let path: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        if let Some(found) = resolve_in(&path) {
            return Ok(found);
        }
        if std::env::consts::OS == "macos" {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            if let Some(found) = newest_in(&well_known_directories(home.as_deref())) {
                return Ok(found);
            }
        }
        Err(not_installed(phase))
    }

    /// Uses an explicitly configured path from settings, validated the same way, for install
    /// layouts automatic resolution cannot find.
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

/// macOS install directories `PATH` may not name.
///
/// An app opened from the Finder or Dock gets launchd's `PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`),
/// which misses Homebrew, npm and nvm installs. A Windows app inherits the user's `PATH`.
fn well_known_directories(home: Option<&Path>) -> Vec<PathBuf> {
    let mut directories = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = home {
        directories.push(home.join(".local").join("bin"));
        directories.extend(nvm_bins(&home.join(".nvm").join("versions").join("node")));
    }
    directories
}

/// The `bin` directory of each node version under nvm, highest version first.
fn nvm_bins(versions: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(versions) else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    nodes.sort_by_key(|path| std::cmp::Reverse(version_key(path)));
    nodes.into_iter().map(|node| node.join("bin")).collect()
}

/// `v24.18.0` → `(24, 18, 0)`; anything else sorts last.
fn version_key(path: &Path) -> (u64, u64, u64) {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let mut parts = name
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// Among the installs found under `directories`, the one written most recently.
///
/// Unlike `PATH`, these directories express no preference, and old copies linger under other
/// node managers; the install touched last is the one the user means.
fn newest_in(directories: &[PathBuf]) -> Option<CodexBinary> {
    directories
        .iter()
        .filter_map(|directory| native_in(directory).or_else(|| vendored_under(directory)))
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|meta| meta.modified())
                .ok()
        })
        .map(|path| CodexBinary { path })
}

fn resolve_in(directories: &[PathBuf]) -> Option<CodexBinary> {
    directories.iter().find_map(|directory| {
        native_in(directory)
            .or_else(|| vendored_under(directory))
            .map(|path| CodexBinary { path })
    })
}

fn native_in(directory: &Path) -> Option<PathBuf> {
    let candidate = directory.join(executable_name("codex"));
    (is_executable_file(&candidate) && !is_script(&candidate)).then_some(candidate)
}

/// The executable the npm package vendors for this platform.
///
/// Both the current platform package (`@openai/codex-darwin-arm64`) and the older layout vendored
/// inside `@openai/codex` are tried. npm keeps packages beside the executables on Windows and in
/// `../lib` elsewhere, and only the executables directory is on `PATH`.
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

/// Every place the executable may sit under an `@openai` directory.
///
/// npm may hoist the platform package beside `@openai/codex` or nest it inside, and the
/// executable may be in `vendor/<triple>/codex/` (where `bin/codex.js` expects it) or
/// `vendor/<triple>/bin/`.
fn candidates_under(scope: &Path) -> Vec<PathBuf> {
    let (Some(triple), Some(package)) = (target_triple(), platform_package()) else {
        return Vec::new();
    };
    let executable = executable_name("codex");
    let platform = |root: &Path| {
        let vendor = root
            .join(format!("codex-{package}"))
            .join("vendor")
            .join(triple);
        [
            vendor.join("codex").join(&executable),
            vendor.join("bin").join(&executable),
        ]
    };
    let nested = scope.join("codex").join("node_modules").join("@openai");

    let mut candidates = Vec::with_capacity(5);
    candidates.extend(platform(scope));
    candidates.extend(platform(&nested));
    candidates.push(
        scope
            .join("codex")
            .join("vendor")
            .join(triple)
            .join("codex")
            .join(&executable),
    );
    candidates
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
/// On Unix the npm shim is a script with the real executable's name. Unreadable files count as
/// scripts, so nothing that could not be inspected is started.
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
pub(crate) struct AppServerProcess {
    child: Child,
    /// Taken on shutdown: closing stdin is what asks the app server to exit.
    stdin: Option<ChildStdin>,
    phase: Phase,
    reaped: bool,
}

impl AppServerProcess {
    /// Starts the app server against `home`, a directory Toglet generated, returning the process
    /// and its output stream.
    pub(crate) fn spawn(
        binary: &CodexBinary,
        home: &Path,
        phase: Phase,
    ) -> Result<(Self, ChildStdout)> {
        let mut command = Command::new(binary.path());
        command
            .arg(APP_SERVER_ARG)
            .env("CODEX_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Discarded rather than captured: the app server's diagnostics may contain paths and
            // credential material.
            .stderr(Stdio::null());
        without_console(&mut command);
        let mut child = command
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
            // A missing piped handle leaves the process unusable; reap it rather than leak it.
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

    /// The subprocess id, so a client probe can be told to leave Toglet's own server alone.
    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(crate) fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.stdin.as_mut()
    }

    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    /// Closes stdin, waits for the process to exit and reports how it went. Idempotent.
    pub(crate) fn finish(&mut self) -> Result<()> {
        if self.reaped {
            return Ok(());
        }
        self.reaped = true;

        // Closing stdin is the documented shutdown and normally exits with code 0 within
        // milliseconds.
        drop(self.stdin.take());

        match self.wait_for_exit(SHUTDOWN_GRACE)? {
            Some(status) if status.success() => Ok(()),
            Some(status) => Err(crashed(self.phase, "the app server exited abnormally")
                .with_detail(&format!("exit status: {status}"))),
            None => {
                // Only ever this process, only after asking it to leave; never an external Codex
                // client.
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
            // A guard cannot return the error, and an unreported failure would hide a subprocess
            // leak.
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
        /// As `Nested`, with the executable under `vendor/<triple>/codex/` rather than `bin/`.
        NestedCodexDirectory,
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
            Layout::NestedCodexDirectory => bin
                .join("node_modules")
                .join("@openai")
                .join("codex")
                .join("node_modules")
                .join("@openai")
                .join(format!("codex-{package}"))
                .join("vendor")
                .join(triple)
                .join("codex"),
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
        // A directory holding nothing but a wrapper `.cmd`.
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
        // Older installs vendor the binary inside `@openai/codex` and must not read as not
        // installed.
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let npm = npm_layout(scratch.path(), "npm", Layout::Bundled);

        let resolved = std::slice::from_ref(&npm);
        let resolved = resolve_in(resolved).expect("a binary is resolved");

        assert!(resolved.path().starts_with(npm.join("node_modules")));
    }

    #[test]
    fn a_platform_package_left_nested_resolves() {
        // npm hoists the platform package beside `@openai/codex` in some installs and nests it in
        // others.
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let npm = npm_layout(scratch.path(), "npm", Layout::Nested);

        let resolved = std::slice::from_ref(&npm);
        let resolved = resolve_in(resolved).expect("a binary is resolved");

        assert!(resolved.path().starts_with(npm.join("node_modules")));
    }

    #[test]
    fn a_platform_package_keeping_the_executable_in_a_codex_directory_resolves() {
        // The launcher `@openai/codex/bin/codex.js` runs `vendor/<triple>/codex/codex`.
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let npm = npm_layout(scratch.path(), "npm", Layout::NestedCodexDirectory);

        let resolved = std::slice::from_ref(&npm);
        let resolved = resolve_in(resolved).expect("a binary is resolved");

        assert!(resolved.path().starts_with(npm.join("node_modules")));
        assert!(
            resolved
                .path()
                .parent()
                .is_some_and(|dir| dir.ends_with("codex"))
        );
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
    fn outside_path_the_most_recently_written_install_wins() {
        let scratch = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        let older = npm_layout(scratch.path(), "older", Layout::Platform);
        let newer = npm_layout(scratch.path(), "newer", Layout::Platform);
        let executable = |directory: &PathBuf| {
            resolve_in(std::slice::from_ref(directory))
                .expect("the layout resolves")
                .path()
                .to_path_buf()
        };
        let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000);
        std::fs::File::options()
            .write(true)
            .open(executable(&older))
            .and_then(|file| file.set_modified(long_ago))
            .expect("the older install is dated");

        let found = newest_in(&[older.clone(), newer.clone()]).expect("an install is found");

        assert_eq!(found.path(), executable(&newer).as_path());
        assert_eq!(newest_in(&[scratch.path().join("absent")]), None);
    }

    // An app opened from the Finder gets launchd's PATH, which names none of these.
    #[test]
    fn the_well_known_directories_cover_homebrew_local_and_every_nvm_node_newest_first() {
        let home = IsolatedHome::create(Phase::Detect).expect("scratch directory is created");
        for version in ["v18.20.4", "v24.18.0", "v9.0.0", "notes"] {
            std::fs::create_dir_all(home.path().join(".nvm/versions/node").join(version))
                .expect("nvm layout is created");
        }
        std::fs::write(home.path().join(".nvm/versions/node/README"), "").expect("a file");

        let directories = well_known_directories(Some(home.path()));

        assert_eq!(
            directories,
            vec![
                PathBuf::from("/opt/homebrew/bin"),
                PathBuf::from("/usr/local/bin"),
                home.path().join(".local/bin"),
                home.path().join(".nvm/versions/node/v24.18.0/bin"),
                home.path().join(".nvm/versions/node/v18.20.4/bin"),
                home.path().join(".nvm/versions/node/v9.0.0/bin"),
                home.path().join(".nvm/versions/node/notes/bin"),
            ]
        );
        assert_eq!(well_known_directories(None).len(), 2);
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

    /// Windows must not open a console window for the child.
    ///
    /// Scanned rather than observed: the flag cannot be read back from a spawned process, and
    /// what has to hold is that future edits keep applying it. Toglet has no console, so without
    /// it every quota read flashes a window at the user.
    #[test]
    fn the_app_server_is_started_without_a_console_window() {
        let source = include_str!("process.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            implementation.contains("CREATE_NO_WINDOW"),
            "the Windows branch must pass CREATE_NO_WINDOW"
        );
        assert_eq!(
            implementation
                .matches("without_console(&mut command)")
                .count(),
            1,
            "every spawn goes through the one helper that applies the flag"
        );
        assert_eq!(
            implementation.matches("Command::new").count(),
            1,
            "a second spawn site would need the flag of its own"
        );
    }
}
