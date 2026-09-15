use crate::codex_home::{EnvironmentReport, detect_environment};

/// Runs the first-run checks. Infallible: a check that could not run reports `notApplicable`.
// `async`: probing runs processes and reads files, which must not block the event loop.
#[tauri::command(async)]
pub fn detect_environment_command() -> EnvironmentReport {
    detect_environment()
}
