//! Toglet application library.
//!
//! Dependencies run one way only: `commands` at the top, `diagnostics` as a leaf that nothing
//! business-related may reach back into.

pub mod diagnostics;

pub mod accounts;
pub mod app_server;
pub mod codex_config;
pub mod codex_home;
pub mod commands;
pub mod credentials;
pub mod process;
pub mod quota;
pub mod storage;
pub mod switching;
pub mod window;

/// Starts the desktop shell. Errors are propagated instead of unwrapped so the process
/// exit code reflects a failed startup.
///
/// **Recovery runs before the window is built.** A journal on disk means a switch was
/// interrupted, and the authentication has to be brought back to a state the user can trust
/// before anything is shown - otherwise the first thing they see is a panel describing an
/// account that may not be the one Codex would actually use.
///
/// A start-up that cannot prepare its own data directory or reach the credential store fails
/// rather than continuing with a degraded store: falling back to plaintext is forbidden
/// without exception.
pub fn run() -> Result<(), StartupFailure> {
    // Only the stable code reaches the process's exit path. The error's detail may carry an
    // operating-system message, and those carry paths.
    let state = commands::AppState::start()
        .map_err(|error| StartupFailure::State(error.code().as_str()))?;

    // A run that was killed outright could not run its own cleanup, and an isolated home left
    // that way holds a decrypted `auth.json`. Its permissions still protect it, but a credential
    // that serves no purpose should not stay on disk. Same idea as the journal below: repair
    // what an interrupted run left behind, before anything else happens.
    let swept = codex_home::sweep_stale();
    if swept > 0 {
        diagnostics::log(
            &diagnostics::LogRecord::new(diagnostics::Level::Warn, "stale_isolated_homes_removed")
                .with_phase(diagnostics::Phase::Detect)
                .with_detail(&swept.to_string()),
        );
    }

    // A recovery that itself fails must not stop the application from starting - the user would
    // be left with no way to reach the repair. It is carried out, and its result is available
    // to the interface rather than being discarded.
    let recovery = commands::switching::recover_interrupted_switch(&state);

    tauri::Builder::default()
        .manage(state)
        .manage(StartupRecovery(recovery.ok().flatten()))
        .manage(window::PointerGate::default())
        .manage(commands::onboarding::PendingSignIn::default())
        .setup(|app| {
            dock_main_window(app);
            // Optional on purpose: a tray that could not be created is recorded and skipped.
            // The bar is still on screen, and refusing to start over a missing tray would take
            // away more than it protects.
            drop(window::install_tray(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::environment::detect_environment_command,
            commands::accounts::list_accounts,
            commands::accounts::import_current_account,
            commands::accounts::refresh_quota,
            commands::accounts::remove_account,
            commands::switching::switch_account,
            commands::switching::inspect_clients,
            commands::onboarding::start_login,
            commands::onboarding::finish_login,
            commands::onboarding::cancel_login,
            commands::settings::read_settings,
            commands::settings::update_settings,
            commands::window::set_tray_summary,
            commands::window::set_tray_labels,
            commands::window::set_dock_expansion,
            commands::window::move_dock,
            commands::window::end_drag,
            startup_recovery,
        ])
        .run(tauri::generate_context!())
        .map_err(|_| StartupFailure::Shell)
}

/// The label Tauri gives the single window declared in `tauri.conf.json`.
pub(crate) const MAIN_WINDOW: &str = "main";

/// Places the bar against the screen edge the user last left it on.
///
/// A failure here is recorded and does not stop the application. The window already exists at
/// the size the configuration gives it, so the user still has something to reach; refusing to
/// start because a monitor could not be measured would be the worse answer.
fn dock_main_window(app: &tauri::App) {
    use tauri::Manager;

    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        diagnostics::log(
            &diagnostics::LogRecord::new(diagnostics::Level::Error, "dock_window_missing")
                .with_phase(diagnostics::Phase::Dock),
        );
        return;
    };

    let state = app.state::<commands::AppState>();
    let settings = state.read_document(|document| document.settings.clone());

    match window::dock_window(&window, &settings) {
        Ok(outcome) => remember_placement(&state, &outcome),
        Err(error) => diagnostics::log(&diagnostics::LogRecord::from_error("dock_failed", &error)),
    }

    // The strip is transparent almost everywhere; this is what lets clicks through the part of
    // it that is not the bar. It runs for the life of the process.
    window::watch_pointer(
        window.clone(),
        app.state::<window::PointerGate>().inner().clone(),
    );

    // The bar has no title bar, no menu and never takes keyboard focus, so there is no gesture
    // that could reach the inspector. Without this a frontend error in the docked window is
    // invisible - which is how a broken IPC call went unnoticed for a whole stage. Compiled out
    // of release builds by `debug_assertions`.
    #[cfg(debug_assertions)]
    window.open_devtools();

    // The window starts hidden so it is never seen at the configuration's default position
    // before being moved. It is shown on both paths: a bar that could not be placed is still
    // better than no bar at all.
    if window.show().is_err() {
        diagnostics::log(
            &diagnostics::LogRecord::new(diagnostics::Level::Error, "dock_window_not_shown")
                .with_phase(diagnostics::Phase::Dock),
        );
    }
}

/// Stores the monitor the bar ended up on and the offset it was given, so the next start
/// returns to exactly this.
///
/// Best effort: the bar is already placed, and a settings file that could not be written is not
/// a reason to undo that. The failure is recorded rather than dropped.
fn remember_placement(state: &commands::AppState, outcome: &window::DockOutcome) {
    if let Err(error) = commands::window::remember(state, outcome) {
        diagnostics::log(&diagnostics::LogRecord::from_error(
            "dock_display_not_remembered",
            &error,
        ));
    }
}

/// A start-up that could not complete.
///
/// Carries a stable code and nothing else. The shell's own error is deliberately dropped: its
/// `Debug` form can include a path, and this value is what the process prints as it exits.
#[derive(Debug)]
pub enum StartupFailure {
    /// Toglet could not prepare its own state - data directory, credential store or metadata.
    State(&'static str),
    /// The desktop shell itself failed to start.
    Shell,
}

/// What the interrupted-switch recovery did at start-up, if anything.
struct StartupRecovery(Option<&'static str>);

/// Lets the interface report an interrupted switch that was dealt with before it opened.
#[tauri::command]
fn startup_recovery(state: tauri::State<'_, StartupRecovery>) -> Option<&'static str> {
    state.0
}
