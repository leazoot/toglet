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
            // Set when a command has been handled and its outcome has not reached the phone yet.
            let mut unsent = false;
            loop {
                let wait = round(&app, &mut guard, &mut backoff, &mut unsent);
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
fn round(app: &AppHandle, guard: &mut Guard, backoff: &mut Backoff, unsent: &mut bool) -> Duration {
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

    // Enabled but not yet paired: not an error, and not worth logging every round. Loaded before
    // the interval decision, because the round that stops polling is the one that still owes the
    // phone a receipt.
    let Ok(bridge) = load_bridge(state.secrets()) else {
        return DORMANT;
    };

    let Some(interval) = poll::interval(config.enabled, plan.state, *backoff) else {
        // A cancel ends the task, and the state it ends in is the state that stops polling - so
        // the outcome would never be sent and the phone would wait for a result that cannot
        // arrive. This is the round to send it from: the state has settled by now, which it had
        // not when the command was handled.
        if poll::owes_closing_receipt(config.enabled, plan.state, *unsent) {
            let excerpt = sealed_excerpt(&config, &autorun, bridge.secret.as_bytes());
            let receipt = receipt_of(&config, &plan, DORMANT, excerpt);
            match tauri::async_runtime::block_on(poll::exchange(
                &bridge.endpoint,
                bridge.secret.as_bytes(),
                &receipt,
            )) {
                // Sent: whatever the bridge had queued cannot be acted on in this state, and
                // leaving the flag set would repeat this receipt every dormant round.
                Ok(_) => *unsent = false,
                Err(error) => log(&LogRecord::from_error("remote_poll_failed", &error)),
            }
        }
        return DORMANT;
    };

    // Sealed here and nowhere else: the plaintext never leaves Rust.
    let excerpt = sealed_excerpt(&config, &autorun, bridge.secret.as_bytes());
    let receipt = receipt_of(&config, &plan, interval, excerpt);
    let answer = tauri::async_runtime::block_on(poll::exchange(
        &bridge.endpoint,
        bridge.secret.as_bytes(),
        &receipt,
    ));

    let payload = match answer {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            *backoff = backoff.after_success();
            // The receipt just sent carried whatever outcome was stored, so nothing is owed.
            *unsent = false;
            return interval;
        }
        Err(error) => {
            *backoff = backoff.after_failure();
            log(&LogRecord::from_error("remote_poll_failed", &error));
            return backoff.delay();
        }
    };
    *backoff = backoff.after_success();
    // The receipt sent above carried the previous outcome; this round's is owed from here on.
    *unsent = false;

    handle(
        &remote,
        &autorun,
        guard,
        &config,
        plan.state.as_str(),
        bridge.secret.as_bytes(),
        &payload,
    );

    // Deliberately not decided here. A user event is handed to the driver without waiting for it
    // to be applied, so reading the state back now can still show the state this command ended,
    // which would judge the receipt unnecessary and lose it. The next round decides, by which
    // time the state has settled.
    *unsent = true;
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
fn receipt_of(
    config: &RemoteConfig,
    plan: &AutoRunPlan,
    next_poll: Duration,
    excerpt: Option<crate::remote::crypt::Sealed>,
) -> Receipt {
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
        excerpt_ciphertext: excerpt.as_ref().map(|sealed| sealed.ciphertext_hex.clone()),
        excerpt_nonce: excerpt.as_ref().map(|sealed| sealed.nonce_hex.clone()),
        last_command: config
            .last_command
            .as_ref()
            .and_then(LastCommandRecord::to_envelope),
    }
}

/// The agent's last message, sealed, or `None` when the user has the preview switched off,
/// when the session has not been read yet, or when sealing failed.
///
/// A failure to seal is `None` rather than plaintext: the point of the field is that only the
/// phone can read it.
fn sealed_excerpt(
    config: &RemoteConfig,
    autorun: &AutoRun,
    secret: &[u8],
) -> Option<crate::remote::crypt::Sealed> {
    if !config.share_excerpt {
        return None;
    }
    let excerpt = autorun.agent_excerpt()?;
    match crate::remote::crypt::seal(secret, &excerpt) {
        Ok(sealed) => Some(sealed),
        Err(error) => {
            log(&LogRecord::from_error("remote_excerpt_not_sealed", &error));
            None
        }
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

    let outcome = deliver(autorun, accepted.action, accepted.text.as_deref());
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
fn deliver(autorun: &AutoRun, action: Action, text: Option<&str>) -> Outcome {
    // The one action with a parameter. It goes through `AutoRun`, the same door the panel uses;
    // the driver turns it into the continuation a "continue" press would have started.
    if action == Action::Send {
        let Some(text) = text else {
            // Verified as a `send` but carrying nothing: refused rather than turned into a
            // plain continue, which would run words the user never wrote.
            return Outcome::Malformed;
        };
        return match autorun.send_text(text.to_owned()) {
            Ok(()) => Outcome::Applied,
            Err(error) => {
                log(&LogRecord::from_error(
                    "remote_command_not_delivered",
                    &error,
                ));
                Outcome::Unavailable
            }
        };
    }
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
