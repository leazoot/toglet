//! Background loop that reads the reset feed while the switch is on.
//!
//! Same shape as `remote_poll`: a plain thread that mostly waits. The wait is interruptible, so
//! turning the switch on reads at once instead of up to a minute later. No connection is opened
//! here; `resets::fetch` does that.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use super::resets::{AnnouncementView, RESET_ANNOUNCED_EVENT, Resets, unix_seconds};
use crate::diagnostics::{Level, LogRecord, Phase, log};
use crate::quota::Backoff;
use crate::resets::{self, Fetched};

const PHASE: Phase = Phase::Resets;

/// Wait between looks at the switch while it is off. Nothing is sent on these rounds.
const DORMANT: Duration = Duration::from_secs(60);

/// Starts the poll thread. `woken` is nudged by a save, which ends the current wait early.
pub fn start(app: AppHandle, woken: Receiver<()>) {
    let started = thread::Builder::new()
        .name("toglet-resets-poll".to_owned())
        .spawn(move || {
            let mut backoff = Backoff::new();
            loop {
                let wait = round(&app, &mut backoff);
                match woken.recv_timeout(wait) {
                    // Nudged or timed out: either way, the next round decides what to do.
                    Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                    // The sender lives in managed state, so this only happens at shutdown.
                    Err(RecvTimeoutError::Disconnected) => thread::sleep(wait),
                }
            }
        });
    if started.is_err() {
        // Optional, like the tray: a failure to start is logged, not fatal.
        log(&LogRecord::new(Level::Warn, "resets_poll_not_started").with_phase(PHASE));
    }
}

/// One pass. Returns how long to wait before the next one.
fn round(app: &AppHandle, backoff: &mut Backoff) -> Duration {
    let resets = app.state::<Resets>();
    if !resets.snapshot().enabled {
        *backoff = Backoff::new();
        return DORMANT;
    }

    let etag = resets.etag();
    match tauri::async_runtime::block_on(resets::fetch(etag.as_deref())) {
        Ok(Fetched::Unchanged) => {
            *backoff = backoff.after_success();
            resets.record_unchanged(unix_seconds());
            resets.announce_state(app);
            resets::next_wait(*backoff, None)
        }
        Ok(Fetched::Fresh { status, etag }) => {
            *backoff = backoff.after_success();
            // Decided before the markers move on, or nothing would ever be new.
            let events = resets.announcements(&status);
            resets.record_reading(*status, etag, unix_seconds());
            resets.announce_state(app);
            for event in &events {
                if app
                    .emit(RESET_ANNOUNCED_EVENT, AnnouncementView::of(event))
                    .is_err()
                {
                    log(
                        &LogRecord::new(Level::Warn, "reset_announcement_not_delivered")
                            .with_phase(PHASE),
                    );
                }
            }
            resets::next_wait(*backoff, None)
        }
        Err(retry) => {
            *backoff = backoff.after_failure();
            log(&LogRecord::from_error("resets_fetch_failed", &retry.error));
            resets.record_failure(retry.error.code().as_str());
            resets.announce_state(app);
            resets::next_wait(*backoff, retry.after)
        }
    }
}
