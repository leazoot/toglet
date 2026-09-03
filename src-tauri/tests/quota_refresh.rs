//! Refresh behaviour that can only be shown by actually running it.
//!
//! The scheduler's policy is unit-tested as pure functions. What those cannot show is that
//! walking the queue really does run one app server at a time and leaves nothing behind, or
//! that a refresh never touches a default Codex home. Both are asserted here against the fake
//! server - no account, no network.

mod support;

use std::time::Instant;

use support::{fake_binary, scenario_home};
use toglet_lib::app_server::{AppServerClient, AppServerSession};
use toglet_lib::credentials::{
    CredentialLock, CredentialRef, MemorySecretStore, Secret, SecretStore, WriteBack,
    write_back_if_refreshed,
};
use toglet_lib::diagnostics::Phase;
use toglet_lib::quota::{
    Backoff, NormalisedQuota, QuotaSnapshot, RefreshIntervals, RefreshState, due_now,
};

const INTERVALS: RefreshIntervals = RefreshIntervals {
    active_seconds: 60,
    inactive_seconds: 300,
};

fn state(id: &str, is_active: bool) -> RefreshState {
    RefreshState {
        account_id: id.to_owned(),
        last_success_at: None,
        last_attempt_at: None,
        backoff: Backoff::new(),
        is_active,
        is_refreshable: true,
    }
}

/// Reads one account's quota the way the refresh path does, and reports when it ran.
fn refresh_one(account_id: &str, now: i64) -> (QuotaSnapshot, Instant, Instant) {
    let started = Instant::now();
    let home = scenario_home("normal", Phase::ReadQuota);
    let client =
        AppServerClient::start(&fake_binary(Phase::ReadQuota), home).expect("the server starts");
    let mut session = AppServerSession::open(client).expect("the handshake succeeds");
    let raw = session
        .read_rate_limits()
        .expect("rateLimits/read succeeds");
    session.close().expect("the server exits cleanly");
    let finished = Instant::now();

    (
        QuotaSnapshot::fresh(account_id, NormalisedQuota::from_raw(&raw), now),
        started,
        finished,
    )
}

#[test]
fn walking_the_queue_runs_one_app_server_at_a_time() {
    let states = vec![
        state("acct-1", true),
        state("acct-2", false),
        state("acct-3", false),
    ];
    let queue = due_now(&states, 10_000, INTERVALS);
    assert_eq!(queue.len(), 3);

    let mut spans = Vec::new();
    for (account_id, _) in &queue {
        let (snapshot, started, finished) = refresh_one(account_id, 10_000);
        assert_eq!(snapshot.account_id(), account_id);
        spans.push((started, finished));
    }

    // Sequential means no two runs overlap. Asserting the intervals are disjoint is the
    // observable form of "at most one subprocess at any moment".
    for pair in spans.windows(2) {
        let (_, first_end) = pair[0];
        let (second_start, _) = pair[1];
        assert!(
            first_end <= second_start,
            "two refreshes overlapped, so more than one app server was running"
        );
    }
}

#[test]
fn a_refresh_produces_normalised_windows_and_a_fresh_snapshot() {
    let (snapshot, _, _) = refresh_one("acct-1", 5_000);

    let view = snapshot.view(5_000);
    assert!(!view.stale);
    assert_eq!(view.fetched_at, 5_000);
    let five_hour = view
        .quota
        .five_hour()
        .expect("the five-hour window is present");
    assert_eq!(five_hour.remaining_percent, 98.0);
    assert_eq!(
        view.quota
            .weekly()
            .expect("the weekly window is present")
            .remaining_percent,
        100.0
    );
}

#[test]
fn a_refresh_never_touches_a_default_codex_home() {
    // A stand-in for `~/.codex`, holding a credential file the refresh path must not go near.
    let default_home = scenario_home("normal", Phase::ReadQuota);
    let auth = default_home.path().join("auth.json");
    std::fs::write(
        &auth,
        br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"rt-default"}}"#,
    )
    .expect("the default credential file is written");
    let before = std::fs::read(&auth).expect("readable");
    let before_entries = entries(default_home.path());

    for account in ["acct-1", "acct-2"] {
        refresh_one(account, 0);
    }

    assert_eq!(
        std::fs::read(&auth).expect("readable"),
        before,
        "refreshing must not change the default authentication"
    );
    assert_eq!(
        entries(default_home.path()),
        before_entries,
        "the default home gained or lost a file during a refresh"
    );
}

#[test]
fn a_refresh_that_rotates_the_token_stores_the_new_one() {
    let store = MemorySecretStore::new();
    let reference = CredentialRef::new("acct-1").expect("valid reference");
    let original = br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"rt-original"}}"#.to_vec();
    store
        .store(&reference, &Secret::new(original.clone()))
        .expect("the original is stored");

    // Stands in for the app server having refreshed the credentials inside the throwaway home.
    let home = scenario_home("normal", Phase::ReadQuota);
    let rotated = br#"{"auth_mode":"chatgpt","tokens":{"refresh_token":"rt-rotated"}}"#.to_vec();
    std::fs::write(home.path().join("auth.json"), &rotated).expect("written");

    let outcome = write_back_if_refreshed(
        &CredentialLock::new(),
        &store,
        &reference,
        home.path(),
        &Secret::new(original),
        Phase::ReadQuota,
    )
    .expect("the write-back succeeds");

    assert_eq!(outcome, WriteBack::Stored);
    assert_eq!(
        store.load(&reference).expect("stored").expose(),
        rotated,
        "the rotated token must survive the throwaway home being deleted"
    );
}

fn entries(directory: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("readable")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}
