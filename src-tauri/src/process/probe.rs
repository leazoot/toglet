//! Which Codex clients are running, and nothing more.
//!
//! The OS can say a process exists and where its executable is, not whether a session is
//! mid-turn; no type here has a field for the latter, and a test scans this file to keep it so.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningClient {
    pub pid: u32,
    pub kind: ClientKind,
    /// The only permitted source for a restart path. Absolute, so it never leaves the Rust layer.
    pub executable: PathBuf,
}

/// Which installation a running `codex` came from, told apart by its install tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    /// The npm-vendored CLI, under `node_modules/@openai/codex/vendor/...`.
    Cli,
    /// An editor extension's own bundled runtime, under
    /// `.../extensions/openai.chatgpt-<version>/bin/...`, separate from the managed runtime.
    IdeExtension,
    /// The shared managed runtime under `.../OpenAI/Codex/bin/<content hash>/`, used by the
    /// desktop app.
    ///
    /// Whether anything else also runs from this tree is unknown; it is treated as the desktop
    /// case, which is safe because the user is asked to close Codex either way.
    ManagedRuntime,
    /// The `codex` a macOS desktop app ships in its bundle, e.g.
    /// `/Applications/ChatGPT.app/Contents/Resources/codex`; the bundle allows a graceful quit.
    DesktopApp,
    /// A `codex` executable in an unknown location; still a running client.
    Unrecognised,
}

/// The result of a probe.
///
/// [`Self::Unknown`] is kept apart from an empty list so a failed probe never lets a switch
/// proceed while a session is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPresence {
    /// The probe ran. An empty list means nothing was running.
    Known(Vec<RunningClient>),
    /// The probe could not run. Callers must read this as "there may be clients".
    Unknown,
}

pub trait ClientProbe {
    /// Lists running clients except the pids in `exclude`, which keeps Toglet's own app servers
    /// out.
    fn running_clients(&self, exclude: &[u32]) -> ClientPresence;
}

/// The executable file name Toglet looks for, without its extension.
const CODEX_STEM: &str = "codex";

/// Decides which installation an executable path belongs to.
///
/// Case-insensitive with normalised separators so one rule set serves both platforms. The
/// extension rule comes first because an extension directory can sit inside a broader match.
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
    // The macOS desktop app ships `codex` in its bundle (`ChatGPT.app/Contents/Resources/codex`,
    // bundle id `com.openai.codex`); left unrecognised it would block every switch.
    if path.contains(".app/contents/") {
        return ClientKind::DesktopApp;
    }
    ClientKind::Unrecognised
}

/// Matches the file stem, so `codex.exe` and `codex` count but sibling helpers such as
/// `codex-command-runner.exe` do not.
pub fn is_codex_executable(file_name: &str) -> bool {
    Path::new(file_name)
        .file_stem()
        .is_some_and(|stem| stem.to_string_lossy().to_lowercase() == CODEX_STEM)
}

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

/// The executable a live process was started from, or `None` if it has gone or belongs to
/// another user.
#[cfg(target_os = "macos")]
pub(crate) fn executable_of(pid: u32) -> Option<std::path::PathBuf> {
    platform::executable_of(pid)
}

/// Every process this user can see; `None` when the scan failed.
#[cfg(target_os = "macos")]
pub(crate) fn running_pids() -> Option<Vec<u32>> {
    platform::all_pids()
}

/// Parent hops followed before giving up; reused pids can make a chain loop.
const MAX_ANCESTRY: usize = 64;

/// Whether `pid`'s chain of parents reaches `root`.
///
/// Keeps every `codex app-server` Toglet spawned out of the probe without registering them;
/// otherwise a just-finished verification server can read as a CLI session and block a switch.
/// The walk returns `false` as soon as `parent_of` cannot read a process.
pub(crate) fn descends_from(pid: u32, root: u32, parent_of: impl Fn(u32) -> Option<u32>) -> bool {
    let mut current = pid;
    for _ in 0..MAX_ANCESTRY {
        let Some(parent) = parent_of(current) else {
            return false;
        };
        if parent == root {
            return true;
        }
        // The top of the tree, or a process that reports itself as its own parent.
        if parent <= 1 || parent == current {
            return false;
        }
        current = parent;
    }
    false
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

    use super::{ClientPresence, RunningClient, classify, descends_from, is_codex_executable};

    /// Enough for any Windows path, including the extended-length form.
    const PATH_BUFFER: usize = 32_768;

    struct Entry {
        pid: u32,
        parent: u32,
        name: String,
    }

    pub fn running_clients(exclude: &[u32]) -> ClientPresence {
        let Some(entries) = snapshot() else {
            // A failed snapshot is not "nothing running"; a switch depends on the answer.
            return ClientPresence::Unknown;
        };

        // The snapshot carries every parent, so ancestry is a lookup. Not verified on Windows.
        let parents: std::collections::HashMap<u32, u32> = entries
            .iter()
            .map(|entry| (entry.pid, entry.parent))
            .collect();
        let own = std::process::id();

        let clients = entries
            .iter()
            .filter(|entry| !exclude.contains(&entry.pid))
            .filter(|entry| is_codex_executable(&entry.name))
            .filter(|entry| !descends_from(entry.pid, own, |pid| parents.get(&pid).copied()))
            .filter_map(|entry| {
                // Skipped without its path. No elevation is needed, so in practice only a process
                // that just exited fails here.
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
        // SAFETY: the handle is checked against INVALID_HANDLE_VALUE and closed by `OwnedHandle` on
        // every path, including the early return.
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
                parent: entry.th32ParentProcessID,
                name: wide_to_string(&entry.szExeFile),
            });
            // SAFETY: same handle, and `entry` is fully initialised by the previous call.
            if unsafe { Process32NextW(snapshot.0, &mut entry) } == 0 {
                break;
            }
        }
        Some(entries)
    }

    pub(super) fn executable_of(pid: u32) -> Option<PathBuf> {
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

/// The macOS counterpart of the Toolhelp snapshot, using `proc_listallpids` and `proc_pidpath`.
///
/// Another user's processes have no readable path, and only this user's Codex matters.
#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{OsStr, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use super::{ClientPresence, RunningClient, classify, descends_from, is_codex_executable};

    /// `PROC_PIDPATHINFO_MAXSIZE`: four times `MAXPATHLEN`, which is what `proc_pidpath` wants.
    const PATH_BUFFER: usize = 4096;

    /// `PROC_PIDT_SHORTBSDINFO`: the flavour of `proc_pidinfo` that answers with a process's
    /// parent in a 68-byte structure, rather than the full `proc_bsdinfo`.
    const PROC_PIDT_SHORTBSDINFO: i32 = 13;

    /// `struct proc_bsdshortinfo` from `<sys/proc_info.h>`, laid out exactly as the header has
    /// it. Only `pbsi_ppid` is read; the rest is here so the size is right.
    #[repr(C)]
    struct ProcBsdShortInfo {
        pbsi_pid: u32,
        pbsi_ppid: u32,
        pbsi_pgid: u32,
        pbsi_status: u32,
        pbsi_comm: [u8; 16],
        pbsi_flags: u32,
        pbsi_uid: u32,
        pbsi_gid: u32,
        pbsi_ruid: u32,
        pbsi_rgid: u32,
        pbsi_svuid: u32,
        pbsi_svgid: u32,
        pbsi_rfu: u32,
    }

    /// Headroom over the count the first call reports, because processes start while this runs.
    const PID_HEADROOM: usize = 256;

    #[link(name = "proc")]
    unsafe extern "C" {
        /// Returns the number of process ids written, not a byte count.
        fn proc_listallpids(buffer: *mut c_void, buffersize: i32) -> i32;
        /// Writes the executable path. Returns its length, or 0 when the path cannot be read.
        fn proc_pidpath(pid: i32, buffer: *mut c_void, buffersize: u32) -> i32;
        /// Fills `buffer` with the `flavor` structure for `pid`. Returns the bytes written, or
        /// 0 when the process cannot be read.
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut c_void,
            buffersize: i32,
        ) -> i32;
    }

    pub fn running_clients(exclude: &[u32]) -> ClientPresence {
        let Some(pids) = all_pids() else {
            // A failed scan is not "nothing running"; a credential replacement depends on the
            // answer.
            return ClientPresence::Unknown;
        };

        let own = std::process::id();
        let clients = pids
            .into_iter()
            .filter(|pid| !exclude.contains(pid))
            .filter_map(|pid| {
                // An unreadable path means another user's process or one that already exited.
                let executable = executable_of(pid)?;
                let name = executable.file_name()?.to_string_lossy().into_owned();
                is_codex_executable(&name).then(|| RunningClient {
                    pid,
                    kind: classify(&executable),
                    executable,
                })
            })
            // After the name check, so the parent walk is only ever done for a Codex.
            .filter(|client| !descends_from(client.pid, own, parent_of))
            .collect();

        ClientPresence::Known(clients)
    }

    /// The parent of `pid`, or `None` when the process has gone or cannot be read.
    pub(super) fn parent_of(pid: u32) -> Option<u32> {
        let pid = i32::try_from(pid).ok()?;
        let mut info = std::mem::MaybeUninit::<ProcBsdShortInfo>::uninit();
        let size = i32::try_from(size_of::<ProcBsdShortInfo>()).ok()?;
        // SAFETY: `info` is writable for `size` bytes, which is exactly what this flavour
        // fills; the call writes at most that and reports how much.
        let written = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDT_SHORTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if written != size {
            return None;
        }
        // SAFETY: the call reported the whole structure written.
        let info = unsafe { info.assume_init() };
        Some(info.pbsi_ppid)
    }

    pub(super) fn all_pids() -> Option<Vec<u32>> {
        // SAFETY: the documented way to ask for the count - a null buffer of length zero.
        let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return None;
        }
        let mut pids = vec![0i32; count as usize + PID_HEADROOM];
        let bytes = i32::try_from(pids.len() * size_of::<i32>()).ok()?;
        // SAFETY: the buffer is live and `bytes` is exactly its size.
        let written = unsafe { proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        if written <= 0 {
            return None;
        }
        pids.truncate((written as usize).min(pids.len()));
        // Process 0 is the kernel, which has no path and is not a Codex.
        Some(
            pids.into_iter()
                .filter_map(|pid| u32::try_from(pid).ok())
                .filter(|pid| *pid != 0)
                .collect(),
        )
    }

    pub(super) fn executable_of(pid: u32) -> Option<PathBuf> {
        let mut buffer = [0u8; PATH_BUFFER];
        let pid = i32::try_from(pid).ok()?;
        let length = i32::try_from(buffer.len()).ok()?;
        // SAFETY: the buffer is live and writable for `length` bytes, which is all the call
        // requires; it writes at most that and returns how much.
        let written = unsafe { proc_pidpath(pid, buffer.as_mut_ptr().cast(), length as u32) };
        if written <= 0 {
            return None;
        }
        let written = usize::try_from(written).ok()?.min(buffer.len());
        Some(Path::new(OsStr::from_bytes(&buffer[..written])).to_path_buf())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::ClientPresence;

    /// Unsupported platform: `Unknown` rather than an empty list, so a switch cannot proceed
    /// blindly.
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
    fn the_desktop_applications_own_codex_is_recognised() {
        let path = Path::new("/Applications/ChatGPT.app/Contents/Resources/codex");

        assert_eq!(classify(path), ClientKind::DesktopApp);
    }

    #[test]
    fn an_editor_extension_inside_a_bundle_is_still_an_editor_extension() {
        // An editor is an application too, so the bundle rule must not swallow the extension
        // rule: an editor session blocks a switch, an application is closed and reopened.
        let path = Path::new(
            "/Users/x/.vscode/extensions/openai.chatgpt-26.820.60940-darwin-arm64/bin/codex",
        );

        assert_eq!(classify(path), ClientKind::IdeExtension);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_scan_reads_this_very_process() {
        // A miscounted buffer silently scans only part of the machine; finding this very process
        // catches that.
        let own = std::process::id();

        let path = executable_of(own).expect("this process has a path");
        let pids = running_pids().expect("the scan runs");

        assert!(path.is_absolute());
        assert!(
            pids.contains(&own),
            "the scan missed the process running it"
        );
    }

    #[test]
    fn a_child_and_a_grandchild_descend_from_the_root_and_a_stranger_does_not() {
        // pid → parent. 10 is Toglet; 20 its app server; 30 a helper that server started; 40 a
        // Codex somebody else is running under their shell 41, under launchd.
        let parent_of = |pid: u32| match pid {
            20 => Some(10),
            30 => Some(20),
            40 => Some(41),
            41 => Some(1),
            10 => Some(1),
            _ => None,
        };

        assert!(descends_from(20, 10, parent_of));
        assert!(descends_from(30, 10, parent_of));
        assert!(!descends_from(40, 10, parent_of));
        assert!(
            !descends_from(10, 10, parent_of),
            "a process is not its own descendant"
        );
        assert!(
            !descends_from(99, 10, parent_of),
            "a process that cannot be read is not claimed"
        );
    }

    #[test]
    fn a_looping_parent_chain_ends_rather_than_hanging() {
        // Reused ids can make the chain loop. It must end, and it must not claim the process.
        let parent_of = |pid: u32| Some(if pid == 50 { 51 } else { 50 });

        assert!(!descends_from(50, 10, parent_of));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_process_this_one_started_is_known_to_be_its_own() {
        // A child started here has this process as its parent. `sleep` is a program, not a shell.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep is on every macOS");
        let own = std::process::id();

        let parent = platform::parent_of(child.id());
        let descends = descends_from(child.id(), own, platform::parent_of);
        drop(child.kill());
        drop(child.wait());

        assert_eq!(parent, Some(own));
        assert!(descends);
        assert!(!descends_from(own, own, platform::parent_of));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_probe_answers_rather_than_refusing_to_say() {
        // `Unknown` blocks every switch; the probe must give an answer, even an empty one.
        let presence = SystemClientProbe::new().running_clients(&[]);

        assert!(matches!(presence, ClientPresence::Known(_)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_excluded_process_is_left_out() {
        let own = std::process::id();

        let ClientPresence::Known(_) = SystemClientProbe::new().running_clients(&[own]) else {
            panic!("the scan runs");
        };
        // Nothing stronger can be asserted here without a Codex running, and a test that needs
        // one would be a test that passes on one machine.
        assert!(running_pids().expect("the scan runs").contains(&own));
    }

    #[test]
    fn an_editor_extensions_own_runtime_is_not_mistaken_for_the_shared_one() {
        // The extension ships its own build; misread as the shared runtime, an editor session would
        // be closed and reopened instead of blocking the switch.
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

    /// The probe must not offer an answer to "is it busy?"; scanning the source guards future
    /// edits.
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
        // Enumeration must work as an ordinary user; whether any Codex is running is beside the
        // point.
        let presence = SystemClientProbe::new().running_clients(&[]);

        assert!(
            matches!(presence, ClientPresence::Known(_)),
            "enumerating processes must not need a privilege the app does not have"
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_excluded_process_is_left_out() {
        // Excluding the current process exercises the filter even though it is not a Codex client.
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
