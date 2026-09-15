//! The app server responses Toglet has to survive, driven by the fake server.

mod support;

use std::time::Duration;

use support::start_scenario;
use toglet_lib::accounts::AccountIdentity;
use toglet_lib::app_server::{
    AppServerSession, ServerEvent, ThreadStatus, TurnErrorKind, TurnRecord, TurnStatus,
};
use toglet_lib::diagnostics::{ErrorCode, Phase};

/// A missing event fails as a timeout rather than hanging the suite.
const EVENT_TIMEOUT: Duration = Duration::from_secs(5);

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

    // A server-defined code of unknown meaning is kept for diagnosis rather than guessed as
    // `auth_expired`.
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
        "usage_limit_turn",
        "turn_completed",
        "waiting_on_user_input",
        "unauthorized_turn",
        "network_turn",
        "resume_rejected",
        "multi_thread",
        "turn_in_progress",
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

/// Reads events until the turn finishes. Status changes and unrelated notifications on the
/// way are expected; anything else is a failure of the scenario.
fn completed_turn(session: &mut AppServerSession) -> TurnRecord {
    loop {
        match session.next_event(EVENT_TIMEOUT).expect("an event arrives") {
            ServerEvent::TurnCompleted { thread_id, turn } => {
                assert_eq!(thread_id, "01a09301-0000-7000-8000-000000000001");
                return turn;
            }
            ServerEvent::ThreadStatusChanged { .. } | ServerEvent::Other { .. } => {}
            ServerEvent::ServerRequest { method } => {
                panic!("the server asked `{method}` while a completion was expected")
            }
        }
    }
}

#[test]
fn a_thread_list_is_filtered_by_project_folder() {
    let mut session = session("multi_thread");

    let all = session.list_threads(None).expect("thread/list succeeds");
    let one = session
        .list_threads(Some(std::path::Path::new("/fake/project-b")))
        .expect("a filtered thread/list succeeds");

    assert_eq!(all.threads.len(), 3);
    let folders: std::collections::BTreeSet<_> =
        all.threads.iter().filter_map(|t| t.folder_name()).collect();
    assert_eq!(folders.len(), 3, "three distinct projects were listed");
    assert!(!all.truncated);
    assert_eq!(one.threads.len(), 1);
    assert_eq!(one.threads[0].title.as_deref(), Some("Second project"));
    assert!(
        one.threads[0].turns.is_empty(),
        "a listing carries no turns; only a read does"
    );
    session.close().expect("the server exits cleanly");
}

#[test]
fn an_exhausted_session_is_readable_resumable_and_fails_the_same_way_again() {
    let mut session = session("usage_limit_turn");
    let thread_id = "01a09301-0000-7000-8000-000000000001";

    // Read before touching anything: the reason the desktop app stopped is on the last turn.
    let read = session
        .read_thread(thread_id)
        .expect("thread/read succeeds");
    let last = read.last_turn().expect("turns were loaded");
    assert_eq!(last.status, TurnStatus::Failed);
    assert_eq!(last.error, Some(TurnErrorKind::UsageLimitExceeded));
    assert_eq!(read.status, ThreadStatus::NotLoaded);

    let resumed = session
        .resume_thread(thread_id)
        .expect("thread/resume succeeds");
    assert_eq!(resumed.status, ThreadStatus::Idle);
    assert_eq!(resumed.turns.len(), 2);

    let started = session
        .start_turn(thread_id, "continue")
        .expect("turn/start is accepted");
    assert_eq!(started.status, TurnStatus::InProgress);

    // The completion arrives while another request is in flight; it must be kept, not lost.
    session
        .read_thread(thread_id)
        .expect("a read while the turn runs succeeds");

    let turn = completed_turn(&mut session);
    assert_eq!(turn.id, started.id);
    assert_eq!(turn.status, TurnStatus::Failed);
    assert_eq!(turn.error, Some(TurnErrorKind::UsageLimitExceeded));
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_turn_that_finishes_reports_completed_with_no_error() {
    let mut session = session("turn_completed");

    session
        .resume_thread("01a09301-0000-7000-8000-000000000001")
        .expect("thread/resume succeeds");
    session
        .start_turn("01a09301-0000-7000-8000-000000000001", "continue")
        .expect("turn/start is accepted");

    let turn = completed_turn(&mut session);
    assert_eq!(turn.status, TurnStatus::Completed);
    assert_eq!(turn.error, None);
    assert!(turn.completed_at.is_some());
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_question_from_the_model_surfaces_as_a_request_and_can_be_interrupted() {
    let mut session = session("waiting_on_user_input");
    let thread_id = "01a09301-0000-7000-8000-000000000001";

    let started = session
        .start_turn(thread_id, "continue")
        .expect("turn/start is accepted");

    let status = session
        .next_event(EVENT_TIMEOUT)
        .expect("a status change arrives");
    let ServerEvent::ThreadStatusChanged { status, .. } = status else {
        panic!("expected a status change, got {status:?}");
    };
    assert!(status.is_waiting_on_human());

    let request = session
        .next_event(EVENT_TIMEOUT)
        .expect("the request arrives");
    assert_eq!(
        request,
        ServerEvent::ServerRequest {
            method: "item/tool/requestUserInput".to_owned()
        }
    );
    assert!(
        !format!("{request:?}").contains("Which one?"),
        "the question's content is not kept"
    );

    // Nothing else arrives: the turn is genuinely blocked on a person.
    let idle = session
        .next_event(Duration::from_millis(300))
        .expect_err("a blocked turn sends nothing");
    assert_eq!(idle.code(), ErrorCode::AppServerUnresponsive);

    session
        .interrupt_turn(thread_id, &started.id)
        .expect("turn/interrupt is accepted");
    let turn = completed_turn(&mut session);
    assert_eq!(turn.status, TurnStatus::Interrupted);
    session.close().expect("the server exits cleanly");
}

#[test]
fn refused_credentials_end_the_turn_as_unauthorized() {
    let mut session = session("unauthorized_turn");

    session
        .start_turn("01a09301-0000-7000-8000-000000000001", "continue")
        .expect("turn/start is accepted");

    let turn = completed_turn(&mut session);
    assert_eq!(turn.status, TurnStatus::Failed);
    assert_eq!(turn.error, Some(TurnErrorKind::Unauthorized));
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_transport_failure_ends_the_turn_as_a_network_error_with_its_status() {
    let mut session = session("network_turn");

    session
        .start_turn("01a09301-0000-7000-8000-000000000001", "continue")
        .expect("turn/start is accepted");

    let turn = completed_turn(&mut session);
    assert_eq!(turn.status, TurnStatus::Failed);
    assert_eq!(
        turn.error,
        Some(TurnErrorKind::Network {
            http_status: Some(502)
        })
    );
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_session_the_server_cannot_serve_is_unavailable_not_incompatible() {
    let mut session = session("resume_rejected");
    let thread_id = "01a09301-0000-7000-8000-000000000001";

    let read = session
        .read_thread(thread_id)
        .expect_err("the thread cannot be read");
    let resume = session
        .resume_thread(thread_id)
        .expect_err("the thread cannot be resumed");

    for error in [read, resume] {
        assert_eq!(error.code(), ErrorCode::ThreadUnavailable);
        assert!(!error.retryable(), "retrying cannot bring the session back");
        assert!(
            !error.to_string().contains(thread_id),
            "the thread id from the server's message must not be carried"
        );
    }
    session.close().expect("the server exits cleanly");
}

#[test]
fn a_running_turn_is_visible_before_anything_is_sent() {
    let mut session = session("turn_in_progress");
    let thread_id = "01a09301-0000-7000-8000-000000000001";

    let read = session
        .read_thread(thread_id)
        .expect("thread/read succeeds");

    // This check must happen before a switch or a continuation.
    assert_eq!(
        read.last_turn().expect("turns were loaded").status,
        TurnStatus::InProgress
    );
    assert_eq!(read.status, ThreadStatus::Active(vec![]));
    assert!(!read.status.is_waiting_on_human());

    // Starting anyway is refused by the turn itself, as the schema describes.
    session
        .start_turn(thread_id, "continue")
        .expect("turn/start is accepted at the protocol level");
    let turn = completed_turn(&mut session);
    assert_eq!(turn.error, Some(TurnErrorKind::ActiveTurnNotSteerable));
    session.close().expect("the server exits cleanly");
}

#[test]
fn the_model_catalogue_is_read_for_display_only() {
    let mut session = session("normal");

    let models = session.list_models().expect("model/list succeeds");

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].display_name, "GPT Fake");
    assert!(models[0].is_default);
    session.close().expect("the server exits cleanly");
}
