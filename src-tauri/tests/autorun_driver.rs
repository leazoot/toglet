//! The driver, run on its own thread against fake ports and a clock the test moves: waits end
//! only when the clock says so, late results are dropped by the running thread, the sleep
//! assertion tracks activity, and every change lands on disk before anyone is told.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use toglet_lib::autorun::{
    Action, AutoRunPlan, Binding, Blocked, Blocker, Clock, DriverConfig, DriverHandle,
    ExecutionEnvironment, Exhausted, Fact, Interruption, Machine, Observation, Outcome,
    Participant, PlanStore, Ports, Reason, ResultKind, Resumed, Selection, State, UserEvent,
    Verdict, Verification, WATCH_INTERVAL_SECONDS, WaitReason, spawn,
};
use toglet_lib::codex_home::IsolatedHome;
use toglet_lib::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use toglet_lib::process::FakePowerAssertion;

const T0: i64 = 1_800_000_000;
const BUFFER: i64 = 90;
const SLICE: Duration = Duration::from_millis(5);
const PATIENCE: Duration = Duration::from_secs(3);

// ---- a clock the test moves -------------------------------------------------------------

#[derive(Clone)]
struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    fn at(seconds: i64) -> Self {
        Self(Arc::new(AtomicI64::new(seconds)))
    }

    fn set(&self, seconds: i64) {
        self.0.store(seconds, Ordering::SeqCst);
    }

    fn advance(&self, seconds: i64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

// ---- fake ports ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    ReadThread,
    Select,
    Verify(String),
    Switch(String),
    /// The account, and the sentence the phone sent, when there was one.
    Resume(String, Option<String>),
    StopExecuting,
    Notify(State, Option<WaitReason>),
}

#[derive(Default)]
struct Script {
    read_thread: VecDeque<Result<Fact>>,
    select: VecDeque<Selection>,
    verify: VecDeque<Result<Verification>>,
    switch: VecDeque<Result<()>>,
    resume: VecDeque<Result<Resumed>>,
    poll: VecDeque<Fact>,
    /// Every call, with the clock reading it was made at.
    calls: Vec<(i64, Call)>,
}

/// Lets a test hold a `verify_quota` call open until it says so.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
}

impl Gate {
    fn open(&self) {
        *self.open.lock().expect("gate") = true;
        self.changed.notify_all();
    }

    fn wait(&self) {
        let mut open = self.open.lock().expect("gate");
        while !*open {
            open = self.changed.wait(open).expect("gate");
        }
    }
}

#[derive(Clone)]
struct FakePorts {
    script: Arc<Mutex<Script>>,
    clock: ManualClock,
    verify_gate: Option<Arc<Gate>>,
}

impl FakePorts {
    fn new(clock: &ManualClock) -> Self {
        Self {
            script: Arc::new(Mutex::new(Script::default())),
            clock: clock.clone(),
            verify_gate: None,
        }
    }

    fn with<T>(&self, edit: impl FnOnce(&mut Script) -> T) -> T {
        edit(&mut self.script.lock().expect("script"))
    }

    fn record(&self, call: Call) {
        let now = self.clock.now();
        self.with(|script| script.calls.push((now, call)));
    }

    fn calls(&self) -> Vec<(i64, Call)> {
        self.with(|script| script.calls.clone())
    }

    fn count(&self, matches: impl Fn(&Call) -> bool) -> usize {
        self.calls()
            .iter()
            .filter(|(_, call)| matches(call))
            .count()
    }
}

impl Ports for FakePorts {
    fn read_thread(&mut self, _plan: &AutoRunPlan) -> Result<Fact> {
        self.record(Call::ReadThread);
        self.with(|script| script.read_thread.pop_front())
            .unwrap_or(Ok(Fact::TurnInProgress))
    }

    fn select(&mut self, _plan: &AutoRunPlan, _exhausted: Option<&Exhausted>) -> Selection {
        self.record(Call::Select);
        self.with(|script| script.select.pop_front())
            .unwrap_or(Selection::NeedsHuman)
    }

    fn verify_quota(&mut self, account_id: &str) -> Result<Verification> {
        self.record(Call::Verify(account_id.to_owned()));
        if let Some(gate) = &self.verify_gate {
            gate.wait();
        }
        self.with(|script| script.verify.pop_front())
            .unwrap_or_else(|| Err(network()))
    }

    fn switch(&mut self, account_id: &str) -> Result<()> {
        self.record(Call::Switch(account_id.to_owned()));
        self.with(|script| script.switch.pop_front())
            .unwrap_or(Ok(()))
    }

    fn resume(
        &mut self,
        _plan: &AutoRunPlan,
        account_id: &str,
        _after_turn: Option<&str>,
        instruction: Option<&str>,
    ) -> Result<Resumed> {
        self.record(Call::Resume(
            account_id.to_owned(),
            instruction.map(str::to_owned),
        ));
        self.with(|script| script.resume.pop_front())
            .unwrap_or_else(|| {
                Ok(Resumed::Started {
                    turn_id: "turn-resumed".to_owned(),
                })
            })
    }

    fn poll_turn(&mut self, wait: Duration) -> Option<Fact> {
        let next = self.with(|script| script.poll.pop_front());
        if next.is_none() {
            std::thread::sleep(wait);
        }
        next
    }

    fn stop_executing(&mut self) {
        self.record(Call::StopExecuting);
    }

    fn notify(&mut self, state: State, reason: Option<WaitReason>) {
        self.record(Call::Notify(state, reason));
    }
}

fn network() -> TogletError {
    TogletError::new(
        ErrorCode::NetworkUnavailable,
        Phase::ReadQuota,
        true,
        UserAction::Retry,
    )
}

/// Somebody else had the session. `session_busy` treats this as "lifts by itself", so the
/// driver retries rather than giving up.
fn client_running() -> TogletError {
    TogletError::new(
        ErrorCode::ClientRunning,
        Phase::Autorun,
        true,
        UserAction::CloseCodexClient,
    )
}

fn exhausted(turn: &str) -> Fact {
    Fact::TurnEnded {
        turn_id: turn.to_owned(),
        interruption: Interruption::Exhausted,
    }
}

fn now(account: &str) -> Selection {
    Selection::Now {
        account_id: account.to_owned(),
        reason: Reason::ListPosition,
    }
}

fn later(account: &str, at: i64) -> Selection {
    Selection::Later {
        account_id: account.to_owned(),
        available_at: at,
        blockers: vec![Blocker::FiveHourExhausted {
            resets_at: Some(at),
        }],
        reason: Reason::EarliestAvailable,
    }
}

fn available(active: &str) -> Result<Verification> {
    Ok(Verification {
        verdict: Verdict::Available,
        active_account_id: Some(active.to_owned()),
    })
}

fn blocked_until(at: i64) -> Result<Verification> {
    Ok(Verification {
        verdict: Verdict::Blocked(Blocked {
            blockers: vec![Blocker::FiveHourExhausted {
                resets_at: Some(at),
            }],
            expected_available_at: Some(at),
        }),
        active_account_id: Some("a".to_owned()),
    })
}

/// A complete plan, disabled: the driver refuses to enable anything less.
fn bound_plan() -> AutoRunPlan {
    let mut plan = AutoRunPlan::disabled("2026-09-11T00:00:00Z");
    plan.binding = Some(Binding {
        execution_environment: ExecutionEnvironment::Desktop,
        project_path: std::path::PathBuf::from("/fake/project"),
        project_label: "project".to_owned(),
        thread_id: "thread-1".to_owned(),
        thread_title: None,
        resume_instruction: "Continue.".to_owned(),
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
    plan
}

// ---- the rig --------------------------------------------------------------------------------

struct Rig {
    home: IsolatedHome,
    clock: ManualClock,
    ports: FakePorts,
    power: Arc<FakePowerAssertion>,
    changes: Receiver<AutoRunPlan>,
    handle: Option<DriverHandle>,
}

/// A fake power assertion the test can look at from outside: the driver owns a proxy that
/// forwards to the shared one.
struct SharedPower(Arc<FakePowerAssertion>);

impl toglet_lib::process::PowerAssertion for SharedPower {
    fn acquire(&self, reason: &'static str) -> Result<Box<dyn toglet_lib::process::PowerHold>> {
        self.0.acquire(reason)
    }
}

impl Rig {
    fn start(plan: AutoRunPlan) -> Self {
        let home = IsolatedHome::create(Phase::Storage).expect("scratch");
        let clock = ManualClock::at(T0);
        let ports = FakePorts::new(&clock);
        let power = Arc::new(FakePowerAssertion::default());
        let (tx, changes) = mpsc::channel();
        let store = PlanStore::new(home.path());
        store.save(&plan).expect("saved");
        let handle = spawn(DriverConfig {
            store,
            plan,
            ports: Box::new(ports.clone()),
            clock: Box::new(clock.clone()),
            power: Box::new(SharedPower(Arc::clone(&power))),
            observer: Box::new(move |plan: &AutoRunPlan| {
                // A receiver that went away is a test that finished; nothing to do.
                drop(tx.send(plan.clone()));
            }),
            active_slice: SLICE,
            idle_slice: SLICE,
        })
        .expect("the driver starts");
        Self {
            home,
            clock,
            ports,
            power,
            changes,
            handle: Some(handle),
        }
    }

    fn disabled() -> Self {
        Self::start(bound_plan())
    }

    fn handle(&self) -> &DriverHandle {
        self.handle.as_ref().expect("the driver is running")
    }

    /// Waits until the driver reports a plan in `state`, returning it.
    fn until_state(&self, state: State) -> AutoRunPlan {
        self.until(|plan| plan.state == state)
    }

    fn until(&self, accept: impl Fn(&AutoRunPlan) -> bool) -> AutoRunPlan {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.changes.recv_timeout(remaining) {
                Ok(plan) if accept(&plan) => return plan,
                Ok(_) => {}
                Err(_) => panic!("the driver did not reach the expected state in time"),
            }
        }
    }

    /// Waits until the fake ports have seen `count` calls matching `matches`.
    fn until_calls(&self, count: usize, matches: impl Fn(&Call) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while self.ports.count(&matches) < count {
            assert!(
                Instant::now() < deadline,
                "expected {count} calls, saw {:?}",
                self.ports.calls()
            );
            std::thread::sleep(SLICE);
        }
    }

    /// Gives the driver a while and asserts nothing matching `matches` was called.
    fn settle_without(&self, matches: impl Fn(&Call) -> bool) {
        std::thread::sleep(SLICE * 40);
        assert_eq!(self.ports.count(&matches), 0, "{:?}", self.ports.calls());
    }

    fn on_disk(&self) -> AutoRunPlan {
        PlanStore::new(self.home.path())
            .load("2026-09-11T00:00:00Z")
            .0
    }

    /// Enables the plan and drives it to `armed`.
    fn arm(&self) {
        self.handle().user(UserEvent::Enable).expect("sent");
        self.until_state(State::Armed);
    }

    fn shutdown(&mut self) -> Duration {
        let started = Instant::now();
        if let Some(handle) = self.handle.take() {
            handle.shutdown();
        }
        started.elapsed()
    }
}

// ---- tests ------------------------------------------------------------------------------------

// Nothing is enabled without a complete plan, whatever the command layer checked.
#[test]
fn an_incomplete_plan_cannot_be_enabled() {
    let mut rig = Rig::start(AutoRunPlan::disabled("2026-09-11T00:00:00Z"));
    rig.until_state(State::Disabled);
    rig.handle().user(UserEvent::Enable).expect("sent");
    rig.settle_without(|call| *call == Call::ReadThread);
    assert_eq!(rig.on_disk().state, State::Disabled);
    assert_eq!(rig.power.live(), 0);
    rig.shutdown();
}

// A change to the plan lands on disk and is announced like a state change.
#[test]
fn an_edit_to_the_plan_is_written_and_announced() {
    let mut rig = Rig::start(AutoRunPlan::disabled("2026-09-11T00:00:00Z"));
    rig.until_state(State::Disabled);
    rig.handle()
        .edit(Box::new(|plan| {
            plan.participants = vec![Participant {
                account_id: "z".to_owned(),
                order: 0,
            }];
            plan.max_resumes = Some(3);
        }))
        .expect("sent");
    let edited = rig.until(|plan| plan.max_resumes == Some(3));
    assert_eq!(edited.participants.len(), 1);
    assert_eq!(rig.on_disk().max_resumes, Some(3));
    rig.shutdown();
}

// The enable that follows a bind reads the last reported plan; the bind has to be in it.
#[test]
fn an_edit_has_been_written_and_announced_by_the_time_the_handle_returns() {
    let mut rig = Rig::start(AutoRunPlan::disabled("2026-09-11T00:00:00Z"));
    rig.until_state(State::Disabled);

    rig.handle()
        .edit(Box::new(|plan| {
            plan.max_resumes = Some(5);
        }))
        .expect("applied");

    assert_eq!(rig.on_disk().max_resumes, Some(5));
    rig.shutdown();
}

#[test]
fn a_fresh_plan_starts_disabled_and_is_written_before_it_is_observed() {
    let mut rig = Rig::disabled();
    let first = rig.until_state(State::Disabled);
    assert!(!first.enabled);
    assert_eq!(rig.on_disk().state, State::Disabled);
    assert_eq!(rig.power.live(), 0);
    rig.shutdown();
}

#[test]
fn enabling_arms_and_watches_the_thread_at_the_interval() {
    let mut rig = Rig::disabled();
    rig.arm();
    rig.until_calls(1, |call| *call == Call::ReadThread);
    rig.settle_without(|call| *call == Call::Select);
    assert_eq!(rig.ports.count(|call| *call == Call::ReadThread), 1);

    rig.clock.advance(WATCH_INTERVAL_SECONDS);
    rig.until_calls(2, |call| *call == Call::ReadThread);
    assert_eq!(rig.on_disk().state, State::Armed);
    rig.shutdown();
}

// The account is read again only after the expected time plus the buffer, and never
// continued while it is still blocked.
#[test]
fn a_blocked_account_is_re_read_only_after_the_expected_time_plus_the_buffer() {
    let mut rig = Rig::disabled();
    let expected = T0 + 3600;
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(later("b", expected));
        script.verify.push_back(blocked_until(expected + 7200));
    });
    rig.arm();
    let waiting = rig.until_state(State::WaitingQuota);
    assert_eq!(waiting.expected_available_at, Some(expected));
    assert_eq!(waiting.next_check_at, Some(expected + BUFFER));

    // Up to the buffer: nothing is read.
    rig.clock.set(expected + BUFFER - 1);
    rig.settle_without(|call| matches!(call, Call::Verify(_)));

    // At the buffer: one read, which still says blocked, so it waits for the next time.
    rig.clock.set(expected + BUFFER);
    rig.until_calls(1, |call| matches!(call, Call::Verify(_)));
    let waiting = rig.until(|plan| {
        plan.state == State::WaitingQuota && plan.expected_available_at == Some(expected + 7200)
    });
    assert_eq!(waiting.next_check_at, Some(expected + 7200 + BUFFER));

    let verify_at: Vec<i64> = rig
        .ports
        .calls()
        .into_iter()
        .filter(|(_, call)| matches!(call, Call::Verify(_)))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(verify_at, vec![expected + BUFFER]);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Resume(..))), 0);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Switch(_))), 0);
    rig.shutdown();
}

// A failed read backs off; the next read is no sooner than the backoff.
#[test]
fn a_failed_read_is_retried_no_sooner_than_the_backoff() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(Err(network()));
        script.verify.push_back(Err(network()));
        script.verify.push_back(available("a"));
    });
    rig.arm();
    // First read fails at T0; the plan says the retry is 30 s out.
    let waiting = rig.until(|plan| plan.state == State::Verifying && plan.next_check_at.is_some());
    assert_eq!(waiting.next_check_at, Some(T0 + 30));
    rig.settle_without(|call| matches!(call, Call::Resume(..)));
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Verify(_))), 1);

    rig.clock.advance(29);
    rig.settle_without(|call| matches!(call, Call::Resume(..)));
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Verify(_))), 1);

    // Second read fails at T0 + 30; the backoff has doubled.
    rig.clock.advance(1);
    let waiting = rig.until(|plan| plan.next_check_at == Some(T0 + 90));
    assert_eq!(waiting.state, State::Verifying);
    rig.clock.advance(59);
    std::thread::sleep(SLICE * 20);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Verify(_))), 2);
    rig.clock.advance(1);
    rig.until_calls(3, |call| matches!(call, Call::Verify(_)));
    rig.until_state(State::Running);

    let verify_at: Vec<i64> = rig
        .ports
        .calls()
        .into_iter()
        .filter(|(_, call)| matches!(call, Call::Verify(_)))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(verify_at, vec![T0, T0 + 30, T0 + 90]);
    rig.shutdown();
}

// Sleeping through the expected time is noticed on the first look, not after a countdown
// that went negative.
#[test]
fn a_wake_long_after_the_expected_time_moves_on_at_once() {
    let mut rig = Rig::disabled();
    let expected = T0 + 3600;
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(later("a", expected));
        script.verify.push_back(available("a"));
    });
    rig.arm();
    rig.until_state(State::WaitingQuota);
    rig.settle_without(|call| matches!(call, Call::Verify(_)));

    rig.clock.set(expected + 8 * 3600);
    rig.until_calls(1, |call| matches!(call, Call::Verify(_)));
    rig.until_state(State::Running);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Verify(_))), 1);
    rig.shutdown();
}

// A pause that lands while a read is in flight wins over the read.
#[test]
fn a_result_that_arrives_after_a_pause_is_dropped() {
    let gate = Arc::new(Gate::default());
    let mut rig = Rig::disabled();
    // The driver holds its own clone of the ports; the gate has to be in place before start.
    // Rebuild the rig's driver with gated ports.
    rig.shutdown();
    let mut gated = rig.ports.clone();
    gated.verify_gate = Some(Arc::clone(&gate));
    let (tx, changes) = mpsc::channel();
    let store = PlanStore::new(rig.home.path());
    let plan = store.load("2026-09-11T00:00:00Z").0;
    let power = Arc::clone(&rig.power);
    rig.handle = Some(
        spawn(DriverConfig {
            store,
            plan,
            ports: Box::new(gated),
            clock: Box::new(rig.clock.clone()),
            power: Box::new(SharedPower(power)),
            observer: Box::new(move |plan: &AutoRunPlan| drop(tx.send(plan.clone()))),
            active_slice: SLICE,
            idle_slice: SLICE,
        })
        .expect("restarted"),
    );
    rig.changes = changes;

    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
    });
    rig.arm();
    rig.until_calls(1, |call| matches!(call, Call::Verify(_)));
    rig.until_state(State::Verifying);

    rig.handle().user(UserEvent::Pause).expect("sent");
    gate.open();

    let paused = rig.until_state(State::Paused);
    assert!(paused.wait_reason.is_none());
    rig.settle_without(|call| matches!(call, Call::Resume(..) | Call::Switch(_)));
    assert_eq!(rig.on_disk().state, State::Paused);
    rig.shutdown();
}

#[test]
fn cancel_ends_the_wait_within_a_second_and_disables() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(later("a", T0 + 3600));
    });
    rig.arm();
    rig.until_state(State::WaitingQuota);

    let started = Instant::now();
    rig.handle().user(UserEvent::Cancel).expect("sent");
    let disabled = rig.until_state(State::Disabled);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(!disabled.enabled);
    assert_eq!(rig.power.live(), 0);
    rig.settle_without(|call| matches!(call, Call::Verify(_) | Call::Resume(..)));
    rig.shutdown();
}

#[test]
fn shutdown_ends_the_thread_within_a_second_while_waiting() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(later("a", T0 + 3600));
    });
    rig.arm();
    rig.until_state(State::WaitingQuota);
    assert!(rig.shutdown() < Duration::from_secs(1));
    assert_eq!(rig.power.live(), 0);
}

#[test]
fn the_sleep_assertion_is_held_exactly_while_the_plan_is_active() {
    let mut rig = Rig::disabled();
    assert_eq!(rig.power.live(), 0);
    rig.arm();
    assert_eq!(rig.power.live(), 1);
    rig.handle().user(UserEvent::Pause).expect("sent");
    rig.until_state(State::Paused);
    assert_eq!(rig.power.live(), 0);
    rig.handle().user(UserEvent::Resume).expect("sent");
    rig.until_state(State::Armed);
    assert_eq!(rig.power.live(), 1);
    assert_eq!(rig.power.acquired(), 2);
    rig.shutdown();
    assert_eq!(rig.power.live(), 0);
}

// The deadline stops the plan.
#[test]
fn the_deadline_stops_the_plan_and_tells_the_user() {
    let mut plan = bound_plan();
    plan.deadline = Some(T0 + 600);
    let mut rig = Rig::start(plan);
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(later("a", T0 + 3600));
    });
    rig.arm();
    rig.until_state(State::WaitingQuota);

    rig.clock.set(T0 + 600);
    let stopped = rig.until_state(State::Stopped);
    assert_eq!(stopped.wait_reason.as_deref(), Some("deadline"));
    rig.until_calls(1, |call| {
        *call == Call::Notify(State::Stopped, Some(WaitReason::Deadline))
    });
    assert_eq!(rig.power.live(), 0);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Verify(_))), 0);
    rig.shutdown();
}

// One continuation, then the round ends and nothing else is sent.
#[test]
fn a_round_is_one_continuation_then_the_session_is_released() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
        script.poll.push_back(Fact::TurnEnded {
            turn_id: "t2".to_owned(),
            interruption: Interruption::Completed,
        });
    });
    rig.arm();
    let completed = rig.until_state(State::RoundCompleted);
    assert_eq!(completed.resume_count, 1);
    assert_eq!(
        completed.last_result.as_ref().map(|r| r.kind),
        Some(ResultKind::Completed)
    );
    rig.until_calls(1, |call| *call == Call::StopExecuting);
    rig.until_calls(1, |call| *call == Call::Notify(State::RoundCompleted, None));
    rig.settle_without(|call| matches!(call, Call::Switch(_)));
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Resume(..))), 1);
    assert_eq!(rig.ports.count(|call| *call == Call::StopExecuting), 1);
    assert_eq!(rig.power.live(), 0);
    assert_eq!(rig.on_disk().state, State::RoundCompleted);
    rig.shutdown();
}

// A sentence typed on the phone continues this turn and nothing more: the instruction the user
// bound stays exactly as it was, because automatic continuation still needs it when quota
// returns.
#[test]
fn a_phone_sentence_is_used_for_one_turn_and_never_rewrites_the_stored_instruction() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
    });
    rig.arm();
    rig.handle()
        .send_text("go with your recommendation".to_owned())
        .expect("sent");

    rig.until_calls(1, |call| matches!(call, Call::Resume(..)));
    let calls = rig.ports.calls();
    let sent = calls
        .iter()
        .find_map(|(_, call)| match call {
            Call::Resume(_, instruction) => Some(instruction.clone()),
            _ => None,
        })
        .expect("a resume happened");
    assert_eq!(sent.as_deref(), Some("go with your recommendation"));

    // The bound instruction on disk is untouched.
    let stored = rig.on_disk();
    assert_eq!(
        stored
            .binding
            .as_ref()
            .map(|binding| binding.resume_instruction.as_str()),
        Some("Continue.")
    );
    rig.shutdown();
}

// The sentence is session content: it lives in the driver for one turn and never reaches disk.
#[test]
fn a_phone_sentence_never_reaches_the_plan_file() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
    });
    rig.arm();
    let secret = "please delete the staging database";
    rig.handle().send_text(secret.to_owned()).expect("sent");
    rig.until_calls(1, |call| matches!(call, Call::Resume(..)));

    // Every file the driver wrote, not just the one we know the name of.
    let entries = std::fs::read_dir(rig.home.path()).expect("the directory is readable");
    let mut looked_at = 0;
    for entry in entries.flatten() {
        if let Ok(written) = std::fs::read_to_string(entry.path()) {
            looked_at += 1;
            assert!(
                !written.contains(secret),
                "{:?} leaked the sentence:\n{written}",
                entry.path()
            );
        }
    }
    assert!(looked_at > 0, "no files were checked");
    rig.shutdown();
}

// A takeover that was busy retries, and the sentence has to still be there when it does. This
// is why the driver clears it only once a turn really starts, not when one is attempted.
#[test]
fn a_sentence_survives_a_takeover_that_was_busy() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Err(client_running()));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
    });
    rig.arm();
    let sentence = "go with your recommendation";
    rig.handle().send_text(sentence.to_owned()).expect("sent");

    rig.until_calls(1, |call| matches!(call, Call::Resume(..)));
    rig.clock.advance(3 * 60 * 60);
    rig.until_calls(2, |call| matches!(call, Call::Resume(..)));

    let calls = rig.ports.calls();
    let attempts: Vec<Option<String>> = calls
        .iter()
        .filter_map(|(_, call)| match call {
            Call::Resume(_, instruction) => Some(instruction.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(attempts.len(), 2);
    for (index, attempt) in attempts.iter().enumerate() {
        assert_eq!(
            attempt.as_deref(),
            Some(sentence),
            "attempt {index} lost the sentence"
        );
    }
    rig.shutdown();
}

// The case remote control exists for, end to end through the driver: the session stopped to ask
// something and the answer comes from the phone.
//
// Before this, a sentence in `needs_human` took the "continue" path, which re-arms the machine;
// arming re-reads the bound thread, finds it still waiting on a person, and goes straight back
// to `needs_human`. The sentence never reached a continuation at all, and lingered in the driver
// where a later automatic resume would have carried words written for a different moment.
#[test]
fn a_sentence_continues_a_session_that_is_waiting_on_a_person() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        // The automatic continuation runs into a question, so the plan waits for a person.
        script.resume.push_back(Ok(Resumed::WaitingOnHuman));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
    });
    rig.arm();
    rig.until_state(State::NeedsHuman);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Resume(..))), 1);

    rig.handle()
        .send_text("go with your recommendation".to_owned())
        .expect("sent");

    rig.until_calls(2, |call| matches!(call, Call::Resume(..)));
    let calls = rig.ports.calls();
    let sentences: Vec<Option<String>> = calls
        .iter()
        .filter_map(|(_, call)| match call {
            Call::Resume(_, instruction) => Some(instruction.clone()),
            _ => None,
        })
        .collect();
    // The automatic continuation carried the stored instruction; the phone's carried its own.
    assert_eq!(sentences[0], None);
    assert_eq!(sentences[1].as_deref(), Some("go with your recommendation"));
    rig.shutdown();
}

// A sentence nothing can carry is dropped rather than kept: a turn is already running, so there
// is no continuation to attach it to, and it must not ride the next one.
#[test]
fn a_sentence_sent_while_a_turn_runs_is_not_kept_for_the_next_one() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
        // Nothing scripted for the poll yet: an empty script answers "still running", which
        // holds t2 open until the test says otherwise.
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t3".to_owned(),
        }));
    });
    rig.arm();
    rig.until_calls(1, |call| matches!(call, Call::Resume(..)));

    // The turn is running: `send` is not offered on the phone in this state, and if one arrives
    // anyway the driver must not stash it.
    rig.handle()
        .send_text("this must never be sent".to_owned())
        .expect("sent");
    // The loop drains its inbox every slice; this is long enough for the sentence to have been
    // handled while the turn is still running. Scripting the exhaustion up front let a slow
    // machine end t2 first, and a sentence arriving between turns is meant for the next one -
    // which is a different test.
    std::thread::sleep(SLICE * 40);
    rig.ports
        .with(|script| script.poll.push_back(exhausted("t2")));

    rig.until_calls(2, |call| matches!(call, Call::Resume(..)));
    let calls = rig.ports.calls();
    for (_, call) in &calls {
        if let Call::Resume(_, instruction) = call {
            assert_eq!(
                instruction.as_deref(),
                None,
                "a sentence rode a continuation it was not written for"
            );
        }
    }
    rig.shutdown();
}

// The sentence is session content. The log sink is process-wide, so this test binary cannot
// read what was logged; what it can check is that the driver never hands the text to the logger
// in the first place - only how long it was.
#[test]
fn the_driver_logs_how_long_a_sentence_was_and_never_the_sentence() {
    let source = include_str!("../src/autorun/driver.rs");
    let arm = source
        .split("Command::SendText(text) =>")
        .nth(1)
        .expect("the driver handles a sentence")
        .split("Command::Observed")
        .next()
        .expect("the arm ends");

    assert!(
        arm.contains("text.chars().count()"),
        "the driver should log a length, not a sentence"
    );
    assert!(
        !arm.contains("with_detail(&text)") && !arm.contains("with_detail(text)"),
        "the sentence must never be handed to the logger: {arm}"
    );
}

// While the executing account's turn is in flight nobody else is looked at, even when
// another account would long since have recovered.
#[test]
fn while_a_turn_runs_no_other_account_is_selected_or_switched_to() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("a"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
        // What the scheduler would find if it looked again: another account, ready now.
        script.select.push_back(now("b"));
        script.verify.push_back(available("b"));
    });
    rig.arm();
    let running = rig.until_state(State::Running);
    assert_eq!(running.executing_account_id.as_deref(), Some("a"));
    let looked = rig.ports.count(|call| *call == Call::Select);

    rig.clock.advance(3 * 60 * 60);
    std::thread::sleep(SLICE * 40);

    assert_eq!(rig.ports.count(|call| *call == Call::Select), looked);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Switch(_))), 0);
    assert_eq!(rig.ports.count(|call| matches!(call, Call::Resume(..))), 1);
    assert_eq!(rig.on_disk().state, State::Running);
    rig.shutdown();
}

#[test]
fn a_switch_happens_before_a_resume_on_another_account() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(now("b"));
        script.verify.push_back(available("a"));
        script.resume.push_back(Ok(Resumed::Started {
            turn_id: "t2".to_owned(),
        }));
    });
    rig.arm();
    let running = rig.until_state(State::Running);
    assert_eq!(running.executing_account_id.as_deref(), Some("b"));
    let order: Vec<Call> = rig
        .ports
        .calls()
        .into_iter()
        .map(|(_, call)| call)
        .filter(|call| matches!(call, Call::Switch(_) | Call::Resume(..)))
        .collect();
    assert_eq!(
        order,
        vec![
            Call::Switch("b".to_owned()),
            Call::Resume("b".to_owned(), None)
        ]
    );
    rig.shutdown();
}

// A manual switch observed while waiting pauses the plan.
#[test]
fn a_manual_switch_observed_while_waiting_pauses() {
    let mut rig = Rig::disabled();
    rig.ports.with(|script| {
        script.read_thread.push_back(Ok(exhausted("t1")));
        script.select.push_back(later("a", T0 + 3600));
    });
    rig.arm();
    rig.until_state(State::WaitingQuota);
    rig.handle()
        .observe(Observation::ManualSwitch)
        .expect("sent");
    let paused = rig.until_state(State::Paused);
    assert_eq!(paused.wait_reason.as_deref(), Some("manual_switch"));
    rig.shutdown();
}

// A plan that was mid-cycle comes back armed and re-reads the bound thread before it acts,
// rather than selecting on the state the restart destroyed.
#[test]
fn a_plan_saved_mid_cycle_comes_back_armed_after_a_restart() {
    let mut machine = Machine::new(Some(8));
    machine.apply_user(UserEvent::Enable);
    let generation = machine.generation();
    assert!(matches!(
        machine.apply_fact(generation, exhausted("t1")),
        Outcome::Applied(Action::Select { .. })
    ));
    let mut plan = bound_plan();
    plan.record(&machine, "2026-09-11T00:00:00Z");
    assert_eq!(plan.state, State::Selecting);

    let mut rig = Rig::start(plan);
    let restored = rig.until_state(State::Armed);
    assert_eq!(restored.wait_reason, None);
    assert!(restored.enabled);
    assert_eq!(rig.on_disk().state, State::Armed);
    // The thread is re-read; nothing is chosen on what the restart left behind.
    rig.until_calls(1, |call| *call == Call::ReadThread);
    rig.settle_without(|call| *call == Call::Select);
    // Armed is active, so the machine keeps the display awake again from the restart on.
    assert_eq!(rig.power.live(), 1);
    rig.shutdown();
}
