//! The seven pre-checks, each failing on its own.
//!
//! They run as a sequence, which means every one of them has to be reachable and every one of
//! them has to be able to stop the switch by itself. These tests drive each in turn and assert
//! which step reported the failure - a check that cannot be made to fail is a check nobody
//! knows works.

mod support;

use std::path::Path;

use support::{fake_binary, fake_server_binary};
use toglet_lib::accounts::external_change::ActiveAccount;
use toglet_lib::accounts::fingerprint;
use toglet_lib::app_server::CodexBinary;
use toglet_lib::codex_home::IsolatedHome;
use toglet_lib::credentials::{
    CredentialLock, CredentialRef, MemorySecretStore, Secret, SecretStore,
};
use toglet_lib::diagnostics::{ErrorCode, Phase};
use toglet_lib::process::{ClientKind, ClientPresence, ClientProbe, RunningClient};
use toglet_lib::switching::{
    ClientVerdict, Preflight, PreflightStep, SwitchLock, SwitchTarget, verdict,
};

const PHASE: Phase = Phase::Precheck;
const ACCOUNT_ID: &str = "8f14e45f-ceea-467a-9f3a-1c2d3e4f5a6b";
const OTHER_ACCOUNT_ID: &str = "1a2b3c4d-0000-4000-8000-abcdefabcdef";

/// A probe that answers whatever the test says, so every branch of the client check is
/// reachable without any Codex actually running.
struct FakeProbe(ClientPresence);

impl ClientProbe for FakeProbe {
    fn running_clients(&self, exclude: &[u32]) -> ClientPresence {
        match &self.0 {
            ClientPresence::Unknown => ClientPresence::Unknown,
            ClientPresence::Known(clients) => ClientPresence::Known(
                clients
                    .iter()
                    .filter(|client| !exclude.contains(&client.pid))
                    .cloned()
                    .collect(),
            ),
        }
    }
}

fn nothing_running() -> FakeProbe {
    FakeProbe(ClientPresence::Known(Vec::new()))
}

fn auth_json(account_id: &str) -> Vec<u8> {
    format!(r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account_id}"}}}}"#).into_bytes()
}

/// The pieces every pre-check run needs.
struct World {
    home: IsolatedHome,
    store: MemorySecretStore,
    lock: SwitchLock,
    credential_lock: CredentialLock,
    binary: CodexBinary,
    target_ref: CredentialRef,
    active_ref: CredentialRef,
}

impl World {
    /// A default home already signed in as `ACCOUNT_ID`, and a target account holding
    /// `OTHER_ACCOUNT_ID`.
    fn new() -> Self {
        let home = IsolatedHome::create(PHASE).expect("scratch home");
        std::fs::write(home.path().join("auth.json"), auth_json(ACCOUNT_ID)).expect("written");

        let store = MemorySecretStore::new();
        let active_ref = CredentialRef::new("active").expect("valid");
        let target_ref = CredentialRef::new("target").expect("valid");
        store
            .store(&active_ref, &Secret::new(auth_json(ACCOUNT_ID)))
            .expect("stored");
        store
            .store(&target_ref, &Secret::new(auth_json(OTHER_ACCOUNT_ID)))
            .expect("stored");

        Self {
            home,
            store,
            lock: SwitchLock::new(),
            credential_lock: CredentialLock::new(),
            binary: fake_binary(PHASE),
            target_ref,
            active_ref,
        }
    }

    fn preflight<'a>(&'a self, probe: &'a dyn ClientProbe) -> Preflight<'a> {
        Preflight {
            lock: &self.lock,
            credential_lock: &self.credential_lock,
            store: &self.store,
            probe,
            binary: &self.binary,
            default_home: self.home.path(),
            own_processes: &[],
        }
    }

    fn active(&self) -> ActiveAccount<'_> {
        ActiveAccount {
            credentials: &self.active_ref,
            fingerprint: ACTIVE_FINGERPRINT
                .get_or_init(|| fingerprint::from_account_id(ACCOUNT_ID)),
        }
    }

    fn target(&self) -> SwitchTarget<'_> {
        SwitchTarget {
            account_id: "target-account",
            credentials: &self.target_ref,
        }
    }
}

static ACTIVE_FINGERPRINT: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[test]
fn all_seven_checks_pass_when_nothing_is_wrong() {
    let world = World::new();
    let probe = nothing_running();

    let passed = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect("nothing should have stopped this switch");

    assert_eq!(passed.verdict, ClientVerdict::Clear);
    assert!(
        passed.clients.is_empty(),
        "nothing was running, so nothing may be reported as running"
    );
}

#[test]
fn check_one_a_second_switch_is_refused_rather_than_queued() {
    let world = World::new();
    let probe = nothing_running();
    let _first = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect("the first switch passes");

    let failure = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect_err("a concurrent switch must be refused");

    assert_eq!(failure.step, PreflightStep::Lock);
    assert_eq!(failure.error.code(), ErrorCode::SwitchInProgress);
}

#[test]
fn check_two_switching_to_the_account_already_in_use_is_refused() {
    let world = World::new();
    let probe = nothing_running();

    let failure = world
        .preflight(&probe)
        .run(Some("target-account"), Some(world.active()), world.target())
        .expect_err("there is nothing to switch to");

    assert_eq!(failure.step, PreflightStep::Target);
    assert_eq!(failure.error.code(), ErrorCode::AlreadyActive);
    assert!(!failure.error.retryable(), "retrying changes nothing");
}

#[test]
fn check_three_credentials_that_are_not_there_stop_the_switch() {
    let world = World::new();
    let probe = nothing_running();
    world.store.delete(&world.target_ref).expect("removed");

    let failure = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect_err("credentials that cannot be loaded stop the switch");

    assert_eq!(failure.step, PreflightStep::Credentials);
}

#[test]
fn check_four_credentials_that_identify_nobody_stop_the_switch() {
    let world = World::new();
    let probe = nothing_running();
    world
        .store
        .store(&world.target_ref, &Secret::new(b"not json at all".to_vec()))
        .expect("stored");

    let failure = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect_err("unusable credentials must never reach the replacement");

    assert_eq!(failure.step, PreflightStep::Identity);
}

#[test]
fn check_five_an_editor_session_stops_the_switch() {
    let world = World::new();
    let probe = FakeProbe(ClientPresence::Known(vec![RunningClient {
        pid: 4242,
        kind: ClientKind::IdeExtension,
        executable: std::path::PathBuf::from("codex.exe"),
    }]));

    let failure = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect_err("a running editor session blocks the switch");

    assert_eq!(failure.step, PreflightStep::Clients);
    assert_eq!(failure.error.code(), ErrorCode::ClientRunning);
}

#[test]
fn check_five_a_probe_that_could_not_run_stops_the_switch() {
    let world = World::new();
    let probe = FakeProbe(ClientPresence::Unknown);

    let failure = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect_err("not knowing what is running is not the same as nothing running");

    assert_eq!(failure.step, PreflightStep::Clients);
}

#[test]
fn check_five_the_desktop_runtime_alone_does_not_stop_the_switch() {
    let world = World::new();
    let probe = FakeProbe(ClientPresence::Known(vec![RunningClient {
        pid: 99,
        kind: ClientKind::ManagedRuntime,
        executable: std::path::PathBuf::from("codex.exe"),
    }]));

    let passed = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect("the desktop case is a prompt, not a refusal");

    assert_eq!(passed.verdict, ClientVerdict::DesktopOnly);
    assert_eq!(
        passed.clients.len(),
        1,
        "the restart path may only use an executable read from a running process"
    );
}

#[test]
fn check_five_toglets_own_app_servers_do_not_block_its_own_switch() {
    let world = World::new();
    let probe = FakeProbe(ClientPresence::Known(vec![RunningClient {
        pid: 7,
        kind: ClientKind::Cli,
        executable: std::path::PathBuf::from("codex.exe"),
    }]));

    let mut preflight = world.preflight(&probe);
    let own = [7u32];
    preflight.own_processes = &own;

    preflight
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect("a quota refresh Toglet started must not block Toglet");
}

#[test]
fn check_six_a_home_that_cannot_be_written_stops_the_switch() {
    let world = World::new();
    let probe = nothing_running();
    // A path that is not a directory: creating the probe file inside it cannot succeed, which
    // is exactly the condition `atomic_write` would hit later.
    let unwritable = world.home.path().join("auth.json");

    let mut preflight = world.preflight(&probe);
    preflight.default_home = Path::new(&unwritable);

    let failure = preflight
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect_err("a home that cannot be written to stops the switch before it starts");

    assert_eq!(failure.step, PreflightStep::Writable);
    assert_eq!(failure.error.code(), ErrorCode::CodexHomeUnwritable);
}

#[test]
fn check_six_leaves_no_probe_file_behind() {
    let world = World::new();
    let probe = nothing_running();

    world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect("the checks pass");

    let leftovers: Vec<_> = std::fs::read_dir(world.home.path())
        .expect("readable")
        .filter_map(|entry| {
            let name = entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            name.contains("write-probe").then_some(name)
        })
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[test]
fn check_seven_a_sign_in_that_happened_outside_toglet_stops_the_switch() {
    // The default home now holds somebody Toglet does not know about. Replacing
    // it would discard their session without ever mentioning it.
    let world = World::new();
    let probe = nothing_running();
    std::fs::write(
        world.home.path().join("auth.json"),
        auth_json("cccccccc-0000-4000-8000-cccccccccccc"),
    )
    .expect("written");

    let failure = world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect_err("an unrecognised session must be resolved first");

    assert_eq!(failure.step, PreflightStep::Snapshot);
    assert_eq!(failure.error.code(), ErrorCode::ExternalAuthChange);
}

#[test]
fn check_seven_stores_the_current_credentials_before_anything_is_replaced() {
    // The snapshot is what a rollback and a later switch back both depend on.
    let world = World::new();
    let probe = nothing_running();
    let refreshed = auth_json(ACCOUNT_ID);
    let mut with_extra = refreshed.clone();
    with_extra.extend_from_slice(b"\n");
    std::fs::write(world.home.path().join("auth.json"), &with_extra).expect("written");

    world
        .preflight(&probe)
        .run(Some("active-account"), Some(world.active()), world.target())
        .expect("the checks pass");

    assert_eq!(
        world
            .store
            .load(&world.active_ref)
            .expect("readable")
            .expose(),
        with_extra,
        "the snapshot must hold what was actually on disk"
    );
}

#[test]
fn the_lock_is_released_when_the_checks_fail() {
    let world = World::new();
    let probe = nothing_running();

    world
        .preflight(&probe)
        .run(Some("target-account"), Some(world.active()), world.target())
        .expect_err("this run fails at check two");

    assert!(
        world.lock.try_acquire().is_some(),
        "a failed switch must not hold the lock for the rest of the session"
    );
}

#[test]
fn the_verdict_helper_and_the_probe_agree_about_an_empty_machine() {
    // Guards against the two ever drifting apart: the pre-check uses `verdict`, and the fake
    // used here is the same shape the real probe returns.
    assert!(fake_server_binary().is_file());
    assert_eq!(
        verdict(&ClientPresence::Known(Vec::new())),
        ClientVerdict::Clear
    );
}
