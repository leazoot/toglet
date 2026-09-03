//! The app server responses Toglet has to survive, driven by the fake server.
//!
//! None of these need an account, a credential store or the network. Every one of them exists
//! because the real server cannot be asked to fail on demand.

mod support;

use std::time::Duration;

use support::start_scenario;
use toglet_lib::accounts::AccountIdentity;
use toglet_lib::app_server::AppServerSession;
use toglet_lib::diagnostics::{ErrorCode, Phase};

fn session(scenario: &str) -> AppServerSession {
    AppServerSession::open(start_scenario(scenario)).expect("the handshake succeeds")
}

#[test]
fn a_normal_exchange_returns_the_account_and_both_windows() {
    let mut session = session("normal");

    let account = session.read_account().expect("account/read succeeds");
    let limits = session
        .read_rate_limits()
        .expect("account/rateLimits/read succeeds");

    assert_eq!(
        account,
        Some(AccountIdentity::Chatgpt {
            email: "tester@example.com".to_owned(),
            plan_type: Some("plus".to_owned()),
        })
    );
    let primary = limits.primary.expect("the five-hour window is present");
    assert_eq!(primary.used_percent, 2.0);
    assert_eq!(primary.window_duration_mins, Some(300));
    assert_eq!(
        limits
            .secondary
            .expect("the weekly window is present")
            .window_duration_mins,
        Some(10080)
    );
    session.close().expect("the server exits cleanly");
}

#[test]
fn fields_this_build_has_never_heard_of_are_ignored() {
    let mut session = session("unknown_fields");

    let account = session.read_account().expect("account/read still succeeds");
    let limits = session
        .read_rate_limits()
        .expect("rateLimits/read still succeeds");

    assert_eq!(
        account.and_then(|account| account.email().map(str::to_owned)),
        Some("tester@example.com".to_owned())
    );
    assert_eq!(
        limits.primary.expect("primary is present").used_percent,
        2.0,
        "new fields must not disturb the values Toglet does understand"
    );
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_missing_required_field_is_an_error_and_never_a_zero() {
    let mut session = session("missing_field");

    let error = session
        .read_rate_limits()
        .expect_err("a window without usedPercent must be refused");

    assert_eq!(error.code(), ErrorCode::RuntimeIncompatible);
    assert_eq!(error.phase(), Phase::ReadQuota);
    session.close().expect("the server exits cleanly");
}

#[test]
fn an_error_response_is_surfaced_rather_than_reported_as_success() {
    let mut session = session("unauthorized");

    let error = session
        .read_account()
        .expect_err("a 401 must not come back as an account");

    // A server-defined code. Toglet does not yet have evidence for what each one means, so it
    // reports the failure with the code kept for diagnosis instead of guessing `auth_expired`.
    assert_eq!(error.code(), ErrorCode::Internal);
    session.close().expect("the server exits cleanly");
}

#[test]
fn an_abnormal_exit_maps_to_a_stable_code() {
    let mut session = session("crash");

    let error = session
        .read_account()
        .expect_err("a server that dies mid-request must not look like success");

    assert_eq!(error.code(), ErrorCode::AppServerCrashed);
    // Closing reports the non-zero exit rather than pretending the shutdown was clean.
    assert!(session.close().is_err());
}

#[test]
fn a_slow_reply_is_waited_for_rather_than_misreported() {
    let mut session = session("slow");

    let limits = session
        .read_rate_limits()
        .expect("a slow but valid reply is a success");

    assert_eq!(
        limits.primary.expect("primary is present").used_percent,
        2.0
    );
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_server_that_never_answers_hits_the_deadline_instead_of_hanging() {
    let mut client = start_scenario("timeout");
    client
        .send_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .expect("the request is written");

    let error = client
        .recv_line(Duration::from_millis(500))
        .expect_err("an unanswered request must not block forever");

    assert_eq!(error.code(), ErrorCode::AppServerUnresponsive);
    assert!(error.retryable());
    client.shutdown().expect("the server still exits cleanly");
}

#[test]
fn every_scenario_leaves_no_isolated_home_behind() {
    let mut homes = Vec::new();
    for scenario in [
        "normal",
        "unknown_fields",
        "missing_field",
        "unauthorized",
        "slow",
    ] {
        let session = session(scenario);
        homes.push(session.home_path());
        // Dropped without an explicit close, which is the path a panicking caller takes.
        drop(session);
    }

    for home in homes {
        assert!(!home.exists(), "an isolated home survived its session");
    }
}
