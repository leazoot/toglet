//! Background loop that polls the bridge and delivers verified commands to `AutoRun`.
//!
//! Order matters: rebind the session id before sending, verify with `guard` (which advances the
//! replay counter), persist the counter, deliver, then poll again at once so the outcome receipt
//! reaches the phone. No connection is opened here; `remote::poll` does that.

use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Manager};

use super::autorun::AutoRun;
use super::remote::Remote;
use super::state::AppState;
use crate::autorun::plan::AutoRunPlan;
use crate::autorun::{Clock, SystemClock};
use crate::diagnostics::{Level, LogRecord, Phase, log};
use crate::quota::Backoff;
use crate::remote::envelope::{Action, Context, Outcome, Receipt};
use crate::remote::guard::{self, Guard};
use crate::remote::poll;
use crate::remote::store::{LastCommandRecord, RemoteConfig, binding_fingerprint, load_bridge};

const PHASE: Phase = Phase::Remote;

/// Wait between rounds when nothing can be asked: feature off, unpaired, or no actionable state.
/// Nothing is sent on these rounds; they only let enabling take effect without a restart.
const DORMANT: Duration = Duration::from_secs(60);

/// Delay before the follow-up poll whose receipt reports a command's outcome to the phone.
const CONFIRM: Duration = Duration::from_secs(1);

/// Starts the poll thread. A plain thread: the loop mostly sleeps, and only the request itself
/// runs on the async runtime.
pub fn start(app: AppHandle) {
    let started = thread::Builder::new()
        .name("toglet-remote-poll".to_owned())
        .spawn(move || {
            let mut guard = restored(&app);
            let mut backoff = Backoff::new();
            loop {
                let wait = round(&app, &mut guard, &mut backoff);
                thread::sleep(wait);
            }
        });
    if started.is_err() {
        // Optional, like the tray: a failure to start is logged, not fatal.
        log(&LogRecord::new(Level::Warn, "remote_poll_not_started").with_phase(PHASE));
    }
}

/// Replay state left by the previous run; a counter reset on restart would allow replaying
/// older envelopes.
fn restored(app: &AppHandle) -> Guard {
    let config = app.state::<Remote>().snapshot();
    Guard::restore(config.cursor, config.recent_nonces)
}

/// One pass. Returns how long to wait before the next one.
fn round(app: &AppHandle, guard: &mut Guard, backoff: &mut Backoff) -> Duration {
    let remote = app.state::<Remote>();
    let autorun = app.state::<AutoRun>();
    let state = app.state::<AppState>();

    let mut config = remote.snapshot();
    let plan = autorun.latest();

    if rebind(&mut config, &plan) {
        *guard = Guard::restore(config.cursor, config.recent_nonces.clone());
        if let Err(error) = remote.record(&config) {
            log(&LogRecord::from_error(
                "remote_settings_not_written",
                &error,
            ));
        }
    }

    let Some(interval) = poll::interval(config.enabled, plan.state, *backoff) else {
        return DORMANT;
    };

    // Enabled but not yet paired: not an error, and not worth logging every round.
    let Ok(bridge) = load_bridge(state.secrets()) else {
        return DORMANT;
    };

    let receipt = receipt_of(&config, &plan, interval);
    let answer = tauri::async_runtime::block_on(poll::exchange(
        &bridge.endpoint,
        bridge.secret.as_bytes(),
        &receipt,
    ));

    let payload = match answer {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            *backoff = backoff.after_success();
            return interval;
        }
        Err(error) => {
            *backoff = backoff.after_failure();
            log(&LogRecord::from_error("remote_poll_failed", &error));
            return backoff.delay();
        }
    };
    *backoff = backoff.after_success();

    handle(
        &remote,
        &autorun,
        guard,
        &config,
        plan.state.as_str(),
        bridge.secret.as_bytes(),
        &payload,
    );
    CONFIRM
}

/// Issues a new session id when the bound task changed, so a command signed for the previous
/// task cannot land on its replacement.
fn rebind(config: &mut RemoteConfig, plan: &AutoRunPlan) -> bool {
    let fingerprint = plan
        .binding
        .as_ref()
        .map(|binding| binding_fingerprint(&binding.thread_id));
    config.rebind(fingerprint.as_deref())
}

/// Toglet's status for the phone: codes and timestamps only, never sentences.
fn receipt_of(config: &RemoteConfig, plan: &AutoRunPlan, next_poll: Duration) -> Receipt {
    Receipt {
        device_id: config.device_id.clone(),
        session_id: config.session_id.clone(),
        issued_at: SystemClock.now(),
        state: plan.state.as_str().to_owned(),
        wait_reason: plan.wait_reason.clone(),
        expected_available_at: plan.expected_available_at,
        cursor: config.cursor,
        next_poll_seconds: next_poll.as_secs(),
        resume_count: plan.resume_count,
        last_command: config
            .last_command
            .as_ref()
            .and_then(LastCommandRecord::to_envelope),
    }
}

/// Verifies one payload, delivers it, and records the outcome.
fn handle(
    remote: &Remote,
    autorun: &AutoRun,
    guard: &mut Guard,
    config: &RemoteConfig,
    state: &str,
    secret: &[u8],
    payload: &str,
) {
    let now = SystemClock.now();
    let nonces = guard.nonces();
    let context = Context {
        secret,
        session_id: &config.session_id,
        state,
        now,
        cursor: guard.cursor(),
        recent_nonces: &nonces,
    };

    let accepted = match guard.admit(payload, &context) {
        Ok(accepted) => accepted,
        Err(outcome) => {
            // Not recorded: a `lastCommand` with an unaccepted counter would tell the phone its
            // command was collected.
            guard::audit(None, outcome);
            return;
        }
    };

    let outcome = deliver(autorun, accepted.action);
    guard::audit(Some(accepted.action), outcome);

    // Re-read the settings: the user may have saved while the request was in flight, and only
    // these fields belong to the poller.
    let mut latest = remote.snapshot();
    latest.cursor = guard.cursor();
    latest.recent_nonces = guard.nonces();
    latest.last_command = Some(LastCommandRecord::new(
        accepted.counter,
        accepted.action,
        outcome,
        now,
    ));
    if let Err(error) = remote.record(&latest) {
        log(&LogRecord::from_error(
            "remote_settings_not_written",
            &error,
        ));
    }
}

/// Delivers the action through the same path as the panel's buttons. `Applied` only when the
/// event was actually taken, so a driver that never started is not reported as success.
fn deliver(autorun: &AutoRun, action: Action) -> Outcome {
    let Some(event) = guard::event_for(action) else {
        // `status` changes nothing; the receipt that follows is the whole answer.
        return Outcome::Applied;
    };
    match autorun.user(event) {
        Ok(()) => Outcome::Applied,
        Err(error) => {
            log(&LogRecord::from_error(
                "remote_command_not_delivered",
                &error,
            ));
            Outcome::Unavailable
        }
    }
}
