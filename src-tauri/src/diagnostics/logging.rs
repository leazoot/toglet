//! The log sink, and the record type that decides what may reach it.
//!
//! The allow-list of what may be logged is enforced by the shape of
//! [`LogRecord`], not by review:
//!
//! * `event` is a `&'static str`. A caller cannot interpolate a token into a static string, so
//!   the message part of a line is a compile-time constant at every call site.
//! * `phase` and `code` are enums with fixed wire forms.
//! * `detail` is the only free-form field and the only way to set it runs `redact` first.
//!
//! Redaction therefore happens before the value can reach a sink, which is the requirement -
//! not after, and not by a filter someone has to remember to apply.
//!
//! The file itself is opened by `codex_home::permissions::open_private_append`, so it carries
//! the same user-only permissions as every other file Toglet creates. This module takes the
//! open handle rather than the path: `diagnostics` is a leaf and may not depend on a business
//! module.

use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{ErrorCode, Phase, TogletError, redact};

/// The log file name callers should use.
pub const LOG_FILE_NAME: &str = "toglet.log";

/// Upper bound on records held before a sink exists. Failures can happen during startup,
/// before the log location is known; they are drained into the sink by [`install`].
const PENDING_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// One line's worth of diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    level: Level,
    event: &'static str,
    phase: Option<Phase>,
    code: Option<ErrorCode>,
    detail: Option<String>,
}

impl LogRecord {
    pub fn new(level: Level, event: &'static str) -> Self {
        Self {
            level,
            event,
            phase: None,
            code: None,
            detail: None,
        }
    }

    #[must_use]
    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = Some(phase);
        self
    }

    #[must_use]
    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.code = Some(code);
        self
    }

    /// Attaches free-form detail. Redacted here, before it can be stored or written.
    #[must_use]
    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail = Some(redact(detail));
        self
    }

    /// Builds a record from an error. The error's detail is already redacted, so it is carried
    /// across without a second pass.
    pub fn from_error(event: &'static str, error: &TogletError) -> Self {
        let mut record = Self::new(Level::Error, event)
            .with_phase(error.phase())
            .with_code(error.code());
        record.detail = error.detail().map(str::to_owned);
        record
    }

    pub fn event(&self) -> &'static str {
        self.event
    }

    pub fn phase(&self) -> Option<Phase> {
        self.phase
    }

    pub fn code(&self) -> Option<ErrorCode> {
        self.code
    }

    /// The redacted detail, if any.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    /// The line as it is written. Fields are `key=value`, separated by spaces, with the detail
    /// last and quoted - a shape that greps cleanly and has no ambiguity about where a line
    /// ends, because a newline can never appear in any field.
    fn format(&self) -> String {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let mut line = format!(
            "ts={seconds} level={} event={}",
            self.level.as_str(),
            self.event
        );
        if let Some(phase) = self.phase {
            line.push_str(&format!(" phase={}", phase.as_str()));
        }
        if let Some(code) = self.code {
            line.push_str(&format!(" code={}", code.as_str()));
        }
        if let Some(detail) = &self.detail {
            // Newlines would break the one-record-per-line contract; nothing else needs
            // escaping because the value has already been through `redact`.
            line.push_str(&format!(" detail=\"{}\"", detail.replace('\n', " ")));
        }
        line
    }
}

/// A file the process appends records to.
pub struct Logger {
    file: Mutex<std::fs::File>,
}

impl Logger {
    /// Wraps an already-open, already-private file.
    ///
    /// The caller is responsible for having opened it through
    /// `codex_home::permissions::open_private_append`; that is where the permission guarantee
    /// lives, and duplicating the check here would mean a second implementation of it.
    pub fn new(file: std::fs::File) -> Self {
        Self {
            file: Mutex::new(file),
        }
    }

    pub fn record(&self, record: &LogRecord) -> std::io::Result<()> {
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        writeln!(file, "{}", record.format())?;
        file.flush()
    }
}

static SINK: OnceLock<Logger> = OnceLock::new();
static PENDING: Mutex<Vec<LogRecord>> = Mutex::new(Vec::new());

/// Installs the process-wide sink and drains anything recorded before it existed.
///
/// Returns the logger back if one was already installed, so a second call is a visible mistake
/// rather than a silently ignored one.
pub fn install(logger: Logger) -> Result<(), Logger> {
    SINK.set(logger)?;
    let drained = std::mem::take(&mut *pending());
    for record in &drained {
        write_to_sink(record);
    }
    Ok(())
}

/// Records one line.
pub fn log(record: &LogRecord) {
    if SINK.get().is_some() {
        write_to_sink(record);
    } else {
        let mut pending = pending();
        if pending.len() < PENDING_CAPACITY {
            pending.push(record.clone());
        }
    }
}

/// Records a failure raised somewhere that cannot return it - a `Drop` guard, a background
/// thread. Swallowing it is not an option.
pub fn record_background_failure(error: TogletError) {
    log(&LogRecord::from_error("background_failure", &error));
}

/// Removes and returns records that are still waiting for a sink.
///
/// The only consumer other than [`install`] is a test that needs to observe a `Drop` guard's
/// failure without installing a process-wide sink.
pub fn take_pending() -> Vec<LogRecord> {
    std::mem::take(&mut *pending())
}

fn write_to_sink(record: &LogRecord) {
    if let Some(sink) = SINK.get() {
        // The one place in the codebase where a failure has nowhere left to go: this *is* the
        // reporting channel. Escalating would turn a full disk into a crash, and there is no
        // second sink to complain to.
        drop(sink.record(record));
    }
}

fn pending() -> std::sync::MutexGuard<'static, Vec<LogRecord>> {
    PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_home::{IsolatedHome, permissions};
    use crate::diagnostics::UserAction;

    /// A sample containing one of every shape the redaction layer must catch.
    const SENSITIVE: &str = concat!(
        "token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVP-mB92K27uhbUJU1p1r ",
        "key sk-abcdefghijklmnopqrstuvwxyz012345 ",
        "user someone.real@example.com ",
        r"path C:\Users\someone\.codex\auth.json ",
        "callback https://auth.example.com/cb?code=4/0AY0e-g7&state=xyz789abc"
    );

    fn logger() -> (IsolatedHome, Logger, std::path::PathBuf) {
        let directory = IsolatedHome::create(Phase::Storage).expect("scratch directory");
        let path = directory.path().join(LOG_FILE_NAME);
        let file = permissions::open_private_append(&path).expect("the log file opens");
        (directory, Logger::new(file), path)
    }

    /// The leak probe: write constructed secrets, then read the file back and
    /// assert none of them survived.
    #[test]
    fn nothing_sensitive_survives_a_round_trip_through_the_log_file() {
        let (_directory, logger, path) = logger();

        logger
            .record(
                &LogRecord::new(Level::Error, "leak_probe")
                    .with_phase(Phase::ReadQuota)
                    .with_code(ErrorCode::AuthExpired)
                    .with_detail(SENSITIVE),
            )
            .expect("the record is written");

        let written = std::fs::read_to_string(&path).expect("the log file is readable");
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
                "the log file leaked {secret:?}\nline was: {written}"
            );
        }
        // The line still has to be useful, or redaction has just destroyed the diagnostics.
        assert!(written.contains("event=leak_probe"));
        assert!(written.contains("code=auth_expired"));
        assert!(written.contains("phase=read_quota"));
    }

    #[test]
    fn the_log_file_is_readable_by_the_current_user_only() {
        let (_directory, logger, path) = logger();
        logger
            .record(&LogRecord::new(Level::Info, "started"))
            .expect("the record is written");

        permissions::assert_private(&path);
    }

    #[test]
    fn a_record_built_from_an_error_carries_its_code_and_phase_and_no_prose() {
        let error = TogletError::new(
            ErrorCode::CredentialStoreUnavailable,
            Phase::Storage,
            true,
            UserAction::UnlockCredentialStore,
        )
        .with_detail(r"failed for user@example.com at C:\Users\someone\store");

        let line = LogRecord::from_error("store_failed", &error).format();

        assert!(line.contains("code=credential_store_unavailable"));
        assert!(line.contains("phase=storage"));
        assert!(!line.contains("user@example.com"));
        assert!(!line.contains(r"C:\Users"));
    }

    #[test]
    fn every_record_stays_on_one_line() {
        let line = LogRecord::new(Level::Warn, "multi")
            .with_detail("first\nsecond\nthird")
            .format();

        assert!(!line.contains('\n'), "a record must never span lines");
        assert!(line.contains("first second third"));
    }

    #[test]
    fn appending_keeps_earlier_records() {
        let (_directory, logger, path) = logger();

        logger
            .record(&LogRecord::new(Level::Info, "first"))
            .expect("first record");
        logger
            .record(&LogRecord::new(Level::Info, "second"))
            .expect("second record");

        let written = std::fs::read_to_string(&path).expect("the log file is readable");
        assert_eq!(written.lines().count(), 2);
        assert!(written.contains("event=first") && written.contains("event=second"));
    }
}
