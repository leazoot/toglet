//! One of the only two files that open outbound connections (with `notify::send`).
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

/// Poll interval while the user is likely about to act (the states the phone is notified about).
const ATTENTIVE: Duration = Duration::from_secs(20);

/// Poll interval while the task runs, kept only so a remote pause still works.
const BACKGROUND: Duration = Duration::from_secs(300);

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
    if matches!(state, State::Disabled | State::Stopped) {
        return None;
    }
    if backoff.failures() > 0 {
        return Some(backoff.delay());
    }
    let attentive = matches!(
        state,
        State::NeedsHuman | State::Paused | State::WaitingQuota | State::RoundCompleted
    );
    Some(if attentive { ATTENTIVE } else { BACKGROUND })
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
            assert_eq!(interval(true, state, Backoff::new()), Some(ATTENTIVE));
        }
    }

    #[test]
    fn a_running_task_is_asked_about_rarely() {
        for state in [
            State::Running,
            State::Resuming,
            State::Switching,
            State::Verifying,
            State::Selecting,
            State::Armed,
            State::WaitingNetwork,
        ] {
            assert_eq!(interval(true, state, Backoff::new()), Some(BACKGROUND));
        }
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
    fn only_two_files_in_the_whole_crate_build_an_http_client() {
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
        assert_eq!(builders, vec!["notify/send.rs", "remote/poll.rs"]);
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
