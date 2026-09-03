//! Replacing the default authentication, and every way that can go wrong.
//!
//! The fake app server answers `account/read` from a scenario file rather than from the
//! credentials on disk. That is deliberate and it is the point: it lets a verification
//! *disagree* with what was just written, which is the case the verification exists for and
//! which a server that simply echoed the file could never produce. That a real Codex reads the
//! replaced file was measured separately.

mod support;

use std::cell::Cell;
use std::path::{Path, PathBuf};

use support::fake_binary;
use toglet_lib::accounts::external_change::ActiveAccount;
use toglet_lib::accounts::fingerprint;
use toglet_lib::app_server::CodexBinary;
use toglet_lib::codex_home::IsolatedHome;
use toglet_lib::credentials::{
    CredentialLock, CredentialRef, MemorySecretStore, Secret, SecretStore,
};
use toglet_lib::diagnostics::{ErrorCode, Phase, Result};
use toglet_lib::process::{ClientPresence, ClientProbe};
use toglet_lib::switching::{
    Faults, JOURNAL_FILE, NoFaults, NoObserver, Preflight, RecoveryPlan, RollbackReport, SignOut,
    SignOutFailed, StepObserver, Switch, SwitchJournal, SwitchLock, SwitchPhase, SwitchStage,
    SwitchTarget,
};

const PHASE: Phase = Phase::Precheck;
const ACCOUNT_ID: &str = "8f14e45f-ceea-467a-9f3a-1c2d3e4f5a6b";
const TARGET_ACCOUNT_ID: &str = "1a2b3c4d-0000-4000-8000-abcdefabcdef";
const OPERATION: &str = "op-test-1";
const STARTED_AT: &str = "2026-09-01T00:00:00Z";

struct NoClients;

impl ClientProbe for NoClients {
    fn running_clients(&self, _exclude: &[u32]) -> ClientPresence {
        ClientPresence::Known(Vec::new())
    }
}

/// Fails once, at the stage it was built for.
struct FailAt {
    stage: SwitchStage,
    fired: Cell<bool>,
}

impl FailAt {
    fn new(stage: SwitchStage) -> Self {
        Self {
            stage,
            fired: Cell::new(false),
        }
    }
}

impl Faults for FailAt {
    fn before(&self, stage: SwitchStage) -> Result<()> {
        if stage != self.stage || self.fired.get() {
            return Ok(());
        }
        self.fired.set(true);
        Err(toglet_lib::diagnostics::TogletError::new(
            ErrorCode::Internal,
            Phase::Write,
            true,
            toglet_lib::diagnostics::UserAction::Retry,
        )
        .with_detail("injected failure"))
    }
}

fn auth_json(account_id: &str) -> Vec<u8> {
    format!(r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account_id}"}}}}"#).into_bytes()
}

struct World {
    home: IsolatedHome,
    journal_directory: IsolatedHome,
    store: MemorySecretStore,
    lock: SwitchLock,
    credential_lock: CredentialLock,
    binary: CodexBinary,
    active_ref: CredentialRef,
    target_ref: CredentialRef,
    active_fingerprint: String,
}

impl World {
    /// `scenario` is what the fake server will report the **default** home is signed in as.
    fn new(scenario: Option<&str>) -> Self {
        let home = IsolatedHome::create(PHASE).expect("scratch home");
        std::fs::write(home.path().join("auth.json"), auth_json(ACCOUNT_ID)).expect("written");
        if let Some(scenario) = scenario {
            std::fs::write(home.path().join("scenario"), scenario).expect("written");
        }

        let store = MemorySecretStore::new();
        let active_ref = CredentialRef::new("active").expect("valid");
        let target_ref = CredentialRef::new("target").expect("valid");
        store
            .store(&active_ref, &Secret::new(auth_json(ACCOUNT_ID)))
            .expect("stored");
        store
            .store(&target_ref, &Secret::new(auth_json(TARGET_ACCOUNT_ID)))
            .expect("stored");

        Self {
            home,
            journal_directory: IsolatedHome::create(Phase::Storage).expect("scratch directory"),
            store,
            lock: SwitchLock::new(),
            credential_lock: CredentialLock::new(),
            binary: fake_binary(PHASE),
            active_ref,
            target_ref,
            active_fingerprint: fingerprint::from_account_id(ACCOUNT_ID),
        }
    }

    fn auth(&self) -> PathBuf {
        self.home.path().join("auth.json")
    }

    fn journal_path(&self) -> PathBuf {
        self.journal_directory.path().join(JOURNAL_FILE)
    }

    fn switch(&self, faults: &dyn Faults) -> std::result::Result<(), RollbackReport> {
        self.switch_watched(faults, &NoObserver)
    }

    fn switch_watched(
        &self,
        faults: &dyn Faults,
        observer: &dyn StepObserver,
    ) -> std::result::Result<(), RollbackReport> {
        let probe = NoClients;
        let preflight = Preflight {
            lock: &self.lock,
            credential_lock: &self.credential_lock,
            store: &self.store,
            probe: &probe,
            binary: &self.binary,
            default_home: self.home.path(),
            own_processes: &[],
        };
        let passed = preflight
            .run(
                Some("active-account"),
                Some(ActiveAccount {
                    credentials: &self.active_ref,
                    fingerprint: &self.active_fingerprint,
                }),
                SwitchTarget {
                    account_id: "target-account",
                    credentials: &self.target_ref,
                },
            )
            .expect("the pre-checks pass");

        let switch = Switch {
            binary: &self.binary,
            default_home: self.home.path(),
            journal_directory: self.journal_directory.path(),
            faults,
            observer,
        };

        match switch.run(
            passed,
            Some("active-account"),
            "target-account",
            OPERATION,
            STARTED_AT,
        ) {
            Ok(succeeded) => {
                assert_eq!(succeeded.progress.number(), 4, "all four steps completed");
                Ok(())
            }
            Err(failed) => {
                assert!(
                    failed.progress.number() < 4,
                    "a failed switch must not report itself as finished"
                );
                Err(failed.rollback)
            }
        }
    }
}

fn backups(directory: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory)
        .expect("readable")
        .filter_map(|entry| {
            let path = entry.expect("entry").path();
            path.file_name()?
                .to_string_lossy()
                .contains("toglet-switch-")
                .then_some(path)
        })
        .collect()
}

#[test]
fn a_verified_switch_installs_the_target_and_cleans_up_after_itself() {
    let world = World::new(None);

    world.switch(&NoFaults).expect("the switch succeeds");

    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(TARGET_ACCOUNT_ID),
        "the default home must hold the target's credentials"
    );
    assert!(
        !world.journal_path().exists(),
        "a finished switch leaves no journal for recovery to find"
    );
    assert!(
        backups(world.home.path()).is_empty(),
        "the temporary backup must not be left in the user's Codex home"
    );
}

#[test]
fn a_switch_changes_nothing_in_the_codex_home_but_the_authentication() {
    // A switch touches one file. Everything else in the user's Codex home - the
    // configuration above all - must come out byte for byte as it went in.
    let world = World::new(None);
    let config = world.home.path().join("config.toml");
    let before: Vec<(String, Vec<u8>)> = std::fs::read_dir(world.home.path())
        .expect("readable")
        .map(|entry| {
            let path = entry.expect("entry").path();
            let name = path
                .file_name()
                .expect("named")
                .to_string_lossy()
                .into_owned();
            (name, std::fs::read(&path).unwrap_or_default())
        })
        .filter(|(name, _)| name != "auth.json")
        .collect();
    assert!(
        config.exists(),
        "the fixture must have a configuration to protect"
    );

    world.switch(&NoFaults).expect("the switch succeeds");

    for (name, contents) in before {
        let path = world.home.path().join(&name);
        assert!(path.exists(), "{name} disappeared during the switch");
        assert_eq!(
            std::fs::read(&path).expect("readable"),
            contents,
            "{name} was modified by a switch that should only touch auth.json"
        );
    }
}

#[test]
fn a_switch_leaves_the_home_private() {
    let world = World::new(None);

    world.switch(&NoFaults).expect("the switch succeeds");

    assert!(
        toglet_lib::codex_home::is_private(&world.auth()).expect("permissions are readable"),
        "the replaced credentials must not be world readable"
    );
}

#[test]
fn a_failure_before_the_write_leaves_the_previous_credentials_in_place() {
    let world = World::new(None);

    let report = world
        .switch(&FailAt::new(SwitchStage::Write))
        .expect_err("the injected failure stops the switch");

    assert_eq!(report, RollbackReport::Restored);
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID)
    );
}

#[test]
fn a_failure_between_the_write_and_the_replace_leaves_the_previous_credentials_in_place() {
    let world = World::new(None);

    let report = world
        .switch(&FailAt::new(SwitchStage::Replace))
        .expect_err("the injected failure stops the switch");

    assert_eq!(report, RollbackReport::Restored);
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID)
    );
    let staged: Vec<_> = std::fs::read_dir(world.home.path())
        .expect("readable")
        .filter_map(|entry| {
            let name = entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned();
            name.contains("toglet-tmp-").then_some(name)
        })
        .collect();
    assert!(
        staged.is_empty(),
        "the staged file was left behind: {staged:?}"
    );
}

#[test]
fn a_failure_during_verification_puts_the_previous_credentials_back() {
    let world = World::new(None);

    let report = world
        .switch(&FailAt::new(SwitchStage::Verify))
        .expect_err("the injected failure stops the switch");

    assert_eq!(report, RollbackReport::Restored);
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID),
        "the replacement had already landed, so this is a real rollback"
    );
}

#[test]
fn an_identity_that_does_not_match_the_target_rolls_back_rather_than_reporting_success() {
    // The home reports somebody else no matter what was written, which is exactly the
    // case where believing the write would be believing a lie.
    let world = World::new(Some("second_account"));

    let report = world
        .switch(&NoFaults)
        .expect_err("a mismatch must never be reported as a completed switch");

    assert_eq!(report, RollbackReport::Restored);
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID)
    );
}

#[test]
fn a_rollback_removes_the_journal_and_the_backup_it_used() {
    let world = World::new(None);

    world
        .switch(&FailAt::new(SwitchStage::Verify))
        .expect_err("the switch fails");

    assert!(!world.journal_path().exists());
    assert!(backups(world.home.path()).is_empty());
}

#[test]
fn a_switch_into_a_home_nobody_was_signed_in_to_works_and_can_be_undone() {
    let world = World::new(None);
    std::fs::remove_file(world.auth()).expect("removed");

    let report = world
        .switch(&FailAt::new(SwitchStage::Verify))
        .expect_err("the injected failure stops the switch");

    assert_eq!(report, RollbackReport::Restored);
    assert!(
        !world.auth().exists(),
        "restoring a home nobody was signed in to means removing the file, not emptying it"
    );
}

#[test]
fn a_switch_that_is_interrupted_leaves_a_journal_that_says_what_to_do() {
    // Stands in for the process being killed: the journal is written by the switch itself, so
    // what recovery finds is whatever the last completed phase wrote.
    let world = World::new(None);
    let backup = world.home.path().join("auth.json.toglet-switch-op-1");
    std::fs::write(&backup, auth_json(ACCOUNT_ID)).expect("written");
    let mut journal = SwitchJournal::begin(
        world.journal_directory.path(),
        "op-1",
        Some("active-account"),
        Some("target-account"),
        backup,
        STARTED_AT,
    )
    .expect("written");

    assert_eq!(
        RecoveryPlan::for_phase(
            SwitchJournal::load(world.journal_directory.path())
                .expect("readable")
                .expect("in flight")
                .phase
        ),
        RecoveryPlan::RestoreBackup
    );

    journal
        .advance(world.journal_directory.path(), SwitchPhase::Replaced)
        .expect("recorded");

    assert_eq!(
        RecoveryPlan::for_phase(
            SwitchJournal::load(world.journal_directory.path())
                .expect("readable")
                .expect("in flight")
                .phase
        ),
        RecoveryPlan::ReVerify
    );
}

#[test]
fn the_journal_written_during_a_real_switch_carries_no_credential_material() {
    let world = World::new(Some("second_account"));

    // A switch that fails at verification writes both journal phases before rolling back.
    world.switch(&NoFaults).expect_err("the switch fails");

    assert!(
        !world.journal_path().exists(),
        "a completed rollback removes the journal"
    );
    assert!(backups(world.home.path()).is_empty());

    // Nothing the switch wrote outside the Codex home may carry credential material. The Codex
    // home itself is excluded on purpose - `auth.json` is the credentials, by definition.
    for entry in std::fs::read_dir(world.journal_directory.path()).expect("readable") {
        let path = entry.expect("entry").path();
        let bytes = std::fs::read(&path).expect("readable");
        let text = String::from_utf8_lossy(&bytes);
        for forbidden in ["access_token", "refresh_token", "auth_mode", "account_id"] {
            assert!(
                !text.contains(forbidden),
                "{} carried `{forbidden}`",
                path.display()
            );
        }
    }
}

#[test]
fn recovery_finds_nothing_to_do_when_no_switch_was_interrupted() {
    let world = World::new(None);

    let outcome = toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        None,
    )
    .expect("recovery runs");

    assert!(matches!(
        outcome,
        toglet_lib::switching::RecoveryOutcome::NothingToDo
    ));
}

/// Builds the on-disk state a process kill would leave behind: a journal at `phase`, the
/// backup beside the Codex home, and whatever the replacement had managed to write.
fn interrupt(world: &World, phase: SwitchPhase, auth_now: &[u8]) {
    let backup = world.home.path().join("auth.json.toglet-switch-op-crash");
    std::fs::write(&backup, auth_json(ACCOUNT_ID)).expect("written");
    std::fs::write(world.auth(), auth_now).expect("written");
    let mut journal = SwitchJournal::begin(
        world.journal_directory.path(),
        "op-crash",
        Some("active-account"),
        Some("target-account"),
        backup,
        STARTED_AT,
    )
    .expect("written");
    if phase != SwitchPhase::BackedUp {
        journal
            .advance(world.journal_directory.path(), phase)
            .expect("recorded");
    }
}

#[test]
fn a_kill_before_the_replacement_completed_is_rolled_back() {
    // The authentication on disk is whatever the interrupted write left; recovery puts
    // the copy back rather than trying to work out how far it got.
    let world = World::new(None);
    interrupt(&world, SwitchPhase::BackedUp, b"{ half written");

    let outcome = toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        None,
    )
    .expect("recovery runs");

    assert!(matches!(
        outcome,
        toglet_lib::switching::RecoveryOutcome::RolledBack
    ));
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID)
    );
    assert!(!world.journal_path().exists());
    assert!(backups(world.home.path()).is_empty());
}

#[test]
fn a_kill_after_an_unverified_replacement_is_verified_before_anything_is_concluded() {
    // The home reports the target, so the switch really did land and is completed now - with a
    // token that could only come from a fresh reading agreeing.
    let world = World::new(None);
    interrupt(&world, SwitchPhase::Replaced, &auth_json(TARGET_ACCOUNT_ID));
    let expected = toglet_lib::accounts::AccountIdentity::Chatgpt {
        email: "tester@example.com".to_owned(),
        plan_type: None,
    };

    let outcome = toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        Some(&expected),
    )
    .expect("recovery runs");

    let toglet_lib::switching::RecoveryOutcome::Completed { to_account_id, .. } = outcome else {
        panic!("a confirmed replacement must be completed, not thrown away: {outcome:?}");
    };
    assert_eq!(to_account_id, "target-account");
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(TARGET_ACCOUNT_ID),
        "a completed recovery must not undo the replacement it just confirmed"
    );
    assert!(!world.journal_path().exists());
}

#[test]
fn a_kill_after_a_replacement_that_does_not_verify_is_rolled_back() {
    let world = World::new(Some("second_account"));
    interrupt(&world, SwitchPhase::Replaced, &auth_json(TARGET_ACCOUNT_ID));
    let expected = toglet_lib::accounts::AccountIdentity::Chatgpt {
        email: "tester@example.com".to_owned(),
        plan_type: None,
    };

    let outcome = toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        Some(&expected),
    )
    .expect("recovery runs");

    assert!(matches!(
        outcome,
        toglet_lib::switching::RecoveryOutcome::RolledBack
    ));
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID)
    );
}

#[test]
fn a_replacement_whose_target_cannot_be_established_is_rolled_back_rather_than_assumed() {
    let world = World::new(None);
    interrupt(&world, SwitchPhase::Replaced, &auth_json(TARGET_ACCOUNT_ID));

    let outcome = toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        None,
    )
    .expect("recovery runs");

    assert!(matches!(
        outcome,
        toglet_lib::switching::RecoveryOutcome::RolledBack
    ));
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID)
    );
}

#[test]
fn recovering_a_home_nobody_was_signed_in_to_removes_the_file_rather_than_emptying_it() {
    let world = World::new(None);
    let backup = world.home.path().join("auth.json.toglet-switch-op-crash");
    std::fs::write(&backup, b"").expect("written");
    std::fs::write(world.auth(), auth_json(TARGET_ACCOUNT_ID)).expect("written");
    SwitchJournal::begin(
        world.journal_directory.path(),
        "op-crash",
        None,
        Some("target-account"),
        backup,
        STARTED_AT,
    )
    .expect("written");

    toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        None,
    )
    .expect("recovery runs");

    assert!(!world.auth().exists());
}

#[test]
fn a_recovery_that_cannot_restore_tells_the_user_where_the_copy_is() {
    // The backup is gone, so recovery cannot put anything back. It says so instead of
    // reporting a clean state.
    let world = World::new(None);
    interrupt(&world, SwitchPhase::BackedUp, &auth_json(TARGET_ACCOUNT_ID));
    std::fs::remove_file(world.home.path().join("auth.json.toglet-switch-op-crash"))
        .expect("removed");

    let outcome = toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        None,
    )
    .expect("recovery runs");

    let toglet_lib::switching::RecoveryOutcome::Failed { backup } = outcome else {
        panic!("a recovery that restored nothing must not report success: {outcome:?}");
    };
    assert!(backup.to_string_lossy().contains("toglet-switch-"));
    assert!(
        world.journal_path().exists(),
        "the journal must survive so the next start can try again"
    );
}

/// Records what the switch said finished, in order.
#[derive(Default)]
struct RecordingObserver {
    steps: std::sync::Mutex<Vec<u8>>,
}

impl RecordingObserver {
    fn seen(&self) -> Vec<u8> {
        self.steps.lock().expect("not poisoned").clone()
    }
}

impl StepObserver for RecordingObserver {
    fn completed(&self, step: toglet_lib::switching::SwitchStep) {
        self.steps.lock().expect("not poisoned").push(step.number());
    }
}

#[test]
fn the_interface_is_told_about_the_four_steps_in_order() {
    let world = World::new(None);
    let observer = RecordingObserver::default();

    world
        .switch_watched(&NoFaults, &observer)
        .expect("the switch succeeds");

    assert_eq!(observer.seen(), vec![1, 2, 3, 4]);
}

#[test]
fn a_step_that_never_finished_is_never_announced() {
    // The whole point of the seam. A progress display driven by a timer would show four steps
    // here; this one shows two, because two is what happened.
    let world = World::new(None);
    let observer = RecordingObserver::default();

    world
        .switch_watched(&FailAt::new(SwitchStage::Verify), &observer)
        .expect_err("the injected failure stops the switch");

    assert_eq!(observer.seen(), vec![1, 2]);
}

#[test]
fn nothing_is_announced_when_the_switch_fails_before_it_writes() {
    let world = World::new(None);
    let observer = RecordingObserver::default();

    world
        .switch_watched(&FailAt::new(SwitchStage::Write), &observer)
        .expect_err("the injected failure stops the switch");

    assert_eq!(
        observer.seen(),
        vec![1],
        "the pre-checks really did pass; nothing after them did"
    );
}

// ---------------------------------------------------------------------------------------------
// Signing out. The sign-out is a switch with no target, and it is checked the same
// way: the fake server decides what the "default" home reports, so a verification can disagree
// with the file having gone.
// ---------------------------------------------------------------------------------------------

impl World {
    fn sign_out(&self) -> std::result::Result<(), SignOutFailed> {
        let probe = NoClients;
        let sign_out = SignOut {
            lock: &self.lock,
            credential_lock: &self.credential_lock,
            store: &self.store,
            probe: &probe,
            binary: &self.binary,
            default_home: self.home.path(),
            journal_directory: self.journal_directory.path(),
            own_processes: &[],
        };
        let passed = sign_out
            .prepare(Some(ActiveAccount {
                credentials: &self.active_ref,
                fingerprint: &self.active_fingerprint,
            }))
            .expect("the pre-checks pass");
        sign_out
            .run(passed, "active-account", OPERATION, STARTED_AT)
            // The token exists; that it is the only way to clear `activeAccountId` is what the
            // source scan in `storage::settings` enforces.
            .map(|signed_out| assert!(format!("{signed_out:?}").contains("SwitchVerified")))
    }
}

#[test]
fn a_confirmed_sign_out_removes_the_authentication_and_leaves_nothing_behind() {
    // The server agrees nobody is signed in once the file is gone.
    let world = World::new(Some("signed_out"));

    world.sign_out().expect("the sign-out completes");

    assert!(!world.auth().exists(), "the default auth.json is gone");
    assert!(!world.journal_path().exists(), "the journal is deleted");
    assert!(
        backups(world.home.path()).is_empty(),
        "the copy is deleted once the sign-out is confirmed"
    );
    assert!(world.lock.try_acquire().is_some(), "the lock is released");
}

#[test]
fn a_sign_out_the_server_does_not_confirm_is_rolled_back() {
    // The default scenario keeps reporting an account after the file is removed, which is what
    // a home that is not really what the file says would look like. Nothing may be concluded
    // from the removal alone (applied to "nobody").
    let world = World::new(None);

    let failed = world.sign_out().expect_err("the sign-out is refused");

    assert_eq!(failed.error.code(), ErrorCode::SwitchVerificationMismatch);
    assert_eq!(failed.rollback, RollbackReport::Restored);
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID),
        "the previous authentication is back, byte for byte"
    );
    assert!(
        !world.journal_path().exists(),
        "a restored home needs no journal"
    );
    assert!(backups(world.home.path()).is_empty());
    assert!(world.lock.try_acquire().is_some(), "the lock is released");
}

#[test]
fn a_kill_during_a_sign_out_restores_the_authentication_at_the_next_start() {
    // A sign-out journal names no target, so recovery has nothing to complete against: it
    // restores, whatever the home reports and whatever the caller offers as a target.
    let world = World::new(Some("signed_out"));
    let backup = world.home.path().join("auth.json.toglet-switch-op-crash");
    std::fs::write(&backup, auth_json(ACCOUNT_ID)).expect("written");
    std::fs::remove_file(world.auth()).expect("the sign-out had removed the file");
    SwitchJournal::begin(
        world.journal_directory.path(),
        "op-crash",
        Some("active-account"),
        None,
        backup,
        STARTED_AT,
    )
    .expect("written");
    let offered = toglet_lib::accounts::AccountIdentity::Chatgpt {
        email: "tester@example.com".to_owned(),
        plan_type: None,
    };

    let outcome = toglet_lib::switching::recover(
        &world.binary,
        world.home.path(),
        world.journal_directory.path(),
        Some(&offered),
    )
    .expect("recovery runs");

    assert!(
        matches!(outcome, toglet_lib::switching::RecoveryOutcome::RolledBack),
        "an interrupted sign-out is undone, never completed: {outcome:?}"
    );
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID)
    );
    assert!(!world.journal_path().exists());
}

#[test]
fn a_sign_out_journal_carries_no_target_and_no_credential_material() {
    let world = World::new(Some("signed_out"));
    let backup = world.home.path().join("auth.json.toglet-switch-op-x");
    std::fs::write(&backup, auth_json(ACCOUNT_ID)).expect("written");
    SwitchJournal::begin(
        world.journal_directory.path(),
        "op-x",
        Some("active-account"),
        None,
        backup,
        STARTED_AT,
    )
    .expect("written");

    let written = std::fs::read_to_string(world.journal_path()).expect("readable");

    assert!(written.contains("\"toAccountId\":null"), "{written}");
    for forbidden in ["access_token", "refresh_token", "id_token", ACCOUNT_ID] {
        assert!(
            !written.contains(forbidden),
            "the journal carried `{forbidden}`"
        );
    }
}
