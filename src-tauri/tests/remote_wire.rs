//! The wire format, checked against fixtures signed by an independent implementation.
//!
//! Any disagreement on the canonical signed bytes would make Toglet refuse every real phone
//! command, with `remote_bad_mac` as the only symptom.
//! Regenerate with `node examples/remote-bridge/e2e.mjs`.

use serde::Deserialize;
use toglet_lib::remote::envelope::{self, Action, Context, LastCommand, Outcome, Receipt};

#[derive(Deserialize)]
struct Fixtures {
    secret: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "deviceId")]
    device_id: String,
    #[serde(rename = "issuedAt")]
    issued_at: i64,
    genuine: serde_json::Value,
    forged: serde_json::Value,
    receipt: serde_json::Value,
    #[serde(rename = "receiptWaiting")]
    receipt_waiting: serde_json::Value,
}

fn fixtures() -> Fixtures {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/remote_wire.json");
    let text = std::fs::read_to_string(path).expect("run `node examples/remote-bridge/e2e.mjs`");
    serde_json::from_str(&text).expect("the fixtures parse")
}

fn context<'a>(fixtures: &'a Fixtures, state: &'a str, nonces: &'a [String]) -> Context<'a> {
    Context {
        secret: fixtures.secret.as_bytes(),
        session_id: &fixtures.session_id,
        state,
        now: fixtures.issued_at,
        cursor: 42,
        recent_nonces: nonces,
    }
}

#[test]
fn a_command_signed_by_the_reference_page_is_accepted_by_toglet() {
    let fixtures = fixtures();
    let payload = fixtures.genuine.to_string();

    let accepted = envelope::check(&payload, &context(&fixtures, "needs_human", &[]))
        .expect("the two implementations must agree on the bytes a signature covers");

    assert_eq!(accepted.action, Action::Resume);
    assert_eq!(accepted.counter, 43);
}

#[test]
fn a_command_whose_signature_was_altered_is_refused() {
    let fixtures = fixtures();
    let payload = fixtures.forged.to_string();

    assert_eq!(
        envelope::check(&payload, &context(&fixtures, "needs_human", &[])),
        Err(Outcome::BadMac)
    );
}

/// The counter refuses a replayed envelope, checked against a real signed command.
#[test]
fn the_reference_command_cannot_be_played_twice() {
    let fixtures = fixtures();
    let payload = fixtures.genuine.to_string();

    let mut after = context(&fixtures, "needs_human", &[]);
    after.cursor = 43;
    assert_eq!(envelope::check(&payload, &after), Err(Outcome::Replayed));
}

/// Toglet's receipt must serialise to exactly what the reference bridge relayed, MAC included.
#[test]
fn toglet_builds_the_same_receipt_the_reference_implementation_did() {
    let fixtures = fixtures();

    let receipt = Receipt {
        device_id: fixtures.device_id.clone(),
        session_id: fixtures.session_id.clone(),
        issued_at: fixtures.issued_at,
        state: "needs_human".to_owned(),
        wait_reason: Some("waiting_on_human".to_owned()),
        expected_available_at: None,
        cursor: 42,
        next_poll_seconds: 20,
        resume_count: 3,
        last_command: Some(LastCommand {
            counter: 42,
            action: Action::Resume,
            result: Outcome::Applied,
        }),
    };

    let built: serde_json::Value =
        serde_json::from_str(&receipt.to_json(fixtures.secret.as_bytes())).expect("valid json");

    assert_eq!(
        built["mac"], fixtures.receipt["mac"],
        "the signed bytes differ"
    );
    assert_eq!(built["state"], fixtures.receipt["state"]);
    assert_eq!(built["lastCommand"], fixtures.receipt["lastCommand"]);
}

/// Optional fields render specially in the signed bytes, and a mistake only shows once a task
/// waits on quota, so that case has its own fixture.
#[test]
fn a_receipt_carrying_a_recovery_time_also_matches() {
    let fixtures = fixtures();

    let receipt = Receipt {
        device_id: fixtures.device_id.clone(),
        session_id: fixtures.session_id.clone(),
        issued_at: fixtures.issued_at,
        state: "waiting_quota".to_owned(),
        wait_reason: Some("five_hour_exhausted".to_owned()),
        expected_available_at: Some(fixtures.issued_at + 5_040),
        cursor: 42,
        next_poll_seconds: 20,
        resume_count: 3,
        last_command: None,
    };

    let built: serde_json::Value =
        serde_json::from_str(&receipt.to_json(fixtures.secret.as_bytes())).expect("valid json");

    assert_eq!(built["mac"], fixtures.receipt_waiting["mac"]);
    assert_eq!(
        built["expectedAvailableAt"],
        fixtures.receipt_waiting["expectedAvailableAt"]
    );
    assert_eq!(built["lastCommand"], serde_json::Value::Null);
}
