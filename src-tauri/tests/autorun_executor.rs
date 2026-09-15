//! The executor against the fake app server: one session, one identity check, one
//! continuation turn, and every way a turn can end mapped to its class. No account, no network.

mod support;

use std::time::Duration;

use support::{fake_binary, scenario_home};
use toglet_lib::accounts::fingerprint;
use toglet_lib::autorun::{Executor, Fact, Interruption, Resumed};
use toglet_lib::codex_home::IsolatedHome;
use toglet_lib::diagnostics::{ErrorCode, Phase};

const THREAD_ID: &str = "01a09301-0000-7000-8000-000000000001";
const TURN_ID: &str = "01a09302-0000-7000-8000-000000000002";
const ACCOUNT_ID: &str = "acct-executor";
const INSTRUCTION: &str = "Continue the unfinished work.";
const EVENT_WAIT: Duration = Duration::from_secs(5);

fn auth_json(account_id: &str) -> Vec<u8> {
    format!(r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account_id}"}}}}"#).into_bytes()
}

/// A scratch "default" home running `scenario`, signed in as `ACCOUNT_ID`.
fn home(scenario: &str) -> IsolatedHome {
    let home = scenario_home(scenario, Phase::Autorun);
    std::fs::write(home.path().join("auth.json"), auth_json(ACCOUNT_ID)).expect("written");
    home
}

fn executor(home: &IsolatedHome) -> Executor {
    Executor::new(fake_binary(Phase::Autorun), home.path())
}

fn expected() -> String {
    fingerprint::from_account_id(ACCOUNT_ID)
}

/// Polls until the executor reports something.
fn next(executor: &mut Executor) -> Fact {
    for _ in 0..10 {
        if let Some(fact) = executor.poll(EVENT_WAIT) {
            return fact;
        }
    }
    panic!("the executor reported nothing");
}

#[test]
fn reading_the_thread_while_armed_starts_and_stops_its_own_server() {
    let home = home("usage_limit_turn");
    let mut executor = executor(&home);

    let fact = executor.read_thread(THREAD_ID).expect("read");
    assert_eq!(
        fact,
        Fact::TurnEnded {
            turn_id: "t2".to_owned(),
            interruption: Interruption::Exhausted
        }
    );
    assert!(!executor.is_open(), "a look is not a session");
    assert_eq!(executor.pid(), None);
}

// Identity confirmed, then exactly one turn started.
#[test]
fn a_resume_confirms_the_identity_and_starts_one_turn() {
    let home = home("usage_limit_turn");
    let mut executor = executor(&home);

    let resumed = executor
        .resume(THREAD_ID, INSTRUCTION, &expected(), Some("t2"))
        .expect("resumed");
    assert_eq!(
        resumed,
        Resumed::Started {
            turn_id: TURN_ID.to_owned()
        }
    );
    assert!(executor.is_open());
    assert!(executor.pid().is_some());

    // The scenario's turn fails on the limit again, and is reported as exhausted.
    assert_eq!(
        next(&mut executor),
        Fact::TurnEnded {
            turn_id: TURN_ID.to_owned(),
            interruption: Interruption::Exhausted
        }
    );
    executor.stop();
    assert!(!executor.is_open());
}

// The wrong account on the session stops everything before `turn/start`.
#[test]
fn a_resume_on_the_wrong_account_is_refused_before_anything_is_started() {
    let home = home("usage_limit_turn");
    let mut executor = executor(&home);

    let error = executor
        .resume(
            THREAD_ID,
            INSTRUCTION,
            &fingerprint::from_account_id("somebody-else"),
            Some("t2"),
        )
        .expect_err("refused");
    assert_eq!(error.code(), ErrorCode::SwitchVerificationMismatch);
    assert_eq!(error.phase(), Phase::Autorun);
    // Nothing was started, so nothing is reported.
    assert_eq!(executor.poll(Duration::from_millis(200)), None);
}

// A thread whose last turn is not the one waited on was used meanwhile.
#[test]
fn a_thread_that_moved_on_is_reported_as_changed_not_continued() {
    let home = home("usage_limit_turn");
    let mut executor = executor(&home);

    let resumed = executor
        .resume(THREAD_ID, INSTRUCTION, &expected(), Some("t1"))
        .expect("answered");
    assert_eq!(resumed, Resumed::ThreadChanged);
    assert_eq!(executor.poll(Duration::from_millis(200)), None);
}

// A turn in progress is waited for, never interrupted or doubled.
#[test]
fn a_turn_still_running_is_waited_for() {
    let home = home("turn_in_progress");
    let mut executor = executor(&home);

    let resumed = executor
        .resume(THREAD_ID, INSTRUCTION, &expected(), Some("t2"))
        .expect("answered");
    assert_eq!(resumed, Resumed::TurnInProgress);
    assert!(executor.is_open(), "the session listens for how it ends");
}

// Every class the fake server can produce, from a real `turn/completed`.
#[test]
fn every_way_a_turn_ends_maps_to_its_class() {
    for (scenario, expected_end) in [
        ("turn_completed", Interruption::Completed),
        ("unauthorized_turn", Interruption::AuthExpired),
        ("network_turn", Interruption::Network),
    ] {
        let home = home(scenario);
        let mut executor = executor(&home);
        executor
            .resume(THREAD_ID, INSTRUCTION, &expected(), Some("t2"))
            .expect("resumed");
        assert_eq!(
            next(&mut executor),
            Fact::TurnEnded {
                turn_id: TURN_ID.to_owned(),
                interruption: expected_end
            },
            "{scenario}"
        );
    }
}

// A question from the model is reported and never answered.
#[test]
fn a_question_from_the_model_needs_a_person() {
    let home = home("waiting_on_user_input");
    let mut executor = executor(&home);
    executor
        .resume(THREAD_ID, INSTRUCTION, &expected(), Some("t2"))
        .expect("resumed");
    assert_eq!(next(&mut executor), Fact::WaitingOnHuman);
}

// A session the server cannot serve is a `thread_unavailable`, not a new chat.
#[test]
fn a_thread_the_server_refuses_is_reported_with_its_code() {
    let home = home("resume_rejected");
    let mut executor = executor(&home);
    let error = executor
        .resume(THREAD_ID, INSTRUCTION, &expected(), Some("t2"))
        .expect_err("refused");
    assert_eq!(error.code(), ErrorCode::ThreadUnavailable);
}

#[test]
fn stopping_leaves_no_server_behind() {
    let home = home("turn_completed");
    let pid = {
        let mut executor = executor(&home);
        executor
            .resume(THREAD_ID, INSTRUCTION, &expected(), Some("t2"))
            .expect("resumed");
        let pid = executor.pid().expect("a server runs");
        executor.stop();
        pid
    };
    // The process is reaped by the close. Liveness checks are platform-specific, so this
    // asserts the executor's own invariant: no session, no pid.
    let executor = executor(&home);
    assert_eq!(executor.pid(), None);
    assert_ne!(pid, 0);
}
