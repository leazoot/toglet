//! The system tray icon and its menu.
//!
//! The tray exists so the bar is reachable when it is hidden, and so the two facts a user checks
//! most often - which account is in use and how much quota is left - can be read without opening
//! anything.
//!
//! **The summary line is pushed in by the interface, not formatted here.** The interface already
//! owns percentage rounding, the compact reset form and the three-state rules; formatting them
//! a second time in Rust would create a second source of truth that could disagree with the
//! panel about the same number.
//!
//! **What the menu deliberately does not have:** anything that switches accounts. There is
//! deliberately no one-click switch that bypasses the confirmation. Switching is a decision, and
//! a decision needs the panel.

use serde::Deserialize;
use tauri::menu::{MenuBuilder, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use crate::diagnostics::{Level, LogRecord, Phase, log};

/// Events the interface listens for. Stable wire names.
pub const TRAY_SHOW_EVENT: &str = "tray://show";
pub const TRAY_REFRESH_EVENT: &str = "tray://refresh";
pub const TRAY_SETTINGS_EVENT: &str = "tray://settings";

const ITEM_SUMMARY: &str = "summary";
const ITEM_SHOW: &str = "show";
const ITEM_REFRESH: &str = "refresh";
const ITEM_PRIMARY: &str = "primary";
const ITEM_SETTINGS: &str = "settings";
const ITEM_QUIT: &str = "quit";

/// What the menu says before the interface has loaded and sent its own wording.
///
/// English rather than the operating system's language, and deliberately so: resolving a locale
/// here would be a second answer to a question the interface already answers, and the two could
/// disagree. These are a placeholder measured in milliseconds, and the fallback if the webview
/// never comes up at all - in which case the menu is the only way left to quit.
const SUMMARY_PLACEHOLDER: &str = "Reading quota…";
const SHOW_PLACEHOLDER: &str = "Show Toglet";
const REFRESH_PLACEHOLDER: &str = "Refresh quota";
const PRIMARY_PLACEHOLDER: &str = "Move to primary display";
const SETTINGS_PLACEHOLDER: &str = "Settings…";
const QUIT_PLACEHOLDER: &str = "Quit Toglet";

/// The menu's wording, in whichever language the interface is currently showing.
///
/// Pushed in for the same reason the summary line is: the dictionary lives on the interface side,
/// and a second copy of `Show Toglet` / `显示 Toglet` in Rust would be copy that can fall out of
/// step with the panel it sits beside.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrayLabels {
    pub show: String,
    pub refresh: String,
    pub primary: String,
    pub settings: String,
    pub quit: String,
}

/// The menu items whose text changes, kept so they can be relabelled in place.
///
/// Held in managed state rather than looked up from the tray: a `TrayIcon` does not hand its menu
/// back, and rebuilding the whole menu to change a line would drop it out from under a pointer
/// that is on it.
struct Items {
    summary: MenuItem<tauri::Wry>,
    show: MenuItem<tauri::Wry>,
    refresh: MenuItem<tauri::Wry>,
    primary: MenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
    quit: PredefinedMenuItem<tauri::Wry>,
}

/// Builds the tray icon and its menu.
///
/// A tray that cannot be created is recorded and skipped: the bar is still on screen, and
/// refusing to start over a missing tray would take away more than it protects.
pub fn install(app: &AppHandle) -> Option<TrayIcon> {
    let items = Items {
        summary: entry(app, ITEM_SUMMARY, SUMMARY_PLACEHOLDER, false)?,
        show: entry(app, ITEM_SHOW, SHOW_PLACEHOLDER, true)?,
        refresh: entry(app, ITEM_REFRESH, REFRESH_PLACEHOLDER, true)?,
        primary: entry(app, ITEM_PRIMARY, PRIMARY_PLACEHOLDER, true)?,
        settings: entry(app, ITEM_SETTINGS, SETTINGS_PLACEHOLDER, true)?,
        quit: PredefinedMenuItem::quit(app, Some(QUIT_PLACEHOLDER))
            .inspect_err(|_| record("tray_menu_item_failed"))
            .ok()?,
    };

    let menu = MenuBuilder::new(app)
        .item(&items.summary)
        .separator()
        .item(&items.show)
        .item(&items.refresh)
        .item(&items.primary)
        .separator()
        .item(&items.settings)
        .separator()
        .item(&items.quit)
        .build()
        .inspect_err(|_| record("tray_menu_failed"))
        .ok()?;

    app.manage(items);

    let icon = app.default_window_icon().cloned()?;
    TrayIconBuilder::with_id("toglet")
        .icon(icon)
        .menu(&menu)
        // The left click shows the window; the menu is the right click. Without this the left
        // click would open the menu too and there would be no quick way back to the bar.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            // The left button only. `Click` covers every button, and the right button is the one
            // that opens the menu - showing and focusing the window on that click took the focus
            // the menu needs, so it closed in the same instant it appeared.
            //
            // Matched on release rather than press for the same reason: the menu is up by then,
            // and acting on the press would fight it.
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_window(tray.app_handle());
            }
        })
        .build(app)
        .inspect_err(|_| record("tray_icon_failed"))
        .ok()
}

fn entry(app: &AppHandle, id: &str, text: &str, enabled: bool) -> Option<MenuItem<tauri::Wry>> {
    MenuItem::with_id(app, id, text, enabled, None::<&str>)
        .inspect_err(|_| record("tray_menu_item_failed"))
        .ok()
}

/// Replaces the summary line with what the interface currently shows.
///
/// Silently does nothing when there is no tray - the tray is optional, and a missing one is not
/// a reason to fail a routine update.
pub fn set_summary(app: &AppHandle, text: &str) {
    if let Some(items) = app.try_state::<Items>() {
        drop(items.summary.set_text(text));
    }
}

/// Relabels the menu in the language the interface is showing.
///
/// Each item is set independently and a failure on one does not stop the rest: a menu that is
/// half relabelled is still usable, while giving up on the first failure could leave the item
/// that failed as the only one anybody notices.
pub fn set_labels(app: &AppHandle, labels: &TrayLabels) {
    let Some(items) = app.try_state::<Items>() else {
        return;
    };
    drop(items.show.set_text(&labels.show));
    drop(items.refresh.set_text(&labels.refresh));
    drop(items.primary.set_text(&labels.primary));
    drop(items.settings.set_text(&labels.settings));
    drop(items.quit.set_text(&labels.quit));
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        // The bar is always on screen, so "show" has to mean something the user can see: the
        // interface opens the panel. Without the event the entry did nothing visible, which
        // reads as a menu that does not work.
        ITEM_SHOW => {
            show_window(app);
            ask(app, TRAY_SHOW_EVENT);
        }
        // Both are the interface's to carry out: it owns the quota reading and the settings
        // sheet. The tray only asks.
        ITEM_REFRESH => ask(app, TRAY_REFRESH_EVENT),
        ITEM_SETTINGS => {
            show_window(app);
            ask(app, TRAY_SETTINGS_EVENT);
        }
        ITEM_PRIMARY => move_to_primary(app),
        // `quit` is a predefined item and needs no handling here.
        ITEM_QUIT => {}
        _ => {}
    }
}

fn show_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(crate::MAIN_WINDOW) else {
        return;
    };
    drop(window.show());
    drop(window.set_focus());
}

fn ask(app: &AppHandle, event: &str) {
    if app.emit(event, ()).is_err() {
        record("tray_event_not_delivered");
    }
}

/// Puts the bar back on the primary display.
///
/// The way out when a monitor was unplugged while Toglet was not running, or when the bar ended
/// up somewhere the user cannot reach.
fn move_to_primary(app: &AppHandle) {
    let Some(window) = app.get_webview_window(crate::MAIN_WINDOW) else {
        return;
    };
    let state = app.state::<crate::commands::AppState>();

    // `display_id: None` is what "the remembered monitor is not a consideration" looks like, so
    // the selection falls through to the primary display.
    let mut settings = state.read_document(|document| document.settings.clone());
    settings.display_id = None;

    match super::dock_window(&window, &settings) {
        Ok(outcome) => {
            drop(state.with_document(|document| {
                document.settings.display_id = Some(outcome.display_id.clone());
                document.settings.vertical_offset = outcome.vertical_offset;
                Ok(((), true))
            }));
            drop(window.show());
        }
        Err(error) => log(&LogRecord::from_error(
            "tray_move_to_primary_failed",
            &error,
        )),
    }
}

fn record(event: &'static str) {
    log(&LogRecord::new(Level::Warn, event).with_phase(Phase::Dock));
}

#[cfg(test)]
mod tests {
    /// The tray can only ask; the interface carries out. That ask travels as an event, and
    /// `event.listen` is a core plugin call - without an explicit grant it is refused at run time
    /// and every entry in the menu quietly does nothing, because the refusal arrives as a
    /// rejected promise that the interface's IPC layer turns into an ordinary failed result.
    ///
    /// Asserted as an exact list rather than a contains-check: the risk here is a grant growing,
    /// not shrinking. `core:default` would hand the frontend the window, webview, path and app
    /// APIs it must never have, and it would pass any test that
    /// only looked for what is needed.
    #[test]
    fn the_interface_may_listen_for_events_and_nothing_else() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json"))
                .expect("the capability file is valid JSON");

        let granted: Vec<&str> = capability["permissions"]
            .as_array()
            .expect("permissions is a list")
            .iter()
            .map(|one| one.as_str().expect("every permission is a string"))
            .collect();

        assert_eq!(
            granted,
            ["core:event:allow-listen", "core:event:allow-unlisten"]
        );
    }
}
