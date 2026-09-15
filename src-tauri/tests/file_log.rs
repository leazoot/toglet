//! The log file, installed the way the application installs it at start-up.
//!
//! A separate test binary because the sink is process-wide and installed once.

use toglet_lib::codex_home::{IsolatedHome, is_private};
use toglet_lib::commands::state::install_file_log;
use toglet_lib::diagnostics::{ErrorCode, LOG_FILE_NAME, Level, LogRecord, Phase, log};

/// One of every shape the redaction layer must catch, written after the sink is in place.
const SENSITIVE: &str = concat!(
    "token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVP-mB92K27uhbUJU1p1r ",
    "key sk-abcdefghijklmnopqrstuvwxyz012345 ",
    "user someone.real@example.com ",
    r"path C:\Users\someone\.codex\auth.json ",
    "callback https://auth.example.com/cb?code=4/0AY0e-g7&state=xyz789abc"
);

// On the real start-up path: the file is private, records are written (including one made
// before the file existed), and nothing sensitive survives.
#[test]
fn the_application_log_is_private_drained_and_redacted() {
    let directory = IsolatedHome::create(Phase::Storage).expect("scratch directory");
    log(&LogRecord::new(Level::Warn, "before_the_file")
        .with_phase(Phase::Detect)
        .with_detail(SENSITIVE));

    assert!(install_file_log(directory.path()));

    log(&LogRecord::new(Level::Error, "after_the_file")
        .with_phase(Phase::Precheck)
        .with_code(ErrorCode::ClientRunning)
        .with_detail(SENSITIVE));

    let path = directory.path().join(LOG_FILE_NAME);
    assert!(is_private(&path).expect("permissions are readable"));
    let written = std::fs::read_to_string(&path).expect("the log file is readable");

    assert!(written.contains("event=before_the_file"));
    assert!(written.contains("event=started"));
    assert!(written.contains("event=after_the_file"));
    assert!(written.contains("code=client_running"));
    assert!(written.contains("phase=precheck"));
    for secret in [
        "eyJhbGciOiJIUzI1NiJ9",
        "sk-abcdefghijklmnopqrstuvwxyz012345",
        "someone.real@example.com",
        "example.com",
        r"C:\Users\someone",
        "4/0AY0e-g7",
        "xyz789abc",
    ] {
        assert!(
            !written.contains(secret),
            "the log file leaked {secret:?}\n{written}"
        );
    }

    // A second install is refused and the first file keeps being written.
    let elsewhere = IsolatedHome::create(Phase::Storage).expect("second scratch directory");
    assert!(!install_file_log(elsewhere.path()));
    log(&LogRecord::new(Level::Info, "still_the_first_file"));
    let written = std::fs::read_to_string(&path).expect("readable");
    assert!(written.contains("event=still_the_first_file"));
}
