//! The environment detection command.

use crate::codex_home::{EnvironmentReport, detect_environment};

/// Runs the seven first-run checks and returns them one by one.
///
/// Infallible on purpose: a check that could not run reports `notApplicable`, so there is no
/// case where the frontend gets an error instead of a report and has nothing to show. The
/// return value carries stable codes and short facts only - no paths, no command lines, no
/// addresses.
// `async`: probing the Codex installation runs processes and reads files. Off the main thread,
// so the event loop is never held behind it.
#[tauri::command(async)]
pub fn detect_environment_command() -> EnvironmentReport {
    detect_environment()
}
