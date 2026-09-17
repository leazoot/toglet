//! The one background task of automatic continuation.
//!
//! Turns each machine [`Action`] into a [`Ports`] call and reports the result as a
//! generation-stamped [`Fact`]. Commands arrive through a channel and are applied between steps
//! on the same thread; waits are absolute instants on an injected [`Clock`], checked in slices.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::availability::{Selection, Verdict};
use super::machine::{Action, Exhausted, Fact, Machine, Outcome, State, UserEvent, WaitReason};
use super::plan::{AutoRunPlan, PlanStore};
use crate::diagnostics::{
    ErrorCode, Level, LogRecord, Phase, Result, TogletError, UserAction, log,
};
use crate::process::{PowerAssertion, PowerHold};

/// How often the bound thread is read while armed. Each read starts an app server, hence a
/// minute rather than a second.
pub const WATCH_INTERVAL_SECONDS: i64 = 60;

/// How long the driver waits between looks at the clock and the command channel while
/// something is scheduled. Also the bound on how late a cancel is noticed.
pub const ACTIVE_SLICE: Duration = Duration::from_secs(1);

/// The slice while nothing is scheduled and the driver only waits for the user or a deadline.
pub const IDLE_SLICE: Duration = Duration::from_secs(30);

/// Shown by the platform next to the sleep assertion.
const ASSERTION_REASON: &str = "Toglet is waiting to continue a Codex session";

/// Unix seconds, injected so the driver can be tested at any speed and any date.
pub trait Clock: Send + 'static {
    fn now(&self) -> i64;
}

/// The wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
            })
    }
}

/// A fresh reading of one account, for `verifying`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    pub verdict: Verdict,
    /// The account Codex is signed in as right now.
    pub active_account_id: Option<String>,
}

/// How a resume attempt ended, when it did not fail outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resumed {
    /// The continuation turn was started.
    Started { turn_id: String },
    /// Somebody else's turn is running on the thread; nothing was started. The executor
    /// session stays open to hear how it ends.
    TurnInProgress,
    /// The thread is waiting on an approval or an answer. Nothing was started.
    WaitingOnHuman,
    /// The thread's last turn is not the one the plan was waiting on: somebody used the
    /// session meanwhile. Nothing was started.
    ThreadChanged,
}

/// What the driver asks the rest of the application to do. Every method blocks the driver's
/// thread.
pub trait Ports: Send + 'static {
    /// The bound thread's last turn, as a fact: ended, in progress, or waiting on a person.
    /// An error is a read that failed, not a turn that failed.
    fn read_thread(&mut self, plan: &AutoRunPlan) -> Result<Fact>;

    /// Reads every participant, judges them and chooses. Cannot fail: an unreadable account is a
    /// `quota_unknown` blocker. `exhausted` is whose turn ran out, when known.
    fn select(&mut self, plan: &AutoRunPlan, exhausted: Option<&Exhausted>) -> Selection;

    /// Reads this account's quota fresh and says who is signed in.
    fn verify_quota(&mut self, account_id: &str) -> Result<Verification>;

    /// Switches to this account through `switching`. `Ok` means verified.
    fn switch(&mut self, account_id: &str) -> Result<()>;

    /// Takes over the bound session on this account and starts the one continuation turn.
    /// `after_turn` is the exhausted turn the plan is continuing from; a thread whose last
    /// turn is another one has been used by somebody else meanwhile.
    ///
    /// `instruction` overrides the plan's stored one for this turn only - what the user typed on
    /// their phone. `None` means the stored instruction, which is what automatic continuation
    /// uses. The stored one is never rewritten.
    fn resume(
        &mut self,
        plan: &AutoRunPlan,
        account_id: &str,
        after_turn: Option<&str>,
        instruction: Option<&str>,
    ) -> Result<Resumed>;

    /// Waits up to `wait` for the running turn to report something. `None` is "nothing yet".
    /// A lost session is reported as a fact by the executor, not as an error here.
    fn poll_turn(&mut self, wait: Duration) -> Option<Fact>;

    /// The plan is no longer executing: close the executor session and give the desktop app
    /// back. Called once per executing stretch.
    fn stop_executing(&mut self);

    /// Tell the user: `needs_human`, `stopped`, or a completed round.
    fn notify(&mut self, state: State, reason: Option<WaitReason>);
}

/// Told every time the plan on disk changed. The event to the frontend is built from this by
/// the command layer, which strips what must not leave the process.
pub trait Observer: Send + 'static {
    fn changed(&mut self, plan: &AutoRunPlan);
}

impl<F: FnMut(&AutoRunPlan) + Send + 'static> Observer for F {
    fn changed(&mut self, plan: &AutoRunPlan) {
        self(plan);
    }
}

/// Something the rest of the application saw and the plan has to react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// The user switched accounts by hand.
    ManualSwitch,
    /// The user opened the desktop app while the plan was waiting.
    DesktopReopened,
}

/// A change to the plan's binding, participants or limits, applied on the driver's thread
/// because that is where the plan lives.
pub type PlanEdit = Box<dyn FnOnce(&mut AutoRunPlan) + Send>;

enum Command {
    User(UserEvent),
    /// A sentence for the bound session, typed on the phone. Carried here rather than on
    /// [`UserEvent`], which stays `Copy` and argument-free.
    SendText(String),
    Observed(Observation),
    /// The change, and where to say it has been applied, written and announced.
    Edit(PlanEdit, SyncSender<()>),
    Shutdown,
}

/// How long `DriverHandle::edit` waits for the driver to apply a change. The driver applies
/// commands between slices of work, and a slice is bounded by the app-server timeouts; a wait
/// past this says the driver is wedged, which is worth an error rather than a hang.
const EDIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Everything the driver is built from.
pub struct DriverConfig {
    pub store: PlanStore,
    pub plan: AutoRunPlan,
    pub ports: Box<dyn Ports>,
    pub clock: Box<dyn Clock>,
    pub power: Box<dyn PowerAssertion>,
    pub observer: Box<dyn Observer>,
    /// The wait slice while something is scheduled; [`ACTIVE_SLICE`] in production, shorter
    /// in tests.
    pub active_slice: Duration,
    pub idle_slice: Duration,
}

/// The handle the application keeps. Dropping it shuts the driver down.
pub struct DriverHandle {
    commands: Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

impl DriverHandle {
    pub fn user(&self, event: UserEvent) -> Result<()> {
        self.send(Command::User(event))
    }

    pub fn observe(&self, observation: Observation) -> Result<()> {
        self.send(Command::Observed(observation))
    }

    /// Continues the bound session with `text` instead of the stored instruction, once.
    pub fn send_text(&self, text: String) -> Result<()> {
        self.send(Command::SendText(text))
    }

    /// Changes the plan: applied in order, written to disk and reported to the observer; the
    /// caller validated it. Returns once reported, so a following read of the last reported plan
    /// already holds the change.
    pub fn edit(&self, edit: PlanEdit) -> Result<()> {
        let (done, applied) = mpsc::sync_channel(1);
        self.send(Command::Edit(edit, done))?;
        applied.recv_timeout(EDIT_TIMEOUT).map_err(|_| {
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail("the automatic continuation driver did not apply the change")
        })
    }

    /// Asks the driver to stop and waits for it. Returns once the thread has ended.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn send(&self, command: Command) -> Result<()> {
        self.commands.send(command).map_err(|_| {
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail("the automatic continuation driver has stopped")
        })
    }

    fn stop(&mut self) {
        // A send that fails means the thread already ended; joining it is still right.
        drop(self.commands.send(Command::Shutdown));
        if let Some(thread) = self.thread.take() {
            // A panic on the driver thread has already been reported by the panic hook;
            // there is nothing more to do with it here.
            drop(thread.join());
        }
    }
}

impl Drop for DriverHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts the driver on its own thread; the restored plan is written and observed before the
/// loop starts.
pub fn spawn(config: DriverConfig) -> Result<DriverHandle> {
    let (commands, inbox) = mpsc::channel();
    let driver = Driver::new(config, inbox);
    let thread = thread::Builder::new()
        .name("toglet-autorun".to_owned())
        .spawn(move || driver.run())
        .map_err(|error| {
            TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
                .with_detail(&error.to_string())
        })?;
    Ok(DriverHandle {
        commands,
        thread: Some(thread),
    })
}

enum Flow {
    /// Keep going.
    Continue,
    /// A command changed the machine; the current step is abandoned.
    Interrupted,
    Shutdown,
}

struct Driver {
    machine: Machine,
    plan: AutoRunPlan,
    store: PlanStore,
    ports: Box<dyn Ports>,
    clock: Box<dyn Clock>,
    power: Box<dyn PowerAssertion>,
    observer: Box<dyn Observer>,
    inbox: Receiver<Command>,
    active_slice: Duration,
    idle_slice: Duration,
    action: Action,
    hold: Option<Box<dyn PowerHold>>,
    executing: bool,
    /// A phone sentence waiting to be used for the next continuation turn, if any.
    ///
    /// Held here rather than in the plan: it is session content and must not be persisted, and
    /// it overrides the stored instruction for one turn only. Cleared when a turn actually
    /// starts - not when one is attempted, or a takeover that retries would lose it.
    pending_text: Option<String>,
}

impl Driver {
    fn new(config: DriverConfig, inbox: Receiver<Command>) -> Self {
        let machine = config.plan.restore_machine();
        Self {
            action: initial_action(machine.state()),
            machine,
            plan: config.plan,
            store: config.store,
            ports: config.ports,
            clock: config.clock,
            power: config.power,
            observer: config.observer,
            inbox,
            active_slice: config.active_slice,
            idle_slice: config.idle_slice,
            hold: None,
            executing: false,
            pending_text: None,
        }
    }

    fn run(mut self) {
        // The restore itself is a change worth seeing, and a plan that came back armed is
        // active from this moment, so the sleep assertion is taken here rather than at the
        // first action.
        self.after_change();
        loop {
            if let Flow::Shutdown = self.drain(Duration::ZERO) {
                break;
            }
            let flow = match self.action.clone() {
                Action::Wait if self.machine.state() == State::Running => self.run_turn(),
                Action::Wait | Action::Notify => self.idle(),
                Action::WatchThread => self.watch_thread(),
                Action::Select { exhausted } => {
                    let selection = self.ports.select(&self.plan, exhausted.as_ref());
                    self.apply(Fact::Chosen(selection))
                }
                Action::VerifyQuota { account_id } => {
                    let fact = match self.ports.verify_quota(&account_id) {
                        Ok(verification) => Fact::Verified {
                            verdict: verification.verdict,
                            active_account_id: verification.active_account_id,
                        },
                        Err(error) => {
                            self.log_failure("autorun_quota_read_failed", &error);
                            Fact::QueryFailed
                        }
                    };
                    self.apply(fact)
                }
                Action::WaitUntil { at } => self.wait_until(at, Fact::WaitElapsed),
                // Made absolute by `adopt` before it can get here; kept for completeness so a
                // future path that skips `adopt` still waits the right length.
                Action::WaitFor { delay } => {
                    let at = self.clock.now().saturating_add(as_seconds(delay));
                    self.wait_until(at, Fact::WaitElapsed)
                }
                Action::ReadThread => {
                    let fact = self.read_thread();
                    self.apply(fact)
                }
                Action::Switch { account_id } => {
                    let fact = match self.ports.switch(&account_id) {
                        Ok(()) => Fact::Switched { account_id },
                        Err(error) => {
                            self.log_failure("autorun_switch_failed", &error);
                            Fact::SwitchFailed { code: error.code() }
                        }
                    };
                    self.apply(fact)
                }
                Action::Resume { account_id } => {
                    let after_turn = self.machine.dedup().last_observed_turn_id.clone();
                    // Borrowed, not taken: a takeover that is busy retries, and the sentence has
                    // to survive until a turn really starts.
                    let instruction = self.pending_text.clone();
                    let fact = match self.ports.resume(
                        &self.plan,
                        &account_id,
                        after_turn.as_deref(),
                        instruction.as_deref(),
                    ) {
                        Ok(Resumed::Started { turn_id }) => {
                            self.pending_text = None;
                            Fact::Resumed { turn_id }
                        }
                        Ok(Resumed::TurnInProgress) => Fact::TurnInProgress,
                        Ok(Resumed::WaitingOnHuman) => Fact::WaitingOnHuman,
                        Ok(Resumed::ThreadChanged) => Fact::DesktopReopened,
                        Err(error) => {
                            self.log_failure("autorun_resume_failed", &error);
                            Fact::ResumeFailed { code: error.code() }
                        }
                    };
                    self.apply(fact)
                }
            };
            if let Flow::Shutdown = flow {
                break;
            }
        }
        // Leaving with a session open would leave the desktop app closed.
        if self.executing {
            self.ports.stop_executing();
        }
        self.hold = None;
    }

    /// Nothing scheduled: wait for the user, or for the deadline.
    fn idle(&mut self) -> Flow {
        if self.action == Action::Notify {
            self.ports
                .notify(self.machine.state(), self.machine.wait_reason());
            // Told once. The state does not change, so the action is replaced by hand.
            self.action = Action::Wait;
        }
        self.drain(self.idle_slice)
    }

    /// Armed: the desktop app is doing the work; look at the thread's last turn every
    /// [`WATCH_INTERVAL_SECONDS`].
    fn watch_thread(&mut self) -> Flow {
        let fact = self.read_thread();
        match self.apply(fact) {
            Flow::Continue => {}
            other => return other,
        }
        if self.machine.state() != State::Armed {
            return Flow::Continue;
        }
        let due = self.clock.now().saturating_add(WATCH_INTERVAL_SECONDS);
        self.sleep_until(due)
    }

    /// Running: the executor reports the turn; nothing else is done meanwhile.
    fn run_turn(&mut self) -> Flow {
        let generation = self.machine.generation();
        let fact = self.ports.poll_turn(self.active_slice);
        match fact {
            Some(fact) => self.apply_as(generation, fact),
            None => self.drain(Duration::ZERO),
        }
    }

    fn read_thread(&mut self) -> Fact {
        match self.ports.read_thread(&self.plan) {
            Ok(fact) => fact,
            Err(error) => {
                self.log_failure("autorun_thread_read_failed", &error);
                Fact::QueryFailed
            }
        }
    }

    /// Waits for the clock to reach `due`, then applies `then` under the generation the wait
    /// started in. A command that changes the machine meanwhile abandons the wait.
    fn wait_until(&mut self, due: i64, then: Fact) -> Flow {
        let generation = self.machine.generation();
        match self.sleep_until(due) {
            Flow::Continue => self.apply_as(generation, then),
            other => other,
        }
    }

    /// Sleeps in slices until the clock reaches `due`, waking early for commands.
    fn sleep_until(&mut self, due: i64) -> Flow {
        let generation = self.machine.generation();
        loop {
            if self.clock.now() >= due {
                return Flow::Continue;
            }
            match self.drain(self.active_slice) {
                Flow::Shutdown => return Flow::Shutdown,
                Flow::Interrupted => return Flow::Interrupted,
                Flow::Continue => {}
            }
            if self.machine.generation() != generation {
                return Flow::Interrupted;
            }
        }
    }

    /// Applies a fact under the current generation, after any command that arrived first.
    fn apply(&mut self, fact: Fact) -> Flow {
        let generation = self.machine.generation();
        self.apply_as(generation, fact)
    }

    /// Applies a fact issued under `generation`. Commands that arrived meanwhile are applied
    /// first, so a result that outlived a pause or a cancel is stale when the machine sees it.
    fn apply_as(&mut self, generation: u64, fact: Fact) -> Flow {
        if let Flow::Shutdown = self.drain(Duration::ZERO) {
            return Flow::Shutdown;
        }
        match self.machine.apply_fact(generation, fact) {
            outcome @ Outcome::Applied(_) => {
                self.adopt(outcome);
                Flow::Continue
            }
            Outcome::Stale { .. } => {
                log(&LogRecord::new(Level::Info, "autorun_stale_result_dropped")
                    .with_phase(Phase::Storage));
                Flow::Interrupted
            }
            Outcome::Ignored(_) => Flow::Continue,
        }
    }

    /// Applies every command waiting, blocking up to `wait` for the first one, then checks the
    /// deadline. Every wait goes through here, so a passed deadline is noticed within a slice.
    fn drain(&mut self, wait: Duration) -> Flow {
        let mut flow = Flow::Continue;
        let mut wait = wait;
        loop {
            let command = match self.inbox.recv_timeout(wait) {
                Ok(command) => command,
                Err(RecvTimeoutError::Timeout) => break,
                // Every handle is gone: nobody can ever tell the driver anything again.
                Err(RecvTimeoutError::Disconnected) => return Flow::Shutdown,
            };
            wait = Duration::ZERO;
            let outcome = match command {
                Command::Shutdown => return Flow::Shutdown,
                Command::Edit(edit, done) => {
                    edit(&mut self.plan);
                    self.machine.set_max_resumes(self.plan.max_resumes);
                    self.after_change();
                    // A caller that gave up waiting has dropped its end; the change stands.
                    if done.send(()).is_err() {
                        log(
                            &LogRecord::new(Level::Warn, "autorun_edit_ack_not_delivered")
                                .with_phase(Phase::Storage),
                        );
                    }
                    continue;
                }
                // Nothing is enabled without a complete plan. The command layer checks too; the
                // driver is the check that cannot be bypassed.
                Command::User(UserEvent::Enable)
                    if self.plan.binding.is_none() || self.plan.participants.is_empty() =>
                {
                    log(
                        &LogRecord::new(Level::Warn, "autorun_enable_refused_incomplete")
                            .with_phase(Phase::Storage),
                    );
                    continue;
                }
                Command::User(event) => {
                    // The one kind of change the log could not otherwise explain: a pause or a
                    // cancel with no reason recorded is somebody pressing the button.
                    log(&LogRecord::new(Level::Info, "autorun_user_event")
                        .with_phase(Phase::Storage)
                        .with_detail(match event {
                            UserEvent::Enable => "enable",
                            UserEvent::Pause => "pause",
                            UserEvent::Resume => "resume",
                            UserEvent::Cancel => "cancel",
                        }));
                    self.machine.apply_user(event)
                }
                Command::SendText(text) => {
                    // The text itself is session content: only its length is ever recorded.
                    log(&LogRecord::new(Level::Info, "autorun_user_event")
                        .with_phase(Phase::Storage)
                        .with_detail(&format!("send {} chars", text.chars().count())));
                    self.pending_text = Some(text);
                    // Not the path a "continue" press takes: that one re-arms and waits for the
                    // thread to produce a trigger, and a thread parked on a question never
                    // will. A sentence goes straight to a continuation.
                    let outcome = self.machine.apply_steer();
                    // A sentence waits for whatever continuation is already on its way - that is
                    // seconds off, and it is the same turn the user meant. A turn that is
                    // already running is the one case where nothing can carry it: it cannot be
                    // steered, and the continuation after it belongs to a different moment.
                    if self.machine.state() == State::Running {
                        self.pending_text = None;
                    }
                    outcome
                }
                Command::Observed(observation) => {
                    let generation = self.machine.generation();
                    let fact = match observation {
                        Observation::ManualSwitch => Fact::ManualSwitchObserved,
                        Observation::DesktopReopened => Fact::DesktopReopened,
                    };
                    self.machine.apply_fact(generation, fact)
                }
            };
            if self.adopt(outcome) {
                flow = Flow::Interrupted;
            }
        }
        if self.check_deadline() {
            flow = Flow::Interrupted;
        }
        flow
    }

    /// Whether the user's deadline has passed and the machine acted on it.
    fn check_deadline(&mut self) -> bool {
        let Some(deadline) = self.plan.deadline else {
            return false;
        };
        if self.clock.now() < deadline
            || matches!(self.machine.state(), State::Stopped | State::Disabled)
        {
            return false;
        }
        let generation = self.machine.generation();
        let outcome = self.machine.apply_fact(generation, Fact::DeadlineReached);
        self.adopt(outcome)
    }

    /// Takes the machine's answer: a new action is adopted and everything that follows a
    /// change is done. A relative wait becomes an absolute instant here, once, so the plan can
    /// say when the next check is and a clock that jumps meanwhile cannot stretch it.
    fn adopt(&mut self, outcome: Outcome) -> bool {
        let Outcome::Applied(action) = outcome else {
            return false;
        };
        self.action = match action {
            Action::WaitFor { delay } => Action::WaitUntil {
                at: self.clock.now().saturating_add(as_seconds(delay)),
            },
            other => other,
        };
        self.after_change();
        true
    }

    /// After every accepted event: the plan on disk, the sleep assertion, the executor
    /// session, and whoever is watching.
    fn after_change(&mut self) {
        let state = self.machine.state();

        let executing = matches!(
            state,
            State::Resuming | State::Running | State::WaitingNetwork
        );
        if self.executing && !executing {
            self.ports.stop_executing();
        }
        self.executing = executing;

        if state.is_active() {
            if self.hold.is_none() {
                match self.power.acquire(ASSERTION_REASON) {
                    Ok(hold) => self.hold = Some(hold),
                    Err(error) => self.log_failure("autorun_sleep_assertion_unavailable", &error),
                }
            }
        } else {
            self.hold = None;
        }

        let now = self.clock.now();
        self.plan.record(&self.machine, &rfc3339(now));
        if let Action::WaitUntil { at } = self.action {
            // The machine only knows the expected time; the retry after a failed read is an
            // instant the driver fixed, and the user is owed that one too.
            self.plan.next_check_at = Some(at);
        }
        if let Err(error) = self.store.save(&self.plan) {
            // The state is right in memory and wrong on disk. A restart would come back one
            // step behind - and paused, which is the safe side. Reported, not fatal.
            self.log_failure("autorun_plan_not_saved", &error);
        }
        self.observer.changed(&self.plan);
    }

    fn log_failure(&self, event: &'static str, error: &TogletError) {
        log(&LogRecord::new(Level::Warn, event)
            .with_phase(error.phase())
            .with_code(error.code()));
    }
}

/// What a restored machine is doing. Only states that wait for the user come out of a restore.
fn initial_action(state: State) -> Action {
    match state {
        State::Armed => Action::WatchThread,
        _ => Action::Wait,
    }
}

fn as_seconds(delay: Duration) -> i64 {
    i64::try_from(delay.as_secs()).unwrap_or(i64::MAX)
}

/// Formats unix seconds as `1970-01-01T00:00:00Z` (UTC).
pub fn rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The inverse of [`rfc3339`], for the timestamps the profiles carry: `YYYY-MM-DDTHH:MM:SS`,
/// an optional fraction, and `Z`. Anything else is `None`, never a guessed date.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let time = time.split_once('.').map_or(time, |(whole, _)| whole);
    let mut date_parts = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
        date_parts.next()?.ok()?,
    );
    if date_parts.next().is_some() {
        return None;
    }
    let mut time_parts = time.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        time_parts.next()?.ok()?,
        time_parts.next()?.ok()?,
        time_parts.next()?.ok()?,
    );
    if time_parts.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Howard Hinnant's `days_from_civil`.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant's `civil_from_days`, for days since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    // Both are small positive numbers by construction; the casts cannot truncate.
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_seconds_format_as_utc_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_757_548_800), "2025-09-11T00:00:00Z");
        assert_eq!(rfc3339(1_800_000_000), "2027-01-15T08:00:00Z");
        assert_eq!(rfc3339(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn dates_parse_back_to_the_seconds_they_came_from() {
        for seconds in [0, 951_782_400, 1_757_548_800, 1_800_000_000, 86_399, 86_400] {
            assert_eq!(parse_rfc3339(&rfc3339(seconds)), Some(seconds));
        }
        assert_eq!(
            parse_rfc3339("2026-09-11T10:00:00.250Z"),
            Some(1_789_120_800)
        );
        assert_eq!(rfc3339(1_789_120_800), "2026-09-11T10:00:00Z");
        assert_eq!(parse_rfc3339("2026-09-11T10:00:00"), None);
        assert_eq!(parse_rfc3339("2026-13-11T10:00:00Z"), None);
        assert_eq!(parse_rfc3339("1757548800"), None);
        assert_eq!(parse_rfc3339(""), None);
    }

    #[test]
    fn a_restored_machine_starts_idle_unless_armed() {
        assert_eq!(initial_action(State::Paused), Action::Wait);
        assert_eq!(initial_action(State::NeedsHuman), Action::Wait);
        assert_eq!(initial_action(State::Armed), Action::WatchThread);
    }
}
