//! Pointer reach, dragging, and tray menu text.
//!
//! The dock edge is served only by the settings commands, and nothing here resizes the window:
//! it is a fixed strip, and opening the panel only changes what the pointer gate lets through.

use tauri::{AppHandle, State, WebviewWindow};

use super::settings::SettingsView;
use super::state::AppState;
use super::views::ErrorView;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::window::{self, PointerGate};

/// Largest accepted drag value in logical pixels (a sideways increment or the whole vertical
/// travel). Mouse coordinates are 16-bit on supported platforms, so anything larger is not
/// pointer input.
const MAX_DRAG_STEP: f64 = 65536.0;

/// Moves the window sideways by a drag increment, in logical pixels. Stores nothing; only
/// `end_drag` writes.
///
/// Vertical travel is not applied: macOS will not place a window's top above the menu bar, so the
/// interface moves the bar inside the full-height strip and reports the total to `end_drag`.
#[tauri::command]
pub fn move_dock(
    window: WebviewWindow,
    gate: State<'_, PointerGate>,
    dx: f64,
) -> std::result::Result<(), ErrorView> {
    // Keep the gate shut while dragging: letting the pointer through would lose pointer capture.
    gate.update(|reach| reach.dragging = true);
    nudge(&window, dx).map_err(ErrorView::from)
}

fn nudge(window: &WebviewWindow, dx: f64) -> Result<()> {
    let dx = checked_step(dx)?;
    let scale = window.scale_factor().map_err(|_| unreadable_window())?;
    let position = window.outer_position().map_err(|_| unreadable_window())?;

    window
        .set_position(tauri::PhysicalPosition::new(
            position.x + (dx * scale).round() as i32,
            position.y,
        ))
        .map_err(|_| unreadable_window())
}

fn checked_step(value: f64) -> Result<f64> {
    if value.is_finite() && value.abs() <= MAX_DRAG_STEP {
        return Ok(value);
    }
    Err(
        TogletError::new(ErrorCode::Internal, Phase::Dock, false, UserAction::None)
            .with_detail("a drag step was not a distance a pointer could have travelled"),
    )
}

/// Ends a drag: settles the bar, stores its place and re-docks the window. `lift` is the total
/// vertical travel in logical pixels, positive downward. Returns the stored settings so the
/// interface draws the bar where the pointer gate expects it.
#[tauri::command]
pub fn end_drag(
    state: State<'_, AppState>,
    window: WebviewWindow,
    gate: State<'_, PointerGate>,
    lift: f64,
) -> std::result::Result<SettingsView, ErrorView> {
    let settled = checked_step(lift)
        .and_then(|lift| settle(state.inner(), &window, lift))
        .map_err(ErrorView::from);
    // Cleared even on failure: a gate stuck shut would make the strip swallow clicks for good.
    gate.update(|reach| reach.dragging = false);
    settled
}

fn settle(state: &AppState, window: &WebviewWindow, lift: f64) -> Result<SettingsView> {
    let scale = window.scale_factor().map_err(|_| unreadable_window())?;
    let (edge, shape, offset) = state.read_document(|document| {
        (
            document.settings.dock_edge,
            document.settings.dock_shape,
            document.settings.vertical_offset,
        )
    });
    let landed = window::settle(
        &window::TauriDock::new(window),
        window::current_placement(window)?,
        edge,
        shape,
        offset,
        scale,
        lift,
    )?;

    let settings = state.with_document(|document| {
        document.settings.dock_edge = landed.edge;
        document.settings.display_id = Some(landed.display_id.clone());
        document.settings.vertical_offset = landed.vertical_offset;
        Ok((document.settings.clone(), true))
    })?;

    let outcome = window::dock_window(window, &settings)?;
    remember(state, &outcome)?;
    Ok(state.read_document(|document| SettingsView::of(&document.settings)))
}

/// Stores what docking actually did (a fallback monitor, a clamped offset) when it differs from
/// what was asked, since the interface places the bar from the stored offset.
pub(crate) fn remember(state: &AppState, outcome: &window::DockOutcome) -> Result<()> {
    state.with_document(|document| {
        let settings = &mut document.settings;
        let changed = settings.display_id.as_deref() != Some(outcome.display_id.as_str())
            || settings.vertical_offset != outcome.vertical_offset;
        if changed {
            settings.display_id = Some(outcome.display_id.clone());
            settings.vertical_offset = outcome.vertical_offset;
        }
        Ok(((), changed))
    })
}

fn unreadable_window() -> TogletError {
    TogletError::new(ErrorCode::Internal, Phase::Dock, true, UserAction::Retry)
        .with_detail("the window did not answer about its position")
}

/// Tells the pointer gate whether the panel is open: open, the whole window takes pointer events;
/// closed, only the bar does. Not done by resizing, because a window growing leftward flashes its
/// old frame at the new origin.
#[tauri::command]
pub fn set_dock_expansion(gate: State<'_, PointerGate>, expanded: bool) {
    gate.update(|reach| reach.expanded = expanded);
}

/// Sets the tray summary line. The interface formats it so it matches the panel; the length is
/// capped here.
#[tauri::command]
pub fn set_tray_summary(app: AppHandle, summary: String) {
    window::set_summary(&app, &capped(&summary, MAX_SUMMARY));
}

/// Menu line caps, in characters. A summary includes a user-chosen account name; labels are short.
const MAX_SUMMARY: usize = 120;
const MAX_LABEL: usize = 60;

/// Relabels the tray menu from the interface's dictionary, so it matches the panel's language.
/// Capped by characters, not bytes, so CJK labels are never cut mid-character.
#[tauri::command]
pub fn set_tray_labels(app: AppHandle, labels: window::TrayLabels) {
    window::set_labels(
        &app,
        &window::TrayLabels {
            show: capped(&labels.show, MAX_LABEL),
            hide: capped(&labels.hide, MAX_LABEL),
            refresh: capped(&labels.refresh, MAX_LABEL),
            primary: capped(&labels.primary, MAX_LABEL),
            settings: capped(&labels.settings, MAX_LABEL),
            quit: capped(&labels.quit, MAX_LABEL),
        },
    );
}

fn capped(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_drag_step_no_pointer_could_have_made() {
        assert!(checked_step(MAX_DRAG_STEP + 1.0).is_err());
        assert!(checked_step(f64::NAN).is_err());
        assert!(checked_step(f64::INFINITY).is_err());
    }

    #[test]
    fn accepts_a_frame_of_pointer_travel_in_either_direction() {
        assert_eq!(checked_step(-12.5).expect("within range"), -12.5);
        assert_eq!(checked_step(0.0).expect("within range"), 0.0);
    }

    #[test]
    fn a_label_is_cut_by_character_rather_than_by_byte() {
        // Three-byte characters: a byte cap would produce invalid text or panic.
        let long: String = "显".repeat(MAX_LABEL + 10);

        let cut = capped(&long, MAX_LABEL);

        assert_eq!(cut.chars().count(), MAX_LABEL);
        assert!(cut.chars().all(|one| one == '显'));
    }

    #[test]
    fn a_label_that_fits_is_left_exactly_as_it_came() {
        assert_eq!(capped("显示 Toglet", MAX_LABEL), "显示 Toglet");
    }
}
