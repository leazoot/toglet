//! The seven checks that have to pass before any authentication is replaced.
//!
//! They run in a fixed order and stop at the first one that fails, naming which. The order is
//! not cosmetic: the lock comes first so two switches cannot interleave, and the snapshot comes
//! last so it is taken from a state the other six checks have already agreed on.
//!
//! [`PreflightPassed`] can only be built by [`Preflight::run`]. The replacement in `swap` takes
//! one, so there is no path into it that has not been through here.

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

/// The global switch lock.
///
/// One switch at a time, and a second one is **refused rather than queued**. Queuing would
/// leave a user waiting behind a switch they cannot see, and would let a click made under one
/// set of facts execute against another.
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
    /// The guard is built inside the `if` rather than passed to `then_some`, which evaluates
    /// its argument eagerly: that version constructed a guard even when the compare-and-swap
    /// had failed, and dropping that temporary released the lock **somebody else was holding**.
    /// A single-threaded test cannot see it; the sixteen-thread race in `tests/contention.rs`
    /// reported eight winners.
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
    /// Only the shared managed runtime the desktop app runs from. The switch proceeds, and the
    /// user is told to close Codex and reopen it - a running process keeps using the old
    /// credentials without complaining.
    DesktopOnly,
    /// A CLI or an editor extension session, or an installation Toglet does not recognise.
    /// Blocked by default.
    Blocked,
    /// The probe could not run. Treated as blocking, because an empty answer here would be a
    /// guess with a credential replacement riding on it.
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
    if clients
        .iter()
        .all(|client| client.kind == ClientKind::ManagedRuntime)
    {
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
///
/// [`std::fmt::Debug`] is written by hand rather than derived: this holds the credentials that
/// are about to be installed, and a derived implementation would put them into every test
/// failure message and every `{:?}` a future caller reaches for.
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

        // 4. And they have to identify somebody. This runs in a throwaway home, so the default
        //    authentication is untouched by the check.
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

        // 5. A running client keeps using the credentials it started with, so replacing them
        //    underneath one produces exactly the disagreement between Toglet and Codex that the
        //    honest-state rule exists to prevent.
        let (presence, verdict) = check_clients(self.probe, self.own_processes)?;

        // 6. Proven by writing, not by reading a permission bit: a directory can look writable
        //    and still refuse the file `atomic_write` needs to create.
        check_writable(self.default_home)?;

        // 7. Last, so what is captured is the state the checks above agreed on. This also
        //    refuses to continue when somebody else has signed in outside Toglet - replacing
        //    their session without saying so would be wrong.
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
