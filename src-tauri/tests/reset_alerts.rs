//! Reset alerts end to end, as far as a test without a network can go: the parts are proven in
//! their own modules, and this checks that the application actually wires them together and
//! that the boundary the rules describe is the one the code has.

use toglet_lib::resets::{Markers, ResetStatus, Stats, announce, feed};

/// Unit tests prove each part correct but not that the application calls it; this checks that
/// start-up launches the poll loop and the loop reaches every part.
#[test]
fn the_poll_loop_is_started_and_reaches_every_part() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = std::fs::read_to_string(root.join("src/lib.rs")).expect("lib.rs is readable");
    assert!(
        lib.contains("commands::resets_poll::start("),
        "start-up must launch the poll loop, or every part below is unreachable"
    );
    for command in ["read_resets", "save_resets", "open_resets_site"] {
        assert!(
            lib.contains(&format!("commands::resets::{command}")),
            "{command} must be registered, or the interface cannot reach it"
        );
    }

    let loop_source =
        std::fs::read_to_string(root.join("src/commands/resets_poll.rs")).expect("readable");
    for call in [
        // Asks the feed, with the tag from the last fresh body.
        "resets::fetch(",
        // Decides what is new before the markers move on - after would announce nothing, ever.
        "resets.announcements(&status)",
        "resets.record_reading(",
        // Tells the interface, which turns a code into a sentence and a notification.
        "RESET_ANNOUNCED_EVENT",
        // Backs off, and obeys a feed that asks to be left alone.
        "resets::next_wait(",
    ] {
        assert!(
            loop_source.contains(call),
            "the poll loop should still call {call}"
        );
    }
}

/// The switch is the whole feature: off must mean the loop never reaches the request.
#[test]
fn the_loop_looks_at_the_switch_before_anything_else() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source =
        std::fs::read_to_string(root.join("src/commands/resets_poll.rs")).expect("readable");
    let switch = source.find(".enabled").expect("the loop reads the switch");
    let request = source
        .find("resets::fetch(")
        .expect("the loop makes the request");
    assert!(
        switch < request,
        "the switch must be checked before the request is made"
    );
}

/// The feed's own contract, as captured, through parse and dedupe: the first reading is quiet,
/// the same reading again is quiet, a new reset is one event.
#[test]
fn a_captured_reading_becomes_at_most_one_event_per_reset() {
    let body = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/codex_resets_status.json"),
    )
    .expect("the captured status body is readable");
    let first = feed::parse(&body).expect("the captured body parses");

    let (markers, events) = announce(&Markers::default(), &first);
    assert!(
        events.is_empty(),
        "the first reading is the current state, not news"
    );
    let (markers, events) = announce(&markers, &first);
    assert!(events.is_empty(), "the same reading again is not news");

    let mut second: ResetStatus = first.clone();
    let latest = second
        .latest_reset
        .as_mut()
        .expect("the capture has a latest reset");
    latest.id.push_str("-next");
    let (_, events) = announce(&markers, &second);
    assert_eq!(events.len(), 1);

    // The figures ride through untouched; a `null` would have stayed `None`.
    let Stats {
        total,
        days_since_last,
        ..
    } = first.stats;
    assert_eq!(total, 53);
    assert_eq!(days_since_last, Some(4.9));
}
