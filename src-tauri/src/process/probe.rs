//! Which Codex clients are running, and nothing more than that.
//!
//! The line is drawn here: the operating system can say a process exists and which
//! executable it came from. It cannot say whether a session is mid-turn. Every type here is
//! shaped so the second question has no field to be answered in - a test scans this file to
//! keep it that way.
//!
//! **Decided here: the platform API, not `sysinfo`.** Toolhelp plus
//! `QueryFullProcessImageNameW` returns the process id, the parent id and the executable path,
//! which is the whole of what the classification needs, and `windows-sys` is already in the
//! graph for the DACL work - so this is one feature flag rather than a new dependency. The same
//! reasoning applies to the credential store.

use std::path::{Path, PathBuf};

/// A running Codex client, as far as the operating system can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningClient {
    pub pid: u32,
    pub kind: ClientKind,
    /// The executable this process was started from.
    ///
    /// Kept because there is exactly one permitted source for a restart path: the running
    /// process itself, read before it is asked to quit. An absolute path, so it stays inside
    /// the Rust layer and never reaches a log or the frontend.
    pub executable: PathBuf,
}

/// Which installation a running `codex` came from.
///
/// The three trees were confirmed on a real machine, each with its own version, which is what
/// makes them separable at all: `codex-cli 0.98.0` from npm, `0.150.0-alpha.8` bundled inside
/// the editor extension, `0.149.0-alpha.4.3` in the shared managed runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    /// The npm-vendored CLI, under `node_modules/@openai/codex/vendor/...`.
    Cli,
    /// An editor extension's own bundled runtime, under
    /// `.../extensions/openai.chatgpt-<version>/bin/...`.
    ///
    /// Extensions were assumed to share the managed runtime below. They do not - the
    /// extension ships its own copy, which is why an editor session is separable from the
    /// desktop app after all.
    IdeExtension,
    /// The shared managed runtime under `.../OpenAI/Codex/bin/<content hash>/`, which the
    /// desktop app runs from.
    ///
    /// **Whether anything other than the desktop app also drives this tree is not established.**
    /// The desktop app is not installed on the machine these rules were measured on.
    /// The pre-check treats it as the desktop case, which is the safe reading: the user is
    /// asked to close Codex either way.
    ManagedRuntime,
    /// A `codex` executable somewhere Toglet has never seen. Reported rather than ignored: an
    /// unrecognised installation is still a running client.
    Unrecognised,
}

/// The result of a probe.
///
/// [`Self::Unknown`] is a separate state on purpose. Folding "the probe could not run" into an
/// empty list would let a switch proceed while a session was open, which is the one mistake
/// this check exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPresence {
    /// The probe ran. An empty list means nothing was running.
    Known(Vec<RunningClient>),
    /// The probe could not run. Callers must read this as "there may be clients".
    Unknown,
}

/// Finding running Codex clients. Implemented per platform; faked in tests.
pub trait ClientProbe {
    /// Lists running clients, ignoring the process ids in `exclude`.
    ///
    /// `exclude` is how Toglet's own `codex app-server` children stay out of the answer. A
    /// quota refresh that blocked a switch would be Toglet getting in its own way.
    fn running_clients(&self, exclude: &[u32]) -> ClientPresence;
}

/// The executable file name Toglet looks for, without its extension.
const CODEX_STEM: &str = "codex";

/// Decides which installation an executable path belongs to.
///
/// Every rule was measured on a real machine rather than inferred from documentation.
/// Comparison is case-insensitive because Windows paths are, and separators are normalised so
/// one set of rules serves both platforms.
///
/// The extension rule is checked first: an extension directory can sit anywhere, including
/// inside a path that also matches something broader.
pub fn classify(executable: &Path) -> ClientKind {
    let path = executable
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();

    if path.contains("/extensions/openai.chatgpt") {
        return ClientKind::IdeExtension;
    }
    if path.contains("/node_modules/@openai/codex/") {
        return ClientKind::Cli;
    }
    if path.contains("/openai/codex/bin/") {
        return ClientKind::ManagedRuntime;
    }
    ClientKind::Unrecognised
}

/// Whether an executable file name is one Toglet is looking for.
///
/// Matched on the stem rather than the whole name so `codex.exe` and a future extensionless
/// build both count, and so the sibling helpers the managed runtime ships -
/// `codex-command-runner.exe` among them - do not.
pub fn is_codex_executable(file_name: &str) -> bool {
    Path::new(file_name)
        .file_stem()
        .is_some_and(|stem| stem.to_string_lossy().to_lowercase() == CODEX_STEM)
}

/// The probe backed by the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClientProbe;

impl SystemClientProbe {
    pub fn new() -> Self {
        Self
    }
}

impl ClientProbe for SystemClientProbe {
    fn running_clients(&self, exclude: &[u32]) -> ClientPresence {
        platform::running_clients(exclude)
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };

    use super::{ClientPresence, RunningClient, classify, is_codex_executable};

    /// Enough for any Windows path, including the extended-length form.
    const PATH_BUFFER: usize = 32_768;

    /// One process, as the snapshot describes it.
    struct Entry {
        pid: u32,
        name: String,
    }

    pub fn running_clients(exclude: &[u32]) -> ClientPresence {
        let Some(entries) = snapshot() else {
            // The snapshot itself failed. Saying "nothing is running" here would be a guess
            // with a switch riding on it.
            return ClientPresence::Unknown;
        };

        let clients = entries
            .iter()
            .filter(|entry| !exclude.contains(&entry.pid))
            .filter(|entry| is_codex_executable(&entry.name))
            .filter_map(|entry| {
                // A process that cannot be opened is skipped rather than guessed at: without
                // its path there is nothing to classify. This needs no elevation, so in
                // practice it happens only when the process just exited.
                let executable = executable_of(entry.pid)?;
                Some(RunningClient {
                    pid: entry.pid,
                    kind: classify(&executable),
                    executable,
                })
            })
            .collect();

        ClientPresence::Known(clients)
    }

    fn snapshot() -> Option<Vec<Entry>> {
        // SAFETY: the handle is checked against INVALID_HANDLE_VALUE before use and closed on
        // every path below, including the early return when the first entry cannot be read.
        let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let snapshot = OwnedHandle(handle);

        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..unsafe { std::mem::zeroed() }
        };

        // SAFETY: `entry.dwSize` is set as the API requires, and the handle is live.
        if unsafe { Process32FirstW(snapshot.0, &mut entry) } == 0 {
            return None;
        }

        let mut entries = Vec::new();
        loop {
            entries.push(Entry {
                pid: entry.th32ProcessID,
                name: wide_to_string(&entry.szExeFile),
            });
            // SAFETY: same handle, and `entry` is fully initialised by the previous call.
            if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
                break;
            }
        }
        Some(entries)
    }

    fn executable_of(pid: u32) -> Option<PathBuf> {
        // `PROCESS_QUERY_LIMITED_INFORMATION` is the least privilege that answers this, and is
        // granted for the current user's own processes without elevation.
        // SAFETY: the returned handle is validated and closed by `OwnedHandle`.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let process = OwnedHandle(handle);

        let mut buffer = vec![0u16; PATH_BUFFER];
        let mut length = buffer.len() as u32;
        // SAFETY: `buffer` holds `length` writable u16s, and the call writes at most that many.
        let ok =
            unsafe { QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut length) };
        if ok == 0 {
            return None;
        }
        buffer.truncate(length as usize);
        Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer)))
    }

    fn wide_to_string(wide: &[u16]) -> String {
        let end = wide
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }

    /// Closes its handle exactly once, including when an early return skips the close.
    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: the handle came from a call that returned success and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ClientPresence;

    /// Not implemented outside Windows.
    ///
    /// **Unverified platform.** Returning an empty list would be a lie that lets a
    /// switch run while an editor session is open; `Unknown` is the truthful and conservative
    /// answer until this can be written and measured on a real machine.
    pub fn running_clients(_exclude: &[u32]) -> ClientPresence {
        ClientPresence::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_cli_is_recognised_by_its_tree() {
        let path = Path::new(
            r"D:\sdk\Node-js\node_modules\@openai\codex\vendor\x86_64-pc-windows-msvc\codex\codex.exe",
        );

        assert_eq!(classify(path), ClientKind::Cli);
    }

    #[test]
    fn the_shared_managed_runtime_is_recognised() {
        let path = Path::new(r"C:\Users\x\AppData\Local\OpenAI\Codex\bin\8fffe694\codex.exe");

        assert_eq!(classify(path), ClientKind::ManagedRuntime);
    }

    #[test]
    fn an_editor_extensions_own_runtime_is_not_mistaken_for_the_shared_one() {
        // Measured: the extension ships its own build, at a different version from the shared
        // runtime. Without this rule an editor session would be reported as the desktop app and
        // get "close it and reopen" instead of blocking the switch.
        let path = Path::new(
            r"C:\Users\x\.vscode\extensions\openai.chatgpt-26.820.60940-win32-x64\bin\windows-x86_64\codex.exe",
        );

        assert_eq!(classify(path), ClientKind::IdeExtension);
    }

    #[test]
    fn an_installation_toglet_has_never_seen_is_still_reported() {
        let path = Path::new("/opt/somewhere/codex");

        assert_eq!(classify(path), ClientKind::Unrecognised);
    }

    #[test]
    fn classification_does_not_depend_on_the_case_of_a_windows_path() {
        let path = Path::new(r"C:\USERS\X\APPDATA\LOCAL\OPENAI\CODEX\BIN\8FFFE694\CODEX.EXE");

        assert_eq!(classify(path), ClientKind::ManagedRuntime);
    }

    #[test]
    fn the_runtimes_helper_executables_are_not_mistaken_for_a_client() {
        // These ship next to `codex.exe` in the managed runtime. Matching on a prefix would
        // report each of them as a running client.
        for name in [
            "codex-command-runner.exe",
            "codex-code-mode-host.exe",
            "codex-windows-sandbox-setup.exe",
        ] {
            assert!(!is_codex_executable(name), "{name} is not a Codex client");
        }
    }

    #[test]
    fn the_client_executable_is_recognised_with_and_without_an_extension() {
        assert!(is_codex_executable("codex.exe"));
        assert!(is_codex_executable("codex"));
        assert!(is_codex_executable("CODEX.EXE"));
    }

    /// The probe must not offer an answer to "is it busy?".
    ///
    /// Scanning the source rather than reviewing it, because the requirement is about what
    /// future edits may add, not only about what is here today. The same guard protects
    /// `quota::scheduler` from growing a write path.
    #[test]
    fn nothing_here_claims_to_know_whether_a_session_is_busy() {
        let source = include_str!("probe.rs");
        // Up to the test module only: this test names the forbidden words, so scanning itself
        // would always fail.
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part");
        let declarations = implementation
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();

        for forbidden in ["is_idle", "idle", "busy", "generating", "in_progress"] {
            assert!(
                !declarations.contains(forbidden),
                "`{forbidden}` suggests this module answers more than it is allowed to"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn probing_this_machine_answers_rather_than_failing() {
        // The probe must work as an ordinary user, with no elevation and no accessibility
        // permission. Whether any Codex client happens to be running is not the
        // point - that the enumeration succeeds is.
        let presence = SystemClientProbe::new().running_clients(&[]);

        assert!(
            matches!(presence, ClientPresence::Known(_)),
            "enumerating processes must not need a privilege the app does not have"
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_excluded_process_is_left_out() {
        // Toglet's own app servers must not block Toglet's own switch. Excluding the current
        // process proves the filter is applied even though this process is not a Codex client.
        let all = SystemClientProbe::new().running_clients(&[]);
        let without_self = SystemClientProbe::new().running_clients(&[std::process::id()]);

        let (ClientPresence::Known(all), ClientPresence::Known(without_self)) = (all, without_self)
        else {
            panic!("the probe must answer on this platform");
        };
        assert!(!without_self.iter().any(|c| c.pid == std::process::id()));
        assert!(!all.iter().any(|c| c.pid == std::process::id()));
    }
}
