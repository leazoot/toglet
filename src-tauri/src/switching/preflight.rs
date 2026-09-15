//! The seven checks that must pass before any authentication is replaced.
//! Fixed order, stopping at the first failure: the lock first so switches cannot interleave, the
//! snapshot last so it reflects the agreed state. `swap` requires a `PreflightPassed`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::accounts::external_change::{self, ActiveAccount, ExternalChange};
use crate::accounts::onboarding::{self, VerifiedCredentials};
use crate::app_server::CodexBinary;
use crate::codex_home::IsolatedHome;
use crate::credentials::{CredentialLock, CredentialRef, SecretStore};
use crate::diagnostics::{ErrorCode, Phase, TogletError, UserAction};
use crate::process::{ClientKind, ClientPresence, ClientProbe, RunningClient};

const PHASE: Phase = Phase::Precheck;

/// The name of the file used to prove the default home can be written to.
const WRITE_PROBE: &str = ".toglet-write-probe";

/// Which of the seven checks stopped the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightStep {
    /// 1. Take the global switch lock.
    Lock,
    /// 2. The target is not already the active account.
    Target,
    /// 3. The target's credentials exist and decrypt.
    Credentials,
    /// 4. An app server in a throwaway home agrees whose they are.
    Identity,
    /// 5. No running client would ignore the new authentication.
    Clients,
    /// 6. The default home can actually be written to.
    Writable,
    /// 7. The authentication about to be replaced is snapshotted first.
    Snapshot,
}

/// A pre-check that did not pass, and which one it was.
#[derive(Debug)]
pub struct PreflightFailure {
    pub step: PreflightStep,
    pub error: TogletError,
}

/// The global switch lock. A second switch is refused rather than queued, so a click cannot
/// run against facts that changed while it waited.
#[derive(Debug, Default)]
pub struct SwitchLock {
    busy: AtomicBool,
}

impl SwitchLock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the lock, or returns `None` if a switch is already running.
    ///
    /// The guard must be built inside the `if`: `then_some` evaluates eagerly, and dropping a guard
    /// built after a failed compare-and-swap would release another holder's lock.
    pub fn try_acquire(&self) -> Option<SwitchGuard<'_>> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Some(SwitchGuard { lock: self })
        } else {
            None
        }
    }
}

/// Holds the switch lock for as long as it exists.
#[derive(Debug)]
pub struct SwitchGuard<'a> {
    lock: &'a SwitchLock,
}

impl Drop for SwitchGuard<'_> {
    fn drop(&mut self) {
        self.lock.busy.store(false, Ordering::Release);
    }
}

/// What the running clients mean for a switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientVerdict {
    /// Nothing is running. The switch can proceed silently.
    Clear,
    /// Only the desktop app or its managed runtime. The switch proceeds and Codex must be reopened:
    /// a running process keeps using the old credentials.
    DesktopOnly,
    /// A CLI or an editor extension session, or an installation Toglet does not recognise.
    /// Blocked by default.
    Blocked,
    /// The probe could not run. Treated as blocking rather than guessed.
    Unknown,
}

/// Turns what is running into what to do about it.
pub fn verdict(presence: &ClientPresence) -> ClientVerdict {
    let ClientPresence::Known(clients) = presence else {
        return ClientVerdict::Unknown;
    };
    if clients.is_empty() {
        return ClientVerdict::Clear;
    }
    // Both are whole applications, which can be asked to quit and started again around the
    // switch. A terminal session cannot, which is why it blocks instead.
    if clients.iter().all(|client| {
        matches!(
            client.kind,
            ClientKind::ManagedRuntime | ClientKind::DesktopApp
        )
    }) {
        return ClientVerdict::DesktopOnly;
    }
    ClientVerdict::Blocked
}

/// The account a switch is aiming at.
#[derive(Debug, Clone, Copy)]
pub struct SwitchTarget<'a> {
    pub account_id: &'a str,
    pub credentials: &'a CredentialRef,
}

/// Everything the seven checks need.
pub struct Preflight<'a> {
    pub lock: &'a SwitchLock,
    pub credential_lock: &'a CredentialLock,
    pub store: &'a dyn SecretStore,
    pub probe: &'a dyn ClientProbe,
    pub binary: &'a CodexBinary,
    /// The user's real Codex home - the one the switch will write to.
    pub default_home: &'a Path,
    /// Process ids Toglet started itself, which must not block Toglet's own switch.
    pub own_processes: &'a [u32],
}

/// Proof the seven checks passed, and what they produced.
/// `Debug` is hand-written so the credentials held here never reach a `{:?}`.
pub struct PreflightPassed<'a> {
    /// Held for the lifetime of the switch. Dropping this releases it.
    pub guard: SwitchGuard<'a>,
    /// The credentials to install, already verified against a throwaway app server.
    pub target: VerifiedCredentials,
    /// What was running when the checks passed. The restart path may only use these
    /// executables.
    pub clients: Vec<RunningClient>,
    pub verdict: ClientVerdict,
}

impl std::fmt::Debug for PreflightPassed<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreflightPassed")
            .field("verdict", &self.verdict)
            .field("clients", &self.clients.len())
            .finish_non_exhaustive()
    }
}

impl<'a> Preflight<'a> {
    /// Runs the seven checks, stopping at the first failure.
    pub fn run(
        &self,
        active_account_id: Option<&str>,
        active: Option<ActiveAccount<'_>>,
        target: SwitchTarget<'_>,
    ) -> Result<PreflightPassed<'a>, PreflightFailure> {
        // 1. The lock, before anything is read, so two switches cannot both pass their checks.
        let guard = take_lock(self.lock)?;

        // 2. Switching to the account already in use does nothing but risk something.
        if active_account_id == Some(target.account_id) {
            return Err(failure(
                PreflightStep::Target,
                ErrorCode::AlreadyActive,
                false,
                UserAction::None,
                "the target is already the active account",
            ));
        }

        // 3. The credentials have to exist and decrypt before anything else is disturbed.
        let secret = self
            .store
            .load(target.credentials)
            .map_err(|error| PreflightFailure {
                step: PreflightStep::Credentials,
                error,
            })?;

        // 4. They must identify somebody, checked in a throwaway home.
        let home = IsolatedHome::create(PHASE).map_err(|error| PreflightFailure {
            step: PreflightStep::Identity,
            error,
        })?;
        let verified = onboarding::verify(self.binary, home, secret, PHASE).map_err(|error| {
            PreflightFailure {
                step: PreflightStep::Identity,
                error,
            }
        })?;

        // 5. A running client keeps the credentials it started with and would silently disagree.
        let (presence, verdict) = check_clients(self.probe, self.own_processes)?;

        // 6. Proven by writing, not by reading a permission bit: a directory can look writable
        //    and still refuse the file `atomic_write` needs to create.
        check_writable(self.default_home)?;

        // 7. Last, so it captures the state the checks agreed on. Also refuses a sign-in made
        //    outside Toglet.
        snapshot_current(self.credential_lock, self.store, active, self.default_home)?;

        let clients = match presence {
            ClientPresence::Known(clients) => clients,
            ClientPresence::Unknown => Vec::new(),
        };

        Ok(PreflightPassed {
            guard,
            target: verified,
            clients,
            verdict,
        })
    }
}

/// Step 1, shared with the sign-out: the lock, or a refusal to queue behind another switch.
pub(super) fn take_lock(lock: &SwitchLock) -> Result<SwitchGuard<'_>, PreflightFailure> {
    lock.try_acquire().ok_or_else(|| {
        failure(
            PreflightStep::Lock,
            ErrorCode::SwitchInProgress,
            true,
            UserAction::WaitForSwitch,
            "another switch is already running",
        )
    })
}

/// Step 5, shared with the sign-out: what is running, and whether it stops the operation.
pub(super) fn check_clients(
    probe: &dyn ClientProbe,
    own_processes: &[u32],
) -> Result<(ClientPresence, ClientVerdict), PreflightFailure> {
    let presence = probe.running_clients(own_processes);
    let verdict = verdict(&presence);
    match verdict {
        ClientVerdict::Blocked => Err(failure(
            PreflightStep::Clients,
            ErrorCode::ClientRunning,
            true,
            UserAction::CloseCodexClient,
            "a CLI or editor session is running",
        )),
        ClientVerdict::Unknown => Err(failure(
            PreflightStep::Clients,
            ErrorCode::ClientRunning,
            true,
            UserAction::CloseCodexClient,
            "running clients could not be determined",
        )),
        ClientVerdict::Clear | ClientVerdict::DesktopOnly => Ok((presence, verdict)),
    }
}

/// Step 6, shared with the sign-out: the home is proven writable by writing to it.
pub(super) fn check_writable(default_home: &Path) -> Result<(), PreflightFailure> {
    let probe = default_home.join(WRITE_PROBE);
    let created = crate::codex_home::permissions::create_private_file(&probe);
    let outcome = match created {
        Ok(file) => {
            drop(file);
            Ok(())
        }
        Err(error) => Err(failure(
            PreflightStep::Writable,
            ErrorCode::CodexHomeUnwritable,
            true,
            UserAction::FixPermissions,
            &error.to_string(),
        )),
    };
    // Best effort, and only ever removes a file this function just created.
    drop(std::fs::remove_file(&probe));
    outcome
}

/// Step 7, shared with the sign-out: the authentication about to be touched is snapshotted,
/// and a sign-in made outside Toglet stops everything.
pub(super) fn snapshot_current(
    credential_lock: &CredentialLock,
    store: &dyn SecretStore,
    active: Option<ActiveAccount<'_>>,
    default_home: &Path,
) -> Result<(), PreflightFailure> {
    let change = external_change::synchronise(credential_lock, store, active, default_home, PHASE)
        .map_err(|error| PreflightFailure {
            step: PreflightStep::Snapshot,
            error,
        })?;

    match change {
        ExternalChange::Unchanged | ExternalChange::SnapshotUpdated | ExternalChange::SignedOut => {
            Ok(())
        }
        ExternalChange::ExternalLogin { .. } => Err(failure(
            PreflightStep::Snapshot,
            ErrorCode::ExternalAuthChange,
            false,
            UserAction::ResolveExternalChange,
            "somebody signed in outside Toglet since the last synchronisation",
        )),
        ExternalChange::NotUnderstood => Err(failure(
            PreflightStep::Snapshot,
            ErrorCode::AuthFileConflict,
            true,
            UserAction::Retry,
            "the current authentication could not be read",
        )),
    }
}

fn failure(
    step: PreflightStep,
    code: ErrorCode,
    retryable: bool,
    action: UserAction,
    detail: &str,
) -> PreflightFailure {
    PreflightFailure {
        step,
        error: TogletError::new(code, PHASE, retryable, action).with_detail(detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(kind: ClientKind) -> RunningClient {
        RunningClient {
            pid: 1,
            kind,
            executable: std::path::PathBuf::from("codex.exe"),
        }
    }

    #[test]
    fn nothing_running_clears_the_way() {
        assert_eq!(
            verdict(&ClientPresence::Known(Vec::new())),
            ClientVerdict::Clear
        );
    }

    #[test]
    fn only_the_desktop_runtime_means_close_and_reopen_rather_than_stop() {
        let presence = ClientPresence::Known(vec![client(ClientKind::ManagedRuntime)]);

        assert_eq!(verdict(&presence), ClientVerdict::DesktopOnly);
    }

    #[test]
    fn a_desktop_application_also_means_close_and_reopen() {
        // The macOS desktop app; unrecognised, it would block every switch while it is open.
        let presence = ClientPresence::Known(vec![client(ClientKind::DesktopApp)]);

        assert_eq!(verdict(&presence), ClientVerdict::DesktopOnly);
    }

    #[test]
    fn a_terminal_session_beside_a_desktop_application_still_blocks() {
        let presence = ClientPresence::Known(vec![
            client(ClientKind::DesktopApp),
            client(ClientKind::Cli),
        ]);

        assert_eq!(verdict(&presence), ClientVerdict::Blocked);
    }

    #[test]
    fn a_cli_session_blocks_the_switch() {
        let presence = ClientPresence::Known(vec![client(ClientKind::Cli)]);

        assert_eq!(verdict(&presence), ClientVerdict::Blocked);
    }

    #[test]
    fn an_editor_session_blocks_the_switch() {
        let presence = ClientPresence::Known(vec![client(ClientKind::IdeExtension)]);

        assert_eq!(verdict(&presence), ClientVerdict::Blocked);
    }

    #[test]
    fn an_unrecognised_installation_blocks_rather_than_being_waved_through() {
        let presence = ClientPresence::Known(vec![client(ClientKind::Unrecognised)]);

        assert_eq!(verdict(&presence), ClientVerdict::Blocked);
    }

    #[test]
    fn a_desktop_runtime_next_to_an_editor_session_still_blocks() {
        let presence = ClientPresence::Known(vec![
            client(ClientKind::ManagedRuntime),
            client(ClientKind::IdeExtension),
        ]);

        assert_eq!(
            verdict(&presence),
            ClientVerdict::Blocked,
            "the strictest running client decides"
        );
    }

    #[test]
    fn a_probe_that_could_not_run_is_not_read_as_nothing_running() {
        assert_eq!(verdict(&ClientPresence::Unknown), ClientVerdict::Unknown);
    }

    #[test]
    fn the_switch_lock_admits_one_holder_at_a_time() {
        let lock = SwitchLock::new();

        let first = lock.try_acquire().expect("the lock is free");
        assert!(
            lock.try_acquire().is_none(),
            "a second switch must be refused, not queued"
        );

        drop(first);
        assert!(lock.try_acquire().is_some(), "the lock is released on drop");
    }
}
