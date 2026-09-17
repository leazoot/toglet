//! A phone command end to end: from the bytes off the bridge to the event the task receives.
//!
//! Checks that refusals never reach the state machine, and that a remote continue produces
//! the same event as the panel's button.

use toglet_lib::autorun::UserEvent;
use toglet_lib::remote::envelope::{Action, Context, Outcome};
use toglet_lib::remote::guard::{Guard, event_for};
use toglet_lib::remote::mac;

const SECRET: &[u8] = b"the secret the user typed into both ends";
const SESSION: &str = "a83b4c1d9e0f2a3b4c5d6e7f80912233";
const OTHER_SESSION: &str = "ffffffff11112222333344445555ffff";
const NOW: i64 = 1_757_664_000;

/// Stands in for the driver, recording what it was told so a test can assert nothing was.
#[derive(Default)]
struct Task {
    delivered: Vec<UserEvent>,
}

impl Task {
    /// The whole of the remote execution path: admit, map, deliver. `deliver` is the only line
    /// that touches the task, and it is reached only through `Guard::admit`.
    fn receive(&mut self, guard: &mut Guard, payload: &str, state: &str, now: i64) -> Outcome {
        let nonces = guard.nonces();
        let context = Context {
            secret: SECRET,
            session_id: SESSION,
            state,
            now,
            cursor: guard.cursor(),
            recent_nonces: &nonces,
        };
        match guard.admit(payload, &context) {
            Ok(accepted) => {
                if let Some(event) = event_for(accepted.action) {
                    self.delivered.push(event);
                }
                Outcome::Applied
            }
            Err(outcome) => outcome,
        }
    }
}

fn command(
    action: &str,
    session: &str,
    observed: &str,
    counter: u64,
    nonce: &str,
    at: i64,
) -> String {
    let signed = [
        "toglet-remote/2",
        "command",
        action,
        session,
        observed,
        &counter.to_string(),
        nonce,
        &at.to_string(),
        // The argument-free actions sign an empty text segment; `send` has its own helper.
        "",
    ]
    .join("\n");
    let mac = mac::sign_hex(SECRET, signed.as_bytes());
    format!(
        r#"{{"v":2,"kind":"command","action":"{action}","sessionId":"{session}",
           "observedState":"{observed}","counter":{counter},"nonce":"{nonce}",
           "issuedAt":{at},"mac":"{mac}"}}"#
    )
}

fn nonce(seed: u64) -> String {
    format!("{seed:032x}")
}

#[test]
fn a_genuine_continue_reaches_the_task_as_the_event_the_panel_sends() {
    let mut guard = Guard::restore(0, Vec::new());
    let mut task = Task::default();

    let outcome = task.receive(
        &mut guard,
        &command("resume", SESSION, "needs_human", 1, &nonce(1), NOW),
        "needs_human",
        NOW,
    );

    assert_eq!(outcome, Outcome::Applied);
    assert_eq!(task.delivered, vec![UserEvent::Resume]);
}

/// A late, a replayed and a forged command change nothing: not the task, the counter or the nonces.
#[test]
fn a_late_a_replayed_and_a_forged_command_change_nothing() {
    let mut guard = Guard::restore(0, Vec::new());
    let mut task = Task::default();

    let genuine = command("resume", SESSION, "needs_human", 1, &nonce(1), NOW);
    assert_eq!(
        task.receive(&mut guard, &genuine, "needs_human", NOW),
        Outcome::Applied
    );
    let after_genuine = (guard.cursor(), guard.nonces(), task.delivered.len());

    // The same envelope again.
    assert_eq!(
        task.receive(&mut guard, &genuine, "needs_human", NOW),
        Outcome::Replayed
    );

    // Issued while the machine was asleep, but too long ago to still be meant.
    let late = command("cancel", SESSION, "needs_human", 2, &nonce(2), NOW - 1_000);
    assert_eq!(
        task.receive(&mut guard, &late, "needs_human", NOW),
        Outcome::Expired
    );

    // Signed with a secret the bridge does not have.
    //
    // The flip is unconditional on purpose. This was written as "replace every 'a' with 'b'",
    // which is a no-op whenever the signature happens to contain no 'a' - about 1.6% of
    // signatures - and then this test silently asserts nothing. BATCH-07 changed the signed
    // bytes and rolled exactly such a signature, which is how it was found.
    let forged = {
        let real = command("cancel", SESSION, "needs_human", 3, &nonce(3), NOW);
        let marker = "\"mac\":\"";
        let at = real.rfind(marker).expect("a mac") + marker.len();
        let flipped = if real[at..].starts_with('0') {
            '1'
        } else {
            '0'
        };
        format!("{}{flipped}{}", &real[..at], &real[at + 1..])
    };
    assert_eq!(
        task.receive(&mut guard, &forged, "needs_human", NOW),
        Outcome::BadMac
    );

    // Meant for a binding that no longer exists.
    let stale_binding = command("resume", OTHER_SESSION, "needs_human", 4, &nonce(4), NOW);
    assert_eq!(
        task.receive(&mut guard, &stale_binding, "needs_human", NOW),
        Outcome::SessionMismatch
    );

    assert_eq!(
        (guard.cursor(), guard.nonces(), task.delivered.len()),
        after_genuine,
        "a refused command must leave the guard and the task exactly as they were"
    );
}

/// Continuing is the one press aimed at a particular screen. Pausing and cancelling are not,
/// because they mean the same thing whatever the task is doing.
#[test]
fn a_continue_aimed_at_a_screen_that_has_moved_on_is_refused_but_a_cancel_is_not() {
    let mut guard = Guard::restore(0, Vec::new());
    let mut task = Task::default();

    let resume = command("resume", SESSION, "needs_human", 1, &nonce(1), NOW);
    assert_eq!(
        task.receive(&mut guard, &resume, "running", NOW),
        Outcome::StateChanged
    );
    assert!(task.delivered.is_empty());

    let cancel = command("cancel", SESSION, "needs_human", 1, &nonce(1), NOW);
    assert_eq!(
        task.receive(&mut guard, &cancel, "running", NOW),
        Outcome::Applied
    );
    assert_eq!(task.delivered, vec![UserEvent::Cancel]);
}

/// Asking for a status is not a way to make anything happen.
#[test]
fn a_status_request_tells_the_task_nothing() {
    let mut guard = Guard::restore(0, Vec::new());
    let mut task = Task::default();

    let outcome = task.receive(
        &mut guard,
        &command("status", SESSION, "needs_human", 1, &nonce(1), NOW),
        "needs_human",
        NOW,
    );

    assert_eq!(outcome, Outcome::Applied);
    assert!(task.delivered.is_empty());
}

/// Remote control must not be a second implementation: the interface's commands are read to
/// check that the phone sends the same events.
#[test]
fn the_interface_and_the_phone_send_the_same_events() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/autorun.rs"),
    )
    .expect("the command layer is readable");

    for (command_fn, event, action) in [
        ("resume_autorun", "UserEvent::Resume", Action::Resume),
        ("pause_autorun", "UserEvent::Pause", Action::Pause),
        ("cancel_autorun", "UserEvent::Cancel", Action::Cancel),
    ] {
        let body = source
            .split(&format!("fn {command_fn}"))
            .nth(1)
            .unwrap_or_else(|| panic!("{command_fn} should exist"));
        let body = body.split("\npub fn").next().unwrap_or(body);
        assert!(
            body.contains(event),
            "{command_fn} should still send {event}"
        );
        assert_eq!(
            event_for(action).map(|sent| format!("UserEvent::{sent:?}")),
            Some(event.to_owned()),
            "the phone must send what {command_fn} sends"
        );
    }

    // `send` is the one action whose text cannot ride on a `UserEvent`, so it is checked by the
    // route it takes instead: the same command, the same validation, no second implementation.
    let body = source
        .split("fn send_autorun")
        .nth(1)
        .expect("send_autorun should exist");
    let body = body.split("\n#[tauri::command]").next().unwrap_or(body);
    assert!(
        body.contains("autorun.send_text(") || body.contains("send(&autorun"),
        "send_autorun should go through AutoRun"
    );
    assert!(
        source.contains("fn send(autorun: &AutoRun, text: &str)")
            && source.contains("validate_instruction(text)"),
        "a phone sentence must pass the same validation a bound instruction does"
    );
    assert_eq!(
        event_for(Action::Send),
        None,
        "send carries a text, so it deliberately maps to no UserEvent"
    );
}

/// Unit tests prove each part correct but not that the application calls it; this checks that
/// start-up launches the poll loop and the loop reaches every part.
#[test]
fn the_poll_loop_is_started_and_reaches_the_state_machine() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("lib.rs is readable");
    assert!(
        lib.contains("commands::remote_poll::start("),
        "start-up must launch the poll loop, or every part below is unreachable"
    );

    let loop_source =
        std::fs::read_to_string(root.join("src/commands/remote_poll.rs")).expect("readable");
    for call in [
        // Asks the bridge.
        "poll::exchange(",
        // Decides whether what came back is genuine, and advances the replay counter.
        "guard.admit(",
        // Turns an accepted action into the event the panel's buttons send.
        "guard::event_for(",
        // And sends it through the one door.
        "autorun.user(event)",
        // A command that ends the task leaves a state that no longer polls, so the outcome has
        // to go out before the loop goes quiet - otherwise the phone waits forever.
        "poll::owes_closing_receipt(",
    ] {
        assert!(
            loop_source.contains(call),
            "the poll loop should still call {call}"
        );
    }
}
