//! The envelopes that cross the bridge and the checks that decide whether a command is genuine.
//!
//! Pure: no network, clock, or files, so every acceptance decision is testable.

use serde::Deserialize;
use serde_json::json;

use super::mac;

/// Version prefix of every signed byte string; changing it breaks the wire format.
const PROTOCOL: &str = "toglet-remote/1";

/// Allowed clock skew in either direction. Generous because a command may queue at the bridge
/// while the machine sleeps; replay is stopped by the counter and nonce, not by this window.
pub const MAX_SKEW_SECONDS: i64 = 900;

/// Opaque identifiers and nonces are 16 bytes rendered as hex.
const ID_HEX_LEN: usize = 32;

/// Upper bound for code fields, so a bridge cannot make Toglet hold an arbitrary string.
const MAX_CODE_LEN: usize = 48;

/// The whole remote surface: argument-free actions mirroring existing panel buttons. Actions
/// with parameters (account, session, text) are deliberately not allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Resume,
    Pause,
    Cancel,
    /// Changes nothing; only asks for a receipt now rather than at the next poll.
    Status,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resume => "resume",
            Self::Pause => "pause",
            Self::Cancel => "cancel",
            Self::Status => "status",
        }
    }

    /// Public counterpart of [`parse`](Self::parse), for a code read back out of a file.
    pub fn parse_str(value: &str) -> Option<Self> {
        Self::parse(value)
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "resume" => Self::Resume,
            "pause" => Self::Pause,
            "cancel" => Self::Cancel,
            "status" => Self::Status,
            _ => return None,
        })
    }

    /// Whether the action is only valid against the state the phone observed. Only `resume`:
    /// pause and cancel mean the same in every state, so refusing them on drift would only hurt.
    fn needs_the_state_it_was_aimed_at(self) -> bool {
        matches!(self, Self::Resume)
    }
}

/// Why a command was refused, or that it was applied. Stable wire vocabulary echoed to the
/// phone; strings must not change once shipped.
///
/// Not `diagnostics::ErrorCode`: a refused envelope is expected behaviour, not an internal error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    /// A protocol version this build does not speak.
    Version,
    /// Not JSON, a field missing, a field of the wrong type, or one outside its bounds.
    Malformed,
    UnknownAction,
    BadMac,
    Expired,
    /// The counter did not advance (a replay).
    Replayed,
    /// The counter advanced but the nonce was already seen.
    NonceReused,
    /// Aimed at a different, replaced binding.
    SessionMismatch,
    /// Aimed at a state that is no longer current. Only `resume` can fail this way.
    StateChanged,
    /// Produced by `guard`, which holds the history; listed here to keep one vocabulary.
    RateLimited,
    /// Genuine and current, but the target is not running, so it was not carried out.
    /// Produced by the poller; never reported as `Applied`.
    Unavailable,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Version => "remote_version",
            Self::Malformed => "remote_malformed",
            Self::UnknownAction => "remote_unknown_action",
            Self::BadMac => "remote_bad_mac",
            Self::Expired => "remote_expired",
            Self::Replayed => "remote_replayed",
            Self::NonceReused => "remote_nonce_reused",
            Self::SessionMismatch => "remote_session_mismatch",
            Self::StateChanged => "remote_state_changed",
            Self::RateLimited => "remote_rate_limited",
            Self::Unavailable => "remote_unavailable",
        }
    }

    /// Parses a code read back from `remote.json`; unknown text is `None`.
    pub fn parse(code: &str) -> Option<Self> {
        Some(match code {
            "applied" => Self::Applied,
            "remote_version" => Self::Version,
            "remote_malformed" => Self::Malformed,
            "remote_unknown_action" => Self::UnknownAction,
            "remote_bad_mac" => Self::BadMac,
            "remote_expired" => Self::Expired,
            "remote_replayed" => Self::Replayed,
            "remote_nonce_reused" => Self::NonceReused,
            "remote_session_mismatch" => Self::SessionMismatch,
            "remote_state_changed" => Self::StateChanged,
            "remote_rate_limited" => Self::RateLimited,
            "remote_unavailable" => Self::Unavailable,
            _ => return None,
        })
    }
}

/// A command as it arrives, not yet verified.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    v: u32,
    /// Always `"command"`; also signed, so a receipt can never be read as a command.
    kind: String,
    action: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "observedState")]
    observed_state: String,
    counter: u64,
    nonce: String,
    #[serde(rename = "issuedAt")]
    issued_at: i64,
    mac: String,
}

/// A command that passed every check; the caller must persist its counter and nonce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub action: Action,
    pub counter: u64,
    pub nonce: String,
}

/// Everything the checks compare against, passed in to keep this module pure.
pub struct Context<'a> {
    pub secret: &'a [u8],
    /// The binding a command must name to be for this task.
    pub session_id: &'a str,
    /// The current `autorun` state, as `State::as_str()` renders it.
    pub state: &'a str,
    pub now: i64,
    /// The highest counter accepted so far.
    pub cursor: u64,
    pub recent_nonces: &'a [String],
}

/// Decides whether one command from the bridge is genuine and current.
///
/// Check order is a security property: nothing may be persisted before the MAC verifies, so the
/// counter to store is only returned inside [`Accepted`].
pub fn check(payload: &str, context: &Context<'_>) -> Result<Accepted, Outcome> {
    let raw: RawCommand = serde_json::from_str(payload).map_err(|_| Outcome::Malformed)?;

    if raw.v != 1 {
        return Err(Outcome::Version);
    }
    if raw.kind != "command" {
        return Err(Outcome::Malformed);
    }
    if !is_hex_id(&raw.session_id) || !is_hex_id(&raw.nonce) || !is_code(&raw.observed_state) {
        return Err(Outcome::Malformed);
    }
    let action = Action::parse(&raw.action).ok_or(Outcome::UnknownAction)?;

    let signed = canonical_command(
        action,
        &raw.session_id,
        &raw.observed_state,
        raw.counter,
        &raw.nonce,
        raw.issued_at,
    );
    if !mac::verify_hex(context.secret, signed.as_bytes(), &raw.mac) {
        return Err(Outcome::BadMac);
    }

    if (context.now - raw.issued_at).abs() > MAX_SKEW_SECONDS {
        return Err(Outcome::Expired);
    }
    if raw.counter <= context.cursor {
        return Err(Outcome::Replayed);
    }
    if context.recent_nonces.contains(&raw.nonce) {
        return Err(Outcome::NonceReused);
    }
    if raw.session_id != context.session_id {
        return Err(Outcome::SessionMismatch);
    }
    if action.needs_the_state_it_was_aimed_at() && raw.observed_state != context.state {
        return Err(Outcome::StateChanged);
    }

    Ok(Accepted {
        action,
        counter: raw.counter,
        nonce: raw.nonce,
    })
}

/// What Toglet reports to the bridge: only stable codes, opaque ids, counters and timestamps.
///
/// Never sentences, `threadId`, paths, account data, or quota figures; the phone renders text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub device_id: String,
    pub session_id: String,
    pub issued_at: i64,
    pub state: String,
    pub wait_reason: Option<String>,
    /// When the task is expected to be able to continue; `None` when unknown, never estimated.
    pub expected_available_at: Option<i64>,
    pub cursor: u64,
    /// Seconds until the next receipt, so the phone can tell when a state is stale; the
    /// cadence varies with the scheduler state and backoff.
    pub next_poll_seconds: u64,
    /// How many times the bound task has been continued.
    pub resume_count: u32,
    pub last_command: Option<LastCommand>,
}

/// The outcome of the most recent command, so the phone can confirm it was applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastCommand {
    pub counter: u64,
    pub action: Action,
    pub result: Outcome,
}

impl Receipt {
    /// The request body, signed with the shared secret.
    pub fn to_json(&self, secret: &[u8]) -> String {
        let signed = canonical_receipt(self);
        let mac = mac::sign_hex(secret, signed.as_bytes());
        let last = self
            .last_command
            .as_ref()
            .map_or(serde_json::Value::Null, |last| {
                json!({
                    "counter": last.counter,
                    "action": last.action.as_str(),
                    "result": last.result.as_str(),
                })
            });
        let body = json!({
            "v": 1,
            "kind": "receipt",
            "deviceId": self.device_id,
            "sessionId": self.session_id,
            "issuedAt": self.issued_at,
            "state": self.state,
            "waitReason": self.wait_reason,
            "expectedAvailableAt": self.expected_available_at,
            "cursor": self.cursor,
            "nextPollSeconds": self.next_poll_seconds,
            "resumeCount": self.resume_count,
            "lastCommand": last,
            "mac": mac,
        });
        // Every leaf is a string, a number, a bool or null, so serialising cannot fail.
        serde_json::to_string(&body).unwrap_or_else(|_| String::from("{}"))
    }
}

/// The bytes a command's MAC covers: fields in fixed order joined by newlines, because JSON
/// serialisations differ in key order, spacing and escaping across implementations.
fn canonical_command(
    action: Action,
    session_id: &str,
    observed_state: &str,
    counter: u64,
    nonce: &str,
    issued_at: i64,
) -> String {
    [
        PROTOCOL,
        "command",
        action.as_str(),
        session_id,
        observed_state,
        &counter.to_string(),
        nonce,
        &issued_at.to_string(),
    ]
    .join("\n")
}

fn canonical_receipt(receipt: &Receipt) -> String {
    let (last_counter, last_action, last_result) = match &receipt.last_command {
        Some(last) => (
            last.counter.to_string(),
            last.action.as_str().to_owned(),
            last.result.as_str().to_owned(),
        ),
        None => (String::new(), String::new(), String::new()),
    };
    [
        PROTOCOL,
        "receipt",
        &receipt.device_id,
        &receipt.session_id,
        &receipt.issued_at.to_string(),
        &receipt.state,
        // `None` signs as the empty string, which no present value renders as.
        receipt.wait_reason.as_deref().unwrap_or(""),
        &receipt
            .expected_available_at
            .map(|at| at.to_string())
            .unwrap_or_default(),
        &receipt.cursor.to_string(),
        &receipt.next_poll_seconds.to_string(),
        &receipt.resume_count.to_string(),
        &last_counter,
        &last_action,
        &last_result,
    ]
    .join("\n")
}

fn is_hex_id(value: &str) -> bool {
    value.len() == ID_HEX_LEN && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A stable code: bounded lower-case ASCII letters, digits and underscores.
fn is_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CODE_LEN
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"a shared secret the user typed into both ends";
    const SESSION: &str = "a83b4c1d9e0f2a3b4c5d6e7f80912233";
    const NONCE: &str = "9d2e0011223344556677889900aabbcc";
    const NOW: i64 = 1_757_664_000;

    fn context<'a>(nonces: &'a [String]) -> Context<'a> {
        Context {
            secret: SECRET,
            session_id: SESSION,
            state: "needs_human",
            now: NOW,
            cursor: 42,
            recent_nonces: nonces,
        }
    }

    /// Builds a well-formed, correctly signed command, so each test can spoil exactly one thing.
    fn command(
        action: &str,
        session: &str,
        observed: &str,
        counter: u64,
        issued_at: i64,
    ) -> String {
        let signed = [
            PROTOCOL,
            "command",
            action,
            session,
            observed,
            &counter.to_string(),
            NONCE,
            &issued_at.to_string(),
        ]
        .join("\n");
        let mac = mac::sign_hex(SECRET, signed.as_bytes());
        format!(
            r#"{{"v":1,"kind":"command","action":"{action}","sessionId":"{session}",
               "observedState":"{observed}","counter":{counter},"nonce":"{NONCE}",
               "issuedAt":{issued_at},"mac":"{mac}"}}"#
        )
    }

    fn good() -> String {
        command("resume", SESSION, "needs_human", 43, NOW)
    }

    #[test]
    fn a_well_formed_command_is_accepted_and_reports_what_to_persist() {
        let accepted = check(&good(), &context(&[])).expect("accepted");
        assert_eq!(
            accepted,
            Accepted {
                action: Action::Resume,
                counter: 43,
                nonce: NONCE.to_owned(),
            }
        );
    }

    #[test]
    fn a_version_this_build_does_not_speak_is_refused() {
        let payload = good().replace(r#""v":1"#, r#""v":2"#);
        assert_eq!(check(&payload, &context(&[])), Err(Outcome::Version));
    }

    #[test]
    fn anything_that_is_not_the_expected_shape_is_refused_as_malformed() {
        let cases = [
            String::from("not json at all"),
            String::from("{}"),
            // Unknown fields are refused, not ignored.
            good().replace(r#""v":1"#, r#""v":1,"extra":true"#),
            good().replace(SESSION, "short"),
            good().replace(NONCE, &"z".repeat(32)),
            good().replace("needs_human", "Needs Human"),
            good().replace("needs_human", &"x".repeat(MAX_CODE_LEN + 1)),
            good().replace(r#""counter":43"#, r#""counter":"43""#),
        ];
        for payload in cases {
            assert_eq!(
                check(&payload, &context(&[])),
                Err(Outcome::Malformed),
                "payload should have been malformed: {payload}"
            );
        }
    }

    #[test]
    fn an_action_outside_the_four_is_refused() {
        let payload = command("switch_account", SESSION, "needs_human", 43, NOW);
        assert_eq!(check(&payload, &context(&[])), Err(Outcome::UnknownAction));
    }

    #[test]
    fn changing_any_signed_field_invalidates_the_command() {
        // Each of these keeps the original signature while altering what it covers.
        let tampered = [
            good().replace(r#""action":"resume""#, r#""action":"cancel""#),
            good().replace(r#""counter":43"#, r#""counter":44"#),
            good().replace(&format!(r#""issuedAt":{NOW}"#), r#""issuedAt":1757664001"#),
            good().replace(NONCE, "00000000000000000000000000000000"),
        ];
        for payload in tampered {
            assert_eq!(check(&payload, &context(&[])), Err(Outcome::BadMac));
        }
    }

    #[test]
    fn a_signature_made_with_another_secret_is_refused() {
        let payload = good();
        let mut other = context(&[]);
        other.secret = b"a different secret";
        assert_eq!(check(&payload, &other), Err(Outcome::BadMac));
    }

    /// The machine may have been asleep, so fourteen minutes old is still current.
    #[test]
    fn a_command_issued_while_the_machine_slept_is_still_accepted() {
        let payload = command("resume", SESSION, "needs_human", 43, NOW - 840);
        assert!(check(&payload, &context(&[])).is_ok());
    }

    #[test]
    fn a_command_older_or_newer_than_the_window_is_refused() {
        for issued_at in [NOW - MAX_SKEW_SECONDS - 1, NOW + MAX_SKEW_SECONDS + 1] {
            let payload = command("resume", SESSION, "needs_human", 43, issued_at);
            assert_eq!(check(&payload, &context(&[])), Err(Outcome::Expired));
        }
    }

    #[test]
    fn a_counter_that_does_not_advance_is_a_replay() {
        for counter in [41, 42] {
            let payload = command("resume", SESSION, "needs_human", counter, NOW);
            assert_eq!(check(&payload, &context(&[])), Err(Outcome::Replayed));
        }
    }

    #[test]
    fn a_nonce_already_seen_is_refused_even_with_a_fresh_counter() {
        let seen = [NONCE.to_owned()];
        assert_eq!(check(&good(), &context(&seen)), Err(Outcome::NonceReused));
    }

    #[test]
    fn a_command_aimed_at_another_binding_is_refused() {
        let other = "ffffffffffffffffffffffffffffffff";
        let payload = command("resume", other, "needs_human", 43, NOW);
        assert_eq!(
            check(&payload, &context(&[])),
            Err(Outcome::SessionMismatch)
        );
    }

    #[test]
    fn continuing_a_task_that_has_already_moved_on_is_refused() {
        let payload = command("resume", SESSION, "paused", 43, NOW);
        assert_eq!(check(&payload, &context(&[])), Err(Outcome::StateChanged));
    }

    #[test]
    fn pausing_or_cancelling_survives_the_state_moving_on() {
        for action in ["pause", "cancel", "status"] {
            let payload = command(action, SESSION, "waiting_quota", 43, NOW);
            assert!(
                check(&payload, &context(&[])).is_ok(),
                "{action} should not depend on the state it was aimed at"
            );
        }
    }

    /// A counter is only ever reported on success, so failures cannot advance it.
    #[test]
    fn nothing_that_is_refused_reports_a_counter_to_persist() {
        let refused = [
            good().replace(r#""v":1"#, r#""v":9"#),
            String::from("{"),
            command("switch_account", SESSION, "needs_human", 44, NOW),
            good().replace(r#""counter":43"#, r#""counter":9001"#),
        ];
        for payload in refused {
            assert!(check(&payload, &context(&[])).is_err());
        }
    }

    #[test]
    fn a_receipt_carries_only_codes_and_is_signed_over_all_of_them() {
        let receipt = Receipt {
            device_id: "6f1c00112233445566778899aabbccdd".to_owned(),
            session_id: SESSION.to_owned(),
            issued_at: NOW,
            state: "needs_human".to_owned(),
            wait_reason: Some("waiting_on_human".to_owned()),
            expected_available_at: None,
            cursor: 42,
            next_poll_seconds: 20,
            resume_count: 0,
            last_command: Some(LastCommand {
                counter: 42,
                action: Action::Resume,
                result: Outcome::Applied,
            }),
        };
        let body = receipt.to_json(SECRET);

        assert!(body.contains(r#""state":"needs_human""#));
        assert!(body.contains(r#""expectedAvailableAt":null"#));
        assert!(body.contains(r#""result":"applied""#));

        let signed = canonical_receipt(&receipt);
        let mac = mac::sign_hex(SECRET, signed.as_bytes());
        assert!(body.contains(&mac));
    }

    #[test]
    fn a_receipt_has_no_sentence_no_path_and_no_quota_figure() {
        let receipt = Receipt {
            device_id: "6f1c00112233445566778899aabbccdd".to_owned(),
            session_id: SESSION.to_owned(),
            issued_at: NOW,
            state: "waiting_quota".to_owned(),
            wait_reason: Some("five_hour_exhausted".to_owned()),
            expected_available_at: Some(NOW + 5_040),
            cursor: 42,
            next_poll_seconds: 20,
            resume_count: 0,
            last_command: None,
        };
        let body = receipt.to_json(SECRET);

        for forbidden in [
            "threadId",
            "projectPath",
            "resumeInstruction",
            "percent",
            "/",
        ] {
            assert!(
                !body.contains(forbidden),
                "a receipt must not carry {forbidden}: {body}"
            );
        }
        assert!(body.contains(r#""lastCommand":null"#));
    }

    /// Absent fields render as empty in the signed bytes; that must not alias a present value.
    #[test]
    fn an_absent_field_signs_differently_from_a_present_one() {
        let base = Receipt {
            device_id: "6f1c00112233445566778899aabbccdd".to_owned(),
            session_id: SESSION.to_owned(),
            issued_at: NOW,
            state: "running".to_owned(),
            wait_reason: None,
            expected_available_at: None,
            cursor: 7,
            next_poll_seconds: 20,
            resume_count: 0,
            last_command: None,
        };
        let mut with_reason = base.clone();
        with_reason.wait_reason = Some("network".to_owned());

        assert_ne!(canonical_receipt(&base), canonical_receipt(&with_reason));
    }

    #[test]
    fn every_outcome_has_a_distinct_string_that_reads_back() {
        let all = [
            Outcome::Applied,
            Outcome::Version,
            Outcome::Malformed,
            Outcome::UnknownAction,
            Outcome::BadMac,
            Outcome::Expired,
            Outcome::Replayed,
            Outcome::NonceReused,
            Outcome::SessionMismatch,
            Outcome::StateChanged,
            Outcome::RateLimited,
            Outcome::Unavailable,
        ];
        let mut seen = Vec::new();
        for outcome in all {
            let code = outcome.as_str();
            assert!(!seen.contains(&code), "duplicate outcome string {code}");
            assert_eq!(Outcome::parse(code), Some(outcome));
            seen.push(code);
        }
        assert_eq!(Outcome::parse("something_else"), None);
    }

    #[test]
    fn every_action_has_a_distinct_string_that_reads_back() {
        for action in [
            Action::Resume,
            Action::Pause,
            Action::Cancel,
            Action::Status,
        ] {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
        assert_eq!(Action::parse("resume_all"), None);
    }
}
