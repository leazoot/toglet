//! Automatic continuation end to end: the driver, the real ports, the real switch and the
//! fake app server. No account, no network.

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use support::{fake_binary, scenario_home};
use toglet_lib::accounts::{AccountStatus, fingerprint};
use toglet_lib::app_server::CodexBinary;
use toglet_lib::autorun::{
    ActiveAccountRecord, AppPorts, AutoRunPlan, Binding, Clock, DriverConfig, DriverHandle,
    ExecutionEnvironment, Participant, ParticipantRecord, PlanStore, ResultKind, Services, State,
    UserEvent, spawn,
};
use toglet_lib::codex_home::IsolatedHome;
use toglet_lib::credentials::{
    CredentialLock, CredentialRef, MemorySecretStore, Secret, SecretStore,
};
use toglet_lib::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use toglet_lib::process::{
    ClientKind, ClientPresence, ClientProbe, ClientRestart, FakePowerAssertion, QuitOutcome,
    RunningClient,
};
use toglet_lib::storage::SwitchVerified;
use toglet_lib::switching::{Faults, NoFaults, SwitchLock, SwitchStage};

const THREAD_ID: &str = "01a09301-0000-7000-8000-000000000001";
const ACCOUNT_A: &str = "acct-a";
const ACCOUNT_B: &str = "acct-b";
const T0: i64 = 1_800_000_000;
const SLICE: Duration = Duration::from_millis(5);
const PATIENCE: Duration = Duration::from_secs(60);

fn auth_json(account_id: &str) -> Vec<u8> {
    format!(r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account_id}"}}}}"#).into_bytes()
}

// ---- the application, faked ------------------------------------------------------------------

struct World {
    /// The "default" Codex home, running the fake server's scenario.
    home: IsolatedHome,
    journal: IsolatedHome,
    store: MemorySecretStore,
    switch_lock: SwitchLock,
    credential_lock: CredentialLock,
    active: Mutex<Option<String>>,
    recorded: Mutex<Vec<String>>,
}

impl World {
    fn new(scenario: &str) -> Arc<Self> {
        let home = scenario_home(scenario, Phase::Autorun);
        std::fs::write(home.path().join("auth.json"), auth_json(ACCOUNT_A)).expect("written");
        let store = MemorySecretStore::new();
        for (name, id) in [("a", ACCOUNT_A), ("b", ACCOUNT_B)] {
            store
                .store(
                    &CredentialRef::new(name).expect("valid"),
                    &Secret::new(auth_json(id)),
                )
                .expect("stored");
        }
        Arc::new(Self {
            home,
            journal: IsolatedHome::create(Phase::Storage).expect("scratch"),
            store,
            switch_lock: SwitchLock::new(),
            credential_lock: CredentialLock::new(),
            active: Mutex::new(Some("a".to_owned())),
            recorded: Mutex::new(Vec::new()),
        })
    }

    fn set_scenario(&self, scenario: &str) {
        std::fs::write(self.home.path().join("scenario"), scenario).expect("written");
    }

    fn auth(&self) -> Vec<u8> {
        std::fs::read(self.home.path().join("auth.json")).expect("readable")
    }
}

struct Shared(Arc<World>);

impl Services for Shared {
    fn secrets(&self) -> &dyn SecretStore {
        &self.0.store
    }
    fn switch_lock(&self) -> &SwitchLock {
        &self.0.switch_lock
    }
    fn credential_lock(&self) -> &CredentialLock {
        &self.0.credential_lock
    }
    fn journal_directory(&self) -> &Path {
        self.0.journal.path()
    }
    fn default_home(&self) -> Result<PathBuf> {
        Ok(self.0.home.path().to_path_buf())
    }
    fn binary(&self) -> Result<CodexBinary> {
        Ok(fake_binary(Phase::Autorun))
    }
    fn participant(&self, account_id: &str) -> Option<ParticipantRecord> {
        let (credential_ref, fingerprint, status) = match account_id {
            "a" => (
                "a",
                fingerprint::from_account_id(ACCOUNT_A),
                AccountStatus::Active,
            ),
            "b" => (
                "b",
                fingerprint::from_account_id(ACCOUNT_B),
                AccountStatus::Ready,
            ),
            _ => return None,
        };
        Some(ParticipantRecord {
            credential_ref: credential_ref.to_owned(),
            fingerprint,
            status,
            created_at: "2026-09-01T00:00:00Z".to_owned(),
        })
    }
    fn active(&self) -> Option<ActiveAccountRecord> {
        let id = self.0.active.lock().expect("lock").clone()?;
        let record = self.participant(&id)?;
        Some(ActiveAccountRecord {
            account_id: id,
            credential_ref: record.credential_ref,
            fingerprint: record.fingerprint,
        })
    }
    fn record_active(&self, account_id: &str, _verified: &SwitchVerified) -> Result<()> {
        *self.0.active.lock().expect("lock") = Some(account_id.to_owned());
        self.0
            .recorded
            .lock()
            .expect("lock")
            .push(account_id.to_owned());
        Ok(())
    }
    fn reopen_after_switch(&self) -> bool {
        true
    }
}

struct Probe(ClientPresence);

impl ClientProbe for Probe {
    fn running_clients(&self, _exclude: &[u32]) -> ClientPresence {
        self.0.clone()
    }
}

struct NoRestart;

impl ClientRestart for NoRestart {
    fn request_quit(&self, _pid: u32, _timeout: Duration) -> QuitOutcome {
        QuitOutcome::NotFound
    }
    fn launch(&self, _executable: &Path) -> Result<()> {
        Ok(())
    }
}

/// Fails once, before `stage`.
struct FailAt {
    stage: SwitchStage,
    fired: Mutex<bool>,
}

impl Faults for FailAt {
    fn before(&self, stage: SwitchStage) -> Result<()> {
        let mut fired = self.fired.lock().expect("lock");
        if stage != self.stage || *fired {
            return Ok(());
        }
        *fired = true;
        Err(
            TogletError::new(ErrorCode::Internal, Phase::Verify, true, UserAction::Retry)
                .with_detail("injected failure"),
        )
    }
}

#[derive(Clone)]
struct ManualClock(Arc<AtomicI64>);

impl Clock for ManualClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

// ---- the rig ----------------------------------------------------------------------------------

struct Rig {
    changes: Receiver<AutoRunPlan>,
    handle: Option<DriverHandle>,
}

impl Rig {
    fn start(world: Arc<World>, probe: ClientPresence, faults: Box<dyn Faults + Send>) -> Self {
        let mut plan = AutoRunPlan::disabled("2026-09-11T00:00:00Z");
        plan.binding = Some(Binding {
            execution_environment: ExecutionEnvironment::Desktop,
            project_path: PathBuf::from("/fake/project-a"),
            project_label: "project-a".to_owned(),
            thread_id: THREAD_ID.to_owned(),
            thread_title: Some("Fake session".to_owned()),
            resume_instruction: "Continue the unfinished work.".to_owned(),
            bound_at: "2026-09-11T00:00:00Z".to_owned(),
        });
        plan.participants = vec![
            Participant {
                account_id: "a".to_owned(),
                order: 0,
            },
            Participant {
                account_id: "b".to_owned(),
                order: 1,
            },
        ];
        let store = PlanStore::new(world.journal.path());
        store.save(&plan).expect("saved");

        let clock = ManualClock(Arc::new(AtomicI64::new(T0)));
        let ports = AppPorts::new(
            Box::new(Shared(Arc::clone(&world))),
            Box::new(Probe(probe)),
            Box::new(NoRestart),
            faults,
            Box::new(clock.clone()),
            // The agent excerpt goes to `remote`, which this rig does not exercise.
            Arc::new(std::sync::Mutex::new(None)),
        );
        let (tx, changes) = mpsc::channel();
        let handle = spawn(DriverConfig {
            store,
            plan,
            ports: Box::new(ports),
            clock: Box::new(clock),
            power: Box::new(FakePowerAssertion::default()),
            observer: Box::new(move |plan: &AutoRunPlan| drop(tx.send(plan.clone()))),
            active_slice: SLICE,
            idle_slice: SLICE,
        })
        .expect("the driver starts");
        Self {
            changes,
            handle: Some(handle),
        }
    }

    fn until_state(&self, state: State) -> AutoRunPlan {
        self.until(&format!("{state:?}"), |plan| plan.state == state)
    }

    fn until(&self, what: &str, accept: impl Fn(&AutoRunPlan) -> bool) -> AutoRunPlan {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.changes.recv_timeout(remaining) {
                Ok(plan) if accept(&plan) => return plan,
                Ok(_) => {}
                Err(_) => panic!("the driver did not reach {what} in time"),
            }
        }
    }

    fn enable(&self) {
        self.handle
            .as_ref()
            .expect("running")
            .user(UserEvent::Enable)
            .expect("sent");
    }

    fn shutdown(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.shutdown();
        }
    }
}

// ---- tests --------------------------------------------------------------------------------------

// A is exhausted, B is available: verified switch to B, one turn on B, and the round ends
// when that turn completes.
#[test]
fn an_exhausted_session_is_switched_to_the_next_account_and_continued_once() {
    let world = World::new("usage_limit_turn");
    let mut rig = Rig::start(
        Arc::clone(&world),
        ClientPresence::Known(Vec::new()),
        Box::new(NoFaults),
    );
    rig.enable();

    rig.until_state(State::Selecting);
    let switching = rig.until_state(State::Switching);
    assert_eq!(switching.executing_account_id, None);
    // The continuation turn will complete normally: the executor session, opened after the
    // switch, runs this scenario.
    world.set_scenario("turn_completed");

    let resuming = rig.until_state(State::Resuming);
    assert_eq!(resuming.executing_account_id.as_deref(), Some("b"));
    assert_eq!(world.auth(), auth_json(ACCOUNT_B), "the switch is on disk");
    assert_eq!(*world.recorded.lock().expect("lock"), vec!["b".to_owned()]);

    rig.until_state(State::Running);
    let completed = rig.until_state(State::RoundCompleted);
    assert_eq!(completed.resume_count, 1);
    assert_eq!(completed.executing_account_id.as_deref(), Some("b"));
    assert_eq!(
        completed.last_result.as_ref().map(|r| r.kind),
        Some(ResultKind::Completed)
    );
    assert_eq!(completed.dedup.last_resumed_turn_id.as_deref(), Some("t2"));
    rig.shutdown();
    assert_eq!(world.auth(), auth_json(ACCOUNT_B));
}

// The switch's verification fails: rolled back, and a person is asked, with the code.
#[test]
fn a_switch_that_fails_verification_is_rolled_back_and_stops_for_a_person() {
    let world = World::new("usage_limit_turn");
    let mut rig = Rig::start(
        Arc::clone(&world),
        ClientPresence::Known(Vec::new()),
        Box::new(FailAt {
            stage: SwitchStage::Verify,
            fired: Mutex::new(false),
        }),
    );
    rig.enable();

    rig.until_state(State::Switching);
    let stopped = rig.until_state(State::NeedsHuman);
    assert_eq!(stopped.wait_reason.as_deref(), Some("internal"));
    assert_eq!(stopped.resume_count, 0);
    assert_eq!(stopped.executing_account_id, None);
    assert_eq!(
        stopped.last_result.as_ref().and_then(|r| r.code.as_deref()),
        Some("internal")
    );
    rig.shutdown();

    assert_eq!(
        world.auth(),
        auth_json(ACCOUNT_A),
        "the previous credentials are back"
    );
    assert!(world.recorded.lock().expect("lock").is_empty());
    assert_eq!(world.active.lock().expect("lock").as_deref(), Some("a"));
    let leftovers: Vec<_> = std::fs::read_dir(world.home.path())
        .expect("readable")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.contains("backup") || name.contains("toglet-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

// A CLI session running anywhere stops the switch before any credential is touched: the
// plan waits in `switching` with the code and retries, and the default auth file stays untouched.
#[test]
fn a_cli_session_stops_the_switch_before_anything_is_touched() {
    let world = World::new("usage_limit_turn");
    let mut rig = Rig::start(
        Arc::clone(&world),
        ClientPresence::Known(vec![RunningClient {
            pid: 4242,
            kind: ClientKind::Cli,
            executable: PathBuf::from("/usr/local/bin/codex"),
        }]),
        Box::new(NoFaults),
    );
    rig.enable();

    rig.until_state(State::Switching);
    let refused = rig.until("a switch refused by a running CLI", |plan| {
        plan.state == State::Switching && plan.wait_reason.as_deref() == Some("client_running")
    });
    assert!(refused.enabled);
    rig.shutdown();
    assert_eq!(world.auth(), auth_json(ACCOUNT_A));
    assert!(world.recorded.lock().expect("lock").is_empty());
}
