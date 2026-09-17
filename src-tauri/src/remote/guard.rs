//! Replay protection, rate limiting, and mapping accepted actions onto the same user events
//! the panel sends, so remote control adds no second path to the scheduler.

use std::collections::VecDeque;

use super::envelope::{self, Accepted, Action, Context, Outcome};
use crate::autorun::UserEvent;
use crate::diagnostics::{Level, LogRecord, Phase, log};

const PHASE: Phase = Phase::Remote;

/// How many recently accepted nonces to remember; a backup to the counter, which is the
/// primary replay defence.
pub const NONCE_MEMORY: usize = 64;

/// Accepted commands allowed per window; bounds the damage of a leaked secret.
const RATE_WINDOW_SECONDS: i64 = 60;
const RATE_ALLOWANCE: usize = 10;

#[derive(Debug, Clone, Default)]
pub struct Guard {
    cursor: u64,
    nonces: VecDeque<String>,
    accepted_at: VecDeque<i64>,
}

impl Guard {
    /// Restores persisted state; the counter must survive restarts or old envelopes replay.
    pub fn restore(cursor: u64, nonces: Vec<String>) -> Self {
        let mut nonces: VecDeque<String> = nonces.into();
        while nonces.len() > NONCE_MEMORY {
            nonces.pop_front();
        }
        Self {
            cursor,
            nonces,
            accepted_at: VecDeque::new(),
        }
    }

    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// The nonces to persist, oldest first.
    pub fn nonces(&self) -> Vec<String> {
        self.nonces.iter().cloned().collect()
    }

    /// Decides whether a payload may be acted on, and remembers it only if so.
    ///
    /// State advances only after verification; otherwise anyone reaching the bridge could push
    /// the counter past what the real phone sends next.
    pub fn admit(&mut self, payload: &str, context: &Context<'_>) -> Result<Accepted, Outcome> {
        let checked = envelope::check(payload, context)?;

        // Rate-limit only verified commands, so unsigned junk cannot lock the real phone out.
        self.forget_older_than(context.now);
        if self.accepted_at.len() >= RATE_ALLOWANCE {
            return Err(Outcome::RateLimited);
        }

        self.cursor = checked.counter;
        self.nonces.push_back(checked.nonce.clone());
        if self.nonces.len() > NONCE_MEMORY {
            self.nonces.pop_front();
        }
        self.accepted_at.push_back(context.now);
        Ok(checked)
    }

    fn forget_older_than(&mut self, now: i64) {
        while let Some(at) = self.accepted_at.front() {
            if now - at >= RATE_WINDOW_SECONDS {
                self.accepted_at.pop_front();
            } else {
                break;
            }
        }
    }
}

/// The user event an action stands for; `Status` only requests a receipt, so it maps to `None`.
///
/// There is deliberately no remote-only event: every action is one the panel can also send.
///
/// `Send` also maps to `None`, but for the opposite reason: it has no event *yet*. TASK-201
/// wires it to the takeover chain. Until then the poller refuses it outright - mapping to
/// `None` here must never be read as "nothing to do, so it worked".
pub fn event_for(action: Action) -> Option<UserEvent> {
    match action {
        Action::Resume => Some(UserEvent::Resume),
        Action::Pause => Some(UserEvent::Pause),
        Action::Cancel => Some(UserEvent::Cancel),
        Action::Status | Action::Send => None,
    }
}

/// Logs a command's action and outcome code only; never the envelope, address, nonce, or MAC.
pub fn audit(action: Option<Action>, outcome: Outcome) {
    let detail = match action {
        Some(action) => format!("{} {}", action.as_str(), outcome.as_str()),
        None => outcome.as_str().to_owned(),
    };
    let level = if outcome == Outcome::Applied {
        Level::Info
    } else {
        Level::Warn
    };
    log(&LogRecord::new(level, "remote_command")
        .with_phase(PHASE)
        .with_detail(&detail));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::mac;

    const SECRET: &[u8] = b"shared";
    const SESSION: &str = "a83b4c1d9e0f2a3b4c5d6e7f80912233";
    const NOW: i64 = 1_757_664_000;

    fn payload(action: &str, counter: u64, nonce: &str, issued_at: i64) -> String {
        let signed = [
            "toglet-remote/2",
            "command",
            action,
            SESSION,
            "needs_human",
            &counter.to_string(),
            nonce,
            &issued_at.to_string(),
            // The four argument-free actions sign an empty text segment.
            "",
        ]
        .join("\n");
        let mac = mac::sign_hex(SECRET, signed.as_bytes());
        format!(
            r#"{{"v":2,"kind":"command","action":"{action}","sessionId":"{SESSION}",
               "observedState":"needs_human","counter":{counter},"nonce":"{nonce}",
               "issuedAt":{issued_at},"mac":"{mac}"}}"#
        )
    }

    fn nonce(seed: u64) -> String {
        format!("{seed:032x}")
    }

    fn admit(guard: &mut Guard, payload: &str, now: i64) -> Result<Accepted, Outcome> {
        let nonces = guard.nonces();
        let context = Context {
            secret: SECRET,
            session_id: SESSION,
            state: "needs_human",
            now,
            cursor: guard.cursor(),
            recent_nonces: &nonces,
        };
        guard.admit(payload, &context)
    }

    #[test]
    fn an_accepted_command_moves_the_counter_and_is_remembered() {
        let mut guard = Guard::restore(42, Vec::new());
        let accepted = admit(&mut guard, &payload("resume", 43, &nonce(1), NOW), NOW)
            .expect("should be accepted");

        assert_eq!(accepted.action, Action::Resume);
        assert_eq!(guard.cursor(), 43);
        assert_eq!(guard.nonces(), vec![nonce(1)]);
    }

    #[test]
    fn the_same_command_twice_is_obeyed_once() {
        let mut guard = Guard::restore(42, Vec::new());
        let command = payload("cancel", 43, &nonce(1), NOW);

        assert!(admit(&mut guard, &command, NOW).is_ok());
        assert_eq!(admit(&mut guard, &command, NOW), Err(Outcome::Replayed));
        assert_eq!(guard.cursor(), 43);
    }

    #[test]
    fn nothing_that_is_refused_leaves_a_trace_in_the_guard() {
        let mut guard = Guard::restore(42, Vec::new());
        let before = (guard.cursor(), guard.nonces());

        let forged = payload("resume", 99, &nonce(7), NOW).replace(
            r#""mac":""#,
            r#""mac":"0000000000000000000000000000000000000000000000000000000000000000","ignored":""#,
        );
        assert!(admit(&mut guard, &forged, NOW).is_err());
        assert!(admit(&mut guard, "not json", NOW).is_err());
        assert!(
            admit(&mut guard, &payload("resume", 40, &nonce(8), NOW), NOW).is_err(),
            "a counter that went backwards"
        );
        assert!(
            admit(
                &mut guard,
                &payload("resume", 60, &nonce(9), NOW - 5_000),
                NOW
            )
            .is_err(),
            "an envelope older than the window"
        );

        assert_eq!((guard.cursor(), guard.nonces()), before);
    }

    #[test]
    fn a_counter_that_survives_a_restart_still_refuses_what_came_before_it() {
        let mut guard = Guard::restore(43, vec![nonce(1)]);
        assert_eq!(
            admit(&mut guard, &payload("resume", 43, &nonce(2), NOW), NOW),
            Err(Outcome::Replayed)
        );
    }

    #[test]
    fn a_nonce_remembered_across_a_restart_is_still_refused() {
        let mut guard = Guard::restore(42, vec![nonce(5)]);
        assert_eq!(
            admit(&mut guard, &payload("resume", 43, &nonce(5), NOW), NOW),
            Err(Outcome::NonceReused)
        );
    }

    #[test]
    fn only_the_most_recent_nonces_are_kept() {
        let stored: Vec<String> = (0..NONCE_MEMORY as u64 + 20).map(nonce).collect();
        let guard = Guard::restore(1, stored);
        assert_eq!(guard.nonces().len(), NONCE_MEMORY);
        assert_eq!(
            guard.nonces().last().map(String::as_str),
            Some(nonce(NONCE_MEMORY as u64 + 19).as_str())
        );
    }

    #[test]
    fn a_flood_of_genuine_commands_is_cut_off_and_resumes_after_the_window() {
        let mut guard = Guard::restore(0, Vec::new());
        for counter in 1..=RATE_ALLOWANCE as u64 {
            assert!(
                admit(
                    &mut guard,
                    &payload("status", counter, &nonce(counter), NOW),
                    NOW
                )
                .is_ok(),
                "command {counter} should be within the allowance"
            );
        }

        let over = admit(&mut guard, &payload("status", 99, &nonce(99), NOW), NOW);
        assert_eq!(over, Err(Outcome::RateLimited));
        // A refused command must not advance the counter.
        assert_eq!(guard.cursor(), RATE_ALLOWANCE as u64);

        let later = NOW + RATE_WINDOW_SECONDS;
        assert!(admit(&mut guard, &payload("status", 99, &nonce(99), later), later).is_ok());
    }

    #[test]
    fn the_four_actions_map_onto_the_events_the_interface_already_uses() {
        assert_eq!(event_for(Action::Resume), Some(UserEvent::Resume));
        assert_eq!(event_for(Action::Pause), Some(UserEvent::Pause));
        assert_eq!(event_for(Action::Cancel), Some(UserEvent::Cancel));
        assert_eq!(event_for(Action::Status), None);
    }

    /// `send` maps to no `UserEvent` because its text cannot ride on one; the poller carries it
    /// instead - through `AutoRun`, the same door the panel uses, never a second path.
    #[test]
    fn send_carries_its_text_through_the_one_door_the_panel_uses() {
        assert_eq!(event_for(Action::Send), None);

        let loop_source = include_str!("../commands/remote_poll.rs");
        assert!(
            loop_source.contains("autorun.send_text("),
            "the poll loop must deliver a send through AutoRun, not by its own route"
        );
        assert!(
            !loop_source.contains("thread/resume") && !loop_source.contains("turn/start"),
            "the poll loop must never reach the app server itself"
        );
    }

    #[test]
    fn no_action_produces_an_event_the_interface_cannot() {
        assert_ne!(event_for(Action::Resume), Some(UserEvent::Enable));
        assert_ne!(event_for(Action::Pause), Some(UserEvent::Enable));
        assert_ne!(event_for(Action::Cancel), Some(UserEvent::Enable));
    }
}
