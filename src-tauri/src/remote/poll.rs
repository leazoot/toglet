//! One of the only three files that open outbound connections (with `notify::send` and
//! `resets::fetch`).
//!
//! A single POST carries the status receipt out and brings at most one command back. Payloads
//! and verification live in `envelope`; this file only moves bytes.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

use super::envelope::Receipt;
use crate::autorun::State;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::quota::Backoff;

const PHASE: Phase = Phase::Remote;

/// Long enough for a bridge to hold the request open as a long poll; immediate answers also work.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Poll interval whenever a task is bound.
///
/// A running task used to get 300s of its own, described as "kept only so a remote pause still
/// works" - which is the one thing 300s did not do: a pause or a cancel sat in the queue for up
/// to five minutes while the person watched a spinner. The bridge holds a poll open (see
/// [`TIMEOUT`]), so a short interval parks one request rather than repeating many, which is what
/// that design is for.
const BOUND: Duration = Duration::from_secs(20);

/// A bridge answer; any unfamiliar body means "no command", never a guess.
#[derive(Debug, Deserialize)]
struct Answer {
    v: u32,
    #[serde(default)]
    command: Option<serde_json::Value>,
}

/// When to poll next, following the scheduler state, or `None` to not poll at all.
pub fn interval(enabled: bool, state: State, backoff: Backoff) -> Option<Duration> {
    if !enabled {
        return None;
    }
    // Nothing is bound, so every command would have to be refused.
    if !takes_commands(state) {
        return None;
    }
    if backoff.failures() > 0 {
        return Some(backoff.delay());
    }
    Some(BOUND)
}

/// Whether this state can act on a command at all. One definition: [`owes_closing_receipt`] asks
/// the same question, and two copies of it would drift apart.
fn takes_commands(state: State) -> bool {
    !matches!(state, State::Disabled | State::Stopped)
}

/// Whether one last receipt is owed: an outcome is still unsent, and this state no longer polls.
///
/// Cancelling lands the task in `Disabled` - precisely the state [`interval`] stops polling in -
/// so the receipt carrying that command's outcome would never be sent, and the phone would wait
/// for a result that cannot arrive. Silencing the *command* half is what "nothing is bound"
/// means; the receipt half is what the phone is still waiting for.
///
/// `outcome_unsent` is carried by the caller rather than derived from the state, because a user
/// event is delivered to the driver without waiting for it to be applied (`Driver::user` sends
/// and returns; only `Driver::edit` does the handshake that makes a following read see the
/// change). Reading the state back immediately after handling a command can therefore still show
/// the state the command just ended, which would judge the receipt unnecessary and lose it.
pub fn owes_closing_receipt(enabled: bool, state: State, outcome_unsent: bool) -> bool {
    enabled && outcome_unsent && !takes_commands(state)
}

/// Posts one receipt and returns the bridge's pending command, if any, unverified.
///
/// Only `envelope::check` may decide whether the returned command is genuine.
pub async fn exchange(endpoint: &str, secret: &[u8], receipt: &Receipt) -> Result<Option<String>> {
    check_endpoint(endpoint)?;

    let response = client()?
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(receipt.to_json(secret))
        .send()
        .await
        .map_err(|error| unreachable_bridge(&error.to_string()))?;

    let status = response.status();
    if status.is_server_error() {
        return Err(unreachable_bridge(
            "the bridge reported a failure of its own",
        ));
    }
    if !status.is_success() {
        return Err(refused("the bridge did not accept the request"));
    }

    // The reply body is untrusted text: never logged or quoted in an error.
    let body = response.text().await.unwrap_or_default();
    Ok(command_in(&body))
}

/// Extracts a command from a bridge answer. An unreadable body is "no command", not a failure,
/// so an unfamiliar bridge does not put a working setup into permanent backoff.
fn command_in(body: &str) -> Option<String> {
    let answer: Answer = serde_json::from_str(body).ok()?;
    if answer.v != 1 {
        return None;
    }
    let command = answer.command?;
    if command.is_null() {
        return None;
    }
    serde_json::to_string(&command).ok()
}

/// Re-checks the address at the moment of use, even though stored addresses already passed it.
fn check_endpoint(endpoint: &str) -> Result<()> {
    if crate::net::is_safe_endpoint(endpoint) {
        Ok(())
    } else {
        Err(refused("the bridge address is not one Toglet may post to"))
    }
}

/// Shared client with redirects refused: the posted body is signed for one endpoint only.
fn client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(TIMEOUT)
                // Shorter than `TIMEOUT`, which only exists for long-polling bridges.
                .connect_timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("Toglet/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| {
            TogletError::new(ErrorCode::Internal, PHASE, false, UserAction::None)
                .with_detail("the outbound client could not be prepared")
        })
}

/// Bridge unreachable or failing on its own; retryable.
fn unreachable_bridge(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::NetworkUnavailable,
        PHASE,
        true,
        UserAction::CheckNetwork,
    )
    .with_detail(detail)
}

/// Bridge rejected the request; not retryable, the address is likely wrong.
fn refused(detail: &str) -> TogletError {
    TogletError::new(
        ErrorCode::RemoteBridgeRejected,
        PHASE,
        false,
        UserAction::None,
    )
    .with_detail(detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_polled_while_the_feature_is_off() {
        assert_eq!(interval(false, State::NeedsHuman, Backoff::new()), None);
    }

    #[test]
    fn nothing_is_polled_when_no_task_is_bound() {
        for state in [State::Disabled, State::Stopped] {
            assert_eq!(interval(true, state, Backoff::new()), None);
        }
    }

    #[test]
    fn the_states_a_person_could_act_on_are_asked_about_often() {
        for state in [
            State::NeedsHuman,
            State::Paused,
            State::WaitingQuota,
            State::RoundCompleted,
        ] {
            assert_eq!(interval(true, state, Backoff::new()), Some(BOUND));
        }
    }

    /// A running task is the one a person reaches for cancel on, so it may not be the slow case:
    /// the old 300s interval left a cancel queued for up to five minutes.
    #[test]
    fn a_running_task_is_asked_about_just_as_often_so_a_cancel_lands() {
        for state in [
            State::Running,
            State::Resuming,
            State::Switching,
            State::Verifying,
            State::Selecting,
            State::Armed,
            State::WaitingNetwork,
        ] {
            assert_eq!(interval(true, state, Backoff::new()), Some(BOUND));
        }
        assert!(BOUND <= Duration::from_secs(20), "a cancel must not queue");
    }

    /// The bug this pair exists for: cancelling lands in `Disabled`, where polling stops, so the
    /// receipt reporting the cancel would never leave. Without this, the phone spins forever.
    #[test]
    fn the_states_that_stop_polling_still_owe_the_phone_one_last_receipt() {
        for state in [State::Disabled, State::Stopped] {
            assert_eq!(interval(true, state, Backoff::new()), None);
            assert!(owes_closing_receipt(true, state, true));
        }
    }

    #[test]
    fn a_state_that_keeps_polling_owes_no_closing_receipt() {
        for state in [State::Running, State::NeedsHuman, State::Paused] {
            assert!(!owes_closing_receipt(true, state, true));
        }
    }

    /// The master switch is off: the path does not exist, and that includes the last receipt.
    #[test]
    fn nothing_is_owed_once_the_feature_is_switched_off() {
        for state in [State::Disabled, State::Stopped, State::Running] {
            assert!(!owes_closing_receipt(false, state, true));
        }
    }

    /// Nothing was handled, so there is no outcome to report and no receipt to owe. Without the
    /// flag, every dormant round would post one.
    #[test]
    fn a_state_that_stopped_on_its_own_owes_nothing() {
        for state in [State::Disabled, State::Stopped] {
            assert!(!owes_closing_receipt(true, state, false));
        }
    }

    /// The race this signature exists for. A user event is handed to the driver without waiting
    /// for it to be applied, so the round that handled the command may still read the state the
    /// command ended. Deciding then would drop the receipt; the flag carries it to the round
    /// where the state has settled.
    #[test]
    fn an_outcome_survives_a_state_that_has_not_settled_yet() {
        // The round that handled it: the cancel is in flight, the state still reads `Running`.
        assert!(!owes_closing_receipt(true, State::Running, true));
        // The next round, once the driver has applied it. The outcome is still owed, not lost.
        assert!(owes_closing_receipt(true, State::Disabled, true));
    }

    #[test]
    fn a_failing_bridge_is_backed_off_from_rather_than_hammered() {
        let mut backoff = Backoff::new();
        backoff = backoff.after_failure();
        let first = interval(true, State::NeedsHuman, backoff).expect("still polling");
        backoff = backoff.after_failure().after_failure();
        let later = interval(true, State::NeedsHuman, backoff).expect("still polling");

        assert!(first >= Duration::from_secs(30));
        assert!(later > first);
        assert_eq!(backoff.after_success().failures(), 0);
    }

    #[test]
    fn an_answer_with_no_command_yields_nothing_to_do() {
        assert_eq!(command_in(r#"{"v":1,"command":null}"#), None);
        assert_eq!(command_in(r#"{"v":1}"#), None);
    }

    #[test]
    fn a_command_is_handed_back_exactly_as_it_arrived_to_be_judged_elsewhere() {
        let body = r#"{"v":1,"command":{"v":1,"action":"resume","counter":7}}"#;
        let command = command_in(body).expect("a command");
        assert!(command.contains(r#""action":"resume""#));
        assert!(command.contains(r#""counter":7"#));
    }

    #[test]
    fn an_answer_this_build_cannot_read_is_simply_no_command() {
        for body in [
            "",
            "not json",
            "{}",
            r#"{"v":2,"command":{"action":"resume"}}"#,
            r#"{"v":1,"command":"resume"}"#,
        ] {
            let read = command_in(body);
            assert!(
                read.is_none() || read.as_deref() == Some("\"resume\""),
                "unreadable answers must not become commands: {body}"
            );
        }
    }

    #[test]
    fn an_address_toglet_may_not_post_to_is_refused_before_anything_is_built() {
        for endpoint in [
            "http://example.com/poll",
            "ftp://example.com",
            "https://example.com/a\r\nHost: evil",
            "",
        ] {
            let error = check_endpoint(endpoint).expect_err("should have been refused");
            assert_eq!(error.code(), ErrorCode::RemoteBridgeRejected);
        }
        assert!(check_endpoint("https://bridge.example.com/poll").is_ok());
        assert!(check_endpoint("http://127.0.0.1:8080/poll").is_ok());
    }

    /// Errors reach the interface and the log, so they must not quote the address.
    #[test]
    fn refusing_an_address_does_not_repeat_it() {
        let error = check_endpoint("http://secret-bridge.example.com/p").expect_err("refused");
        assert!(!format!("{error:?}").contains("secret-bridge"));
    }

    /// The set of files that may open outbound connections is a security boundary.
    #[test]
    fn only_three_files_in_the_whole_crate_build_an_http_client() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut builders = Vec::new();
        walk(&root, &mut |path, source| {
            if source.contains("reqwest::Client::builder")
                || source.contains("AsyncSmtpTransport::<Tokio1Executor>::relay")
            {
                builders.push(
                    path.strip_prefix(&root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        });
        builders.sort();
        assert_eq!(
            builders,
            vec!["notify/send.rs", "remote/poll.rs", "resets/fetch.rs"]
        );
    }

    fn walk(dir: &std::path::Path, visit: &mut impl FnMut(&std::path::Path, &str)) {
        let entries = std::fs::read_dir(dir).expect("the source tree is readable");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, visit);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).expect("a source file is readable");
                // Comments are stripped so a file may mention the rule without breaking it.
                let code = source
                    .lines()
                    .filter(|line| !line.trim_start().starts_with("//"))
                    .collect::<Vec<_>>()
                    .join("\n");
                visit(&path, &code);
            }
        }
    }
}
