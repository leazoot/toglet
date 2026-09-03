//! Structured error codes and redaction.
//!
//! Leaf module: every other module may depend on it, it depends on none of them.

mod error;
mod logging;
mod redact;

pub use error::{ErrorCode, Phase, TogletError, UserAction};
pub use logging::{
    LOG_FILE_NAME, Level, LogRecord, Logger, install, log, record_background_failure, take_pending,
};
pub use redact::redact;

/// Convenience alias for fallible Toglet operations.
pub type Result<T> = std::result::Result<T, TogletError>;
