//! The system tray icon and its menu.
//!
//! The summary line and wording are pushed in by the interface, which owns formatting. The menu
//! deliberately has no account switch, which would bypass the panel's confirmation. Whether the
//! surface is hidden is a stored setting kept by Rust; the interface does not know.

use std::sync::Mutex;

use serde::Deserialize;
use tauri::image::Image;
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

/// Wording until the interface sends its own, and the fallback if the webview never loads.
///
/// English on purpose: resolving a locale here could disagree with the interface.
const SUMMARY_PLACEHOLDER: &str = "Reading quota…";
const SHOW_PLACEHOLDER: &str = "Show Toglet";
const HIDE_PLACEHOLDER: &str = "Hide Toglet";
const REFRESH_PLACEHOLDER: &str = "Refresh quota";
const PRIMARY_PLACEHOLDER: &str = "Move to primary display";
const SETTINGS_PLACEHOLDER: &str = "Settings…";
const QUIT_PLACEHOLDER: &str = "Quit Toglet";

/// The menu's wording, in whichever language the interface is currently showing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrayLabels {
    pub show: String,
    /// Which of `show`/`hide` is displayed is decided here from the stored setting.
    pub hide: String,
    pub refresh: String,
    pub primary: String,
    pub settings: String,
    pub quit: String,
}

/// The menu items whose text changes, kept so they can be relabelled in place.
///
/// Held in managed state: a `TrayIcon` does not hand its menu back, and rebuilding the menu would
/// close it under the pointer.
struct Items {
    summary: MenuItem<tauri::Wry>,
    show: MenuItem<tauri::Wry>,
    refresh: MenuItem<tauri::Wry>,
    primary: MenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
    quit: PredefinedMenuItem<tauri::Wry>,
    /// Both readings of the first entry, for relabelling on hide and show.
    toggle: Mutex<ToggleWording>,
}

struct ToggleWording {
    show: String,
    hide: String,
}

impl Items {
    /// Puts whichever reading matches the stored setting on the first entry.
    fn relabel_toggle(&self, hidden: bool) {
        let wording = match self.toggle.lock() {
            Ok(guard) => guard,
            // A poisoned lock still holds two whole strings; nothing half-written to recover from.
            Err(poisoned) => poisoned.into_inner(),
        };
        let text = if hidden { &wording.show } else { &wording.hide };
        drop(self.show.set_text(text));
    }
}

/// Builds the tray icon and its menu; a tray that cannot be created is logged and skipped.
pub fn install(app: &AppHandle) -> Option<TrayIcon> {
    let hidden = is_hidden(app);
    let items = Items {
        summary: entry(app, ITEM_SUMMARY, SUMMARY_PLACEHOLDER, false)?,
        show: entry(
            app,
            ITEM_SHOW,
            if hidden {
                SHOW_PLACEHOLDER
            } else {
                HIDE_PLACEHOLDER
            },
            true,
        )?,
        refresh: entry(app, ITEM_REFRESH, REFRESH_PLACEHOLDER, true)?,
        primary: entry(app, ITEM_PRIMARY, PRIMARY_PLACEHOLDER, true)?,
        settings: entry(app, ITEM_SETTINGS, SETTINGS_PLACEHOLDER, true)?,
        quit: PredefinedMenuItem::quit(app, Some(QUIT_PLACEHOLDER))
            .inspect_err(|_| record("tray_menu_item_failed"))
            .ok()?,
        toggle: Mutex::new(ToggleWording {
            show: SHOW_PLACEHOLDER.to_owned(),
            hide: HIDE_PLACEHOLDER.to_owned(),
        }),
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

    TrayIconBuilder::with_id("toglet")
        .icon(tray_image())
        // macOS only: lets the menu bar tint the icon to match its appearance.
        .icon_as_template(true)
        .menu(&menu)
        // Left click shows the window (un-hiding it); the menu is on the right click.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            // Left button only: focusing the window on the right click steals the focus the menu
            // needs and closes it. Matched on release so it does not fight the menu.
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                reveal(tray.app_handle());
            }
        })
        .build(app)
        .inspect_err(|_| record("tray_icon_failed"))
        .ok()
}

/// Raw RGBA from `icons/generate.py` (which fixes the dimensions), so no image decoder is needed.
///
/// A black-on-transparent template the system recolours; the app icon is too small to read here.
#[cfg(target_os = "macos")]
fn tray_image() -> Image<'static> {
    Image::new(include_bytes!("../../icons/tray-macos.rgba"), 36, 36)
}

/// The Windows taskbar does not recolour icons, so the tile stays behind the ring.
#[cfg(not(target_os = "macos"))]
fn tray_image() -> Image<'static> {
    Image::new(include_bytes!("../../icons/tray-windows.rgba"), 32, 32)
}

fn entry(app: &AppHandle, id: &str, text: &str, enabled: bool) -> Option<MenuItem<tauri::Wry>> {
    MenuItem::with_id(app, id, text, enabled, None::<&str>)
        .inspect_err(|_| record("tray_menu_item_failed"))
        .ok()
}

/// Replaces the summary line; does nothing when there is no tray.
pub fn set_summary(app: &AppHandle, text: &str) {
    if let Some(items) = app.try_state::<Items>() {
        drop(items.summary.set_text(text));
    }
}

/// Relabels the menu; each item is set independently so one failure does not stop the rest.
pub fn set_labels(app: &AppHandle, labels: &TrayLabels) {
    let Some(items) = app.try_state::<Items>() else {
        return;
    };
    if let Ok(mut wording) = items.toggle.lock() {
        wording.show = labels.show.clone();
        wording.hide = labels.hide.clone();
    }
    items.relabel_toggle(is_hidden(app));
    drop(items.refresh.set_text(&labels.refresh));
    drop(items.primary.set_text(&labels.primary));
    drop(items.settings.set_text(&labels.settings));
    drop(items.quit.set_text(&labels.quit));
}

fn on_menu(app: &AppHandle, id: &str) {
    match id {
        ITEM_SHOW => {
            if is_hidden(app) {
                reveal(app);
            } else {
                conceal(app);
            }
        }
        // The interface carries these out. The settings sheet lives in the panel, so a hidden
        // window is revealed first.
        ITEM_REFRESH => ask(app, TRAY_REFRESH_EVENT),
        ITEM_SETTINGS => {
            reveal(app);
            ask(app, TRAY_SETTINGS_EVENT);
        }
        ITEM_PRIMARY => move_to_primary(app),
        // `quit` is a predefined item and needs no handling here.
        ITEM_QUIT => {}
        _ => {}
    }
}

/// Read from the stored settings each time rather than cached, so there is one source of truth.
fn is_hidden(app: &AppHandle) -> bool {
    app.try_state::<crate::commands::AppState>()
        .is_some_and(|state| state.read_document(|document| document.settings.dock_hidden))
}

/// A store failure is logged; the window still does what was asked.
fn set_hidden(app: &AppHandle, hidden: bool) {
    let Some(state) = app.try_state::<crate::commands::AppState>() else {
        return;
    };
    let stored = state.with_document(|document| {
        let changed = document.settings.dock_hidden != hidden;
        document.settings.dock_hidden = hidden;
        Ok(((), changed))
    });
    if let Err(error) = stored {
        log(&LogRecord::from_error("dock_hidden_not_stored", &error));
    }
    if let Some(items) = app.try_state::<Items>() {
        items.relabel_toggle(hidden);
    }
}

/// Brings the surface back and opens the panel, so "Show" is visible even when nothing was hidden.
fn reveal(app: &AppHandle) {
    if is_hidden(app) {
        set_hidden(app, false);
        log(&LogRecord::new(Level::Info, "dock_revealed").with_phase(Phase::Dock));
    }
    show_window(app);
    ask(app, TRAY_SHOW_EVENT);
}

/// Takes the surface off the screen, in whatever shape it has. Only the tray can undo this.
fn conceal(app: &AppHandle) {
    set_hidden(app, true);
    log(&LogRecord::new(Level::Info, "dock_hidden").with_phase(Phase::Dock));
    let Some(window) = app.get_webview_window(crate::MAIN_WINDOW) else {
        return;
    };
    if window.hide().is_err() {
        record("dock_window_not_hidden");
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

/// Puts the bar back on the primary display, for a bar left on an unreachable monitor.
fn move_to_primary(app: &AppHandle) {
    let Some(window) = app.get_webview_window(crate::MAIN_WINDOW) else {
        return;
    };
    let state = app.state::<crate::commands::AppState>();

    // Without a remembered monitor, selection falls through to the primary display.
    let mut settings = state.read_document(|document| document.settings.clone());
    settings.display_id = None;

    match super::dock_window(&window, &settings) {
        Ok(outcome) => {
            drop(state.with_document(|document| {
                document.settings.display_id = Some(outcome.display_id.clone());
                document.settings.vertical_offset = outcome.vertical_offset;
                Ok(((), true))
            }));
            // A hidden bar is brought back too.
            reveal(app);
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
    /// Tray actions reach the interface as events, which silently fail without the `event.listen`
    /// grant. An exact list, because the risk is a grant growing: `core:default` would expose the
    /// window, webview, path and app APIs. `notification:default` only posts notifications.
    #[test]
    fn the_interface_may_listen_for_events_and_notify_and_nothing_else() {
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
            [
                "core:event:allow-listen",
                "core:event:allow-unlisten",
                "notification:default"
            ]
        );
    }
}
