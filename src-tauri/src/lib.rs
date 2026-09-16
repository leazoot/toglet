//! Toglet application library.
//!
//! Dependencies run one way: `commands` at the top, `diagnostics` as a leaf.

pub mod diagnostics;

pub mod accounts;
pub mod app_server;
pub mod autorun;
pub mod codex_config;
pub mod codex_home;
pub mod commands;
pub mod credentials;
pub mod net;
pub mod notify;
pub mod process;
pub mod quota;
pub mod remote;
pub mod storage;
pub mod switching;
pub mod window;

/// Starts the desktop shell.
///
/// Interrupted-switch recovery runs before the window is built, so the panel never describes
/// an account Codex would not actually use. If the data directory or credential store cannot
/// be prepared, start-up fails: there is no plaintext fallback.
pub fn run() -> Result<(), StartupFailure> {
    // Only the stable code reaches the exit path; the detail may carry OS messages with paths.
    let state = commands::AppState::start()
        .map_err(|error| StartupFailure::State(error.code().as_str()))?;
    // Best effort: an unwritable log is reported in memory and the application still runs.
    commands::state::install_file_log(state.data_directory());

    // A killed run cannot clean up, leaving isolated homes that hold a decrypted `auth.json`.
    let swept = codex_home::sweep_stale();
    if swept > 0 {
        diagnostics::log(
            &diagnostics::LogRecord::new(diagnostics::Level::Warn, "stale_isolated_homes_removed")
                .with_phase(diagnostics::Phase::Detect)
                .with_detail(&swept.to_string()),
        );
    }

    // A failed recovery must not block start-up, or the user could never reach the repair.
    // Its result is exposed to the interface.
    let recovery = commands::switching::recover_interrupted_switch(&state);

    tauri::Builder::default()
        // System notifications carry only a state and an account display name.
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .manage(StartupRecovery(recovery.ok().flatten()))
        .manage(window::PointerGate::default())
        .manage(commands::onboarding::PendingSignIn::default())
        .setup(|app| {
            use tauri::Manager;
            // Started here because the driver needs the app handle to announce state changes.
            let state = app.state::<commands::AppState>().inner().clone();
            app.manage(commands::notify::Notifications::load(
                state.data_directory(),
            ));
            app.manage(commands::remote::Remote::load(state.data_directory()));
            app.manage(commands::autorun::AutoRun::start(
                app.handle().clone(),
                state,
            ));
            // Must start after `Remote` (settings) and `AutoRun` (state machine) are managed.
            // With remote control off, the default, it sleeps and nothing leaves the machine.
            commands::remote_poll::start(app.handle().clone());
            dock_main_window(app);
            // Best effort: a tray that could not be created is logged; the bar still works.
            drop(window::install_tray(app.handle()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::environment::detect_environment_command,
            commands::accounts::list_accounts,
            commands::accounts::import_current_account,
            commands::accounts::refresh_quota,
            commands::accounts::consume_reset_credit,
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
            commands::autorun::read_autorun,
            commands::autorun::list_threads,
            commands::autorun::bind_autorun,
            commands::autorun::set_autorun_enabled,
            commands::autorun::pause_autorun,
            commands::autorun::resume_autorun,
            commands::autorun::cancel_autorun,
            commands::notify::read_notify_channels,
            commands::notify::save_notify_channel,
            commands::notify::remove_notify_channel,
            commands::notify::send_notification,
            commands::remote::read_remote,
            commands::remote::save_remote,
            commands::remote::forget_remote,
            commands::remote::remote_secret_minimum,
            startup_recovery,
        ])
        .run(tauri::generate_context!())
        .map_err(|_| StartupFailure::Shell)
}

/// The label Tauri gives the single window declared in `tauri.conf.json`.
pub(crate) const MAIN_WINDOW: &str = "main";

/// Places the bar on the screen edge the user last left it on; failures are logged, not fatal.
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

    // The frameless, focus-less bar offers no way to open the inspector, so frontend errors
    // would otherwise be invisible in development. Compiled out of release builds.
    #[cfg(debug_assertions)]
    window.open_devtools();

    // The window starts hidden so it never flashes at the default position. It is shown even if
    // placement failed, unless the user hid it from the tray.
    if !settings.dock_hidden && window.show().is_err() {
        diagnostics::log(
            &diagnostics::LogRecord::new(diagnostics::Level::Error, "dock_window_not_shown")
                .with_phase(diagnostics::Phase::Dock),
        );
    }

    // Lets clicks through the transparent strip outside the bar. Started only after `show()`:
    // on macOS a decision sent to a still-hidden window is lost and the strip swallows clicks.
    window::watch_pointer(
        window.clone(),
        app.state::<window::PointerGate>().inner().clone(),
    );
}

/// Remembers the bar's monitor and offset for the next start. Best effort: failures are logged.
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
/// Carries only a stable code: the shell's error `Debug` form can include a path, and this value
/// is printed as the process exits.
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
