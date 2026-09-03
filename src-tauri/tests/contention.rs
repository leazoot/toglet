//! The cases that only appear when two things happen at once, or when the operating system
//! refuses.
//!
//! Lock contention and a file that is in use. The other integration files drive one operation
//! at a time, which is exactly why those two are here instead: a lock that is only ever taken
//! by one thread has not been shown to serialise anything.

mod support;

#[cfg(windows)]
use std::path::PathBuf;
use std::sync::Barrier;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;

use support::fake_binary;
use toglet_lib::accounts::external_change::ActiveAccount;
use toglet_lib::accounts::fingerprint;
use toglet_lib::codex_home::IsolatedHome;
use toglet_lib::credentials::{
    CredentialLock, CredentialRef, MemorySecretStore, Secret, SecretStore,
};
use toglet_lib::diagnostics::Phase;
use toglet_lib::process::{ClientPresence, ClientProbe};
use toglet_lib::switching::{Preflight, PreflightStep, SwitchLock, SwitchTarget};

const PHASE: Phase = Phase::Precheck;
const ACCOUNT_ID: &str = "8f14e45f-ceea-467a-9f3a-1c2d3e4f5a6b";
const TARGET_ACCOUNT_ID: &str = "1a2b3c4d-0000-4000-8000-abcdefabcdef";

struct NoClients;

impl ClientProbe for NoClients {
    fn running_clients(&self, _exclude: &[u32]) -> ClientPresence {
        ClientPresence::Known(Vec::new())
    }
}

fn auth_json(account_id: &str) -> Vec<u8> {
    format!(r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account_id}"}}}}"#).into_bytes()
}

#[test]
fn only_one_of_many_threads_racing_for_the_switch_lock_gets_it() {
    // Real concurrency. The single-threaded test proves the lock refuses a second caller; this
    // proves it refuses every other caller when they arrive together.
    //
    // Two barriers rather than a sleep: the first makes every thread attempt at the same
    // moment, the second stops the winner releasing until all of them have tried. Without them
    // the threads simply take turns - which is what a first attempt at this test measured.
    const RACERS: usize = 16;
    let lock = SwitchLock::new();
    let winners = AtomicU32::new(0);
    let start = Barrier::new(RACERS);
    let everyone_tried = Barrier::new(RACERS);

    std::thread::scope(|scope| {
        for _ in 0..RACERS {
            scope.spawn(|| {
                start.wait();
                let attempt = lock.try_acquire();
                if attempt.is_some() {
                    winners.fetch_add(1, Ordering::SeqCst);
                }
                everyone_tried.wait();
                drop(attempt);
            });
        }
    });

    assert_eq!(
        winners.load(Ordering::SeqCst),
        1,
        "exactly one switch may run; the rest are refused rather than queued"
    );
}

#[test]
fn the_lock_is_usable_again_once_the_thread_holding_it_finishes() {
    let lock = SwitchLock::new();

    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let guard = lock.try_acquire().expect("the lock is free");
                drop(guard);
            })
            .join()
            .expect("the holder finishes");
    });

    assert!(lock.try_acquire().is_some());
}

#[test]
fn a_switch_waits_for_a_credential_write_back_instead_of_reading_a_snapshot_being_replaced() {
    // The contract, exercised rather than asserted in a comment: the write-back finishes, and
    // whoever wants the credentials next waits for it.
    //
    // The handover is done with channels rather than sleeps so the ordering is established by
    // the threads themselves. One window is inherent to testing a mutex: the waiting thread can
    // only say "about to block", never "now blocked". The order assertion holds either way -
    // the write-back records its end *before* releasing the lock.
    let owned_lock = CredentialLock::new();
    let owned_order = std::sync::Mutex::new(Vec::new());
    // Borrowed before the `move` closures so they capture the references rather than the values.
    let lock = &owned_lock;
    let order = &owned_order;
    let (holding, wait_for_holding) = mpsc::channel();
    let (release, wait_for_release) = mpsc::channel();
    let (waiter_ready, wait_for_waiter) = mpsc::channel();

    std::thread::scope(|scope| {
        scope.spawn(move || {
            let guard = lock.acquire();
            order.lock().expect("not poisoned").push("write-back start");
            holding.send(()).expect("the main thread is listening");
            wait_for_release.recv().expect("released");
            order.lock().expect("not poisoned").push("write-back end");
            drop(guard);
        });
        wait_for_holding
            .recv()
            .expect("the write-back holds the lock");

        scope.spawn(move || {
            waiter_ready.send(()).expect("the main thread is listening");
            let _guard = lock.acquire();
            order.lock().expect("not poisoned").push("switch");
        });
        wait_for_waiter
            .recv()
            .expect("the switch is about to ask for the lock");

        release.send(()).expect("the write-back is listening");
    });

    assert_eq!(
        *owned_order.lock().expect("not poisoned"),
        vec!["write-back start", "write-back end", "switch"],
        "a switch must not read a snapshot that is being replaced"
    );
}

/// A default home signed in as `ACCOUNT_ID`, with a target account ready to switch to.
struct World {
    home: IsolatedHome,
    #[cfg(windows)]
    journal_directory: IsolatedHome,
    store: MemorySecretStore,
    lock: SwitchLock,
    credential_lock: CredentialLock,
    active_ref: CredentialRef,
    target_ref: CredentialRef,
    active_fingerprint: String,
}

impl World {
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
            .store(&target_ref, &Secret::new(auth_json(TARGET_ACCOUNT_ID)))
            .expect("stored");

        Self {
            home,
            #[cfg(windows)]
            journal_directory: IsolatedHome::create(Phase::Storage).expect("scratch directory"),
            store,
            lock: SwitchLock::new(),
            credential_lock: CredentialLock::new(),
            active_ref,
            target_ref,
            active_fingerprint: fingerprint::from_account_id(ACCOUNT_ID),
        }
    }

    #[cfg(windows)]
    fn auth(&self) -> PathBuf {
        self.home.path().join("auth.json")
    }
}

/// A target file another process holds open for reading only.
///
/// Read access is left open on purpose: the copy has to succeed so the switch gets as far as
/// the replacement, which is the step this test is about.
#[cfg(windows)]
#[test]
fn a_replacement_that_the_operating_system_refuses_reports_where_the_backup_is() {
    use std::os::windows::fs::OpenOptionsExt;

    use toglet_lib::diagnostics::ErrorCode;
    use toglet_lib::switching::{NoFaults, NoObserver, RollbackReport, Switch};
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

    let world = World::new();
    let binary = fake_binary(PHASE);
    let probe = NoClients;
    let preflight = Preflight {
        lock: &world.lock,
        credential_lock: &world.credential_lock,
        store: &world.store,
        probe: &probe,
        binary: &binary,
        default_home: world.home.path(),
        own_processes: &[],
    };
    let passed = preflight
        .run(
            Some("active-account"),
            Some(ActiveAccount {
                credentials: &world.active_ref,
                fingerprint: &world.active_fingerprint,
            }),
            SwitchTarget {
                account_id: "target-account",
                credentials: &world.target_ref,
            },
        )
        .expect("the pre-checks pass");

    // Taken after the checks, because a file held like this is exactly what "another program
    // opened it a moment ago" looks like.
    let held = std::fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ)
        .open(world.auth())
        .expect("the other process takes the file");

    let switch = Switch {
        binary: &binary,
        default_home: world.home.path(),
        journal_directory: world.journal_directory.path(),
        faults: &NoFaults,
        observer: &NoObserver,
    };
    let failed = switch
        .run(
            passed,
            Some("active-account"),
            "target-account",
            "op-held",
            "2026-09-01T00:00:00Z",
        )
        .expect_err("a replacement the operating system refused is not a switch");

    let RollbackReport::Failed { backup } = failed.rollback else {
        panic!(
            "the rollback could not run either, and must say so: {:?}",
            failed.rollback
        );
    };
    assert_eq!(
        failed.error.code(),
        ErrorCode::RollbackFailed,
        "the user has to put their own credentials back"
    );
    assert!(backup.exists(), "the copy must still be there to point at");

    drop(held);
    assert_eq!(
        std::fs::read(world.auth()).expect("readable"),
        auth_json(ACCOUNT_ID),
        "nothing was replaced, so the previous credentials are still in place"
    );
}

#[test]
fn a_failed_pre_check_names_the_step_and_a_stable_code() {
    // A failure has to be locatable. Every pre-check failure carries both which check stopped
    // it and a code that does not change between builds.
    let world = World::new();
    let binary = fake_binary(PHASE);
    let probe = NoClients;
    let preflight = Preflight {
        lock: &world.lock,
        credential_lock: &world.credential_lock,
        store: &world.store,
        probe: &probe,
        binary: &binary,
        default_home: world.home.path(),
        own_processes: &[],
    };

    let failure = preflight
        .run(
            Some("target-account"),
            Some(ActiveAccount {
                credentials: &world.active_ref,
                fingerprint: &world.active_fingerprint,
            }),
            SwitchTarget {
                account_id: "target-account",
                credentials: &world.target_ref,
            },
        )
        .expect_err("switching to the active account is refused");

    assert_eq!(failure.step, PreflightStep::Target);
    assert_eq!(failure.error.code().as_str(), "already_active");
    assert_eq!(failure.error.phase().as_str(), "precheck");
}
