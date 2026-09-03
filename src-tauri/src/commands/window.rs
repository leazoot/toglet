//! Where the pointer can reach the surface, the drag, and what the tray menu says.
//!
//! **The side the bar mirrors against is not served from here.** It is stored with the settings,
//! and `read_settings`/`update_settings` already carry it; a second command answering the same
//! stored value gave the interface a copy that nothing refreshed, so changing the edge moved the
//! window while the bar went on drawing its buffer and its rounded corners against the side it
//! had left.
//!
//! **Nothing here resizes the window.** It is a fixed strip (`window::geometry`); opening the
//! panel changes what the pointer gate lets through, not what the window is.

use tauri::{AppHandle, State, WebviewWindow};

use super::settings::SettingsView;
use super::state::AppState;
use super::views::ErrorView;
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::window::{self, PointerGate};

/// The furthest a single drag increment may move the window, in logical pixels.
///
/// An increment is the pointer's net travel since the window last answered a move - the
/// interface sums the moves that arrive while one is on its way - so it can be a whole sweep,
/// not a frame. It still cannot exceed the span of the desktop: mouse coordinates on
/// every platform served here are 16-bit, so no pointer can travel further than this between
/// two readings. Anything larger was not produced by a pointer, and acting on it would throw the
/// window somewhere the user cannot follow.
const MAX_DRAG_STEP: f64 = 65536.0;

/// Moves the window by a drag's latest increment, in logical pixels.
///
/// Relative rather than absolute so the interface never has to reason about screen coordinates or
/// scale factors, and **it stores nothing**: a drag that is abandoned, interrupted or crosses no
/// edge must leave the remembered place exactly as it was. Only `end_drag` writes.
#[tauri::command]
pub fn move_dock(
    window: WebviewWindow,
    gate: State<'_, PointerGate>,
    dx: f64,
    dy: f64,
) -> std::result::Result<(), ErrorView> {
    // The window is about to move out from under the rectangle the gate holds. Until the drag
    // has settled, nothing may be let through - letting the pointer through mid-drag would take
    // the pointer capture, and the window, away from the user.
    gate.update(|reach| reach.dragging = true);
    nudge(&window, dx, dy).map_err(ErrorView::from)
}

fn nudge(window: &WebviewWindow, dx: f64, dy: f64) -> Result<()> {
    let (dx, dy) = (checked_step(dx)?, checked_step(dy)?);
    let scale = window.scale_factor().map_err(|_| unreadable_window())?;
    let position = window.outer_position().map_err(|_| unreadable_window())?;

    window
        .set_position(tauri::PhysicalPosition::new(
            position.x + (dx * scale).round() as i32,
            position.y + (dy * scale).round() as i32,
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

/// Ends a drag: works out where the bar belongs, remembers it and docks the window there.
///
/// Returns the settings as stored, because the interface draws the bar from the stored offset
/// and has no other way to learn the new one. Left to keep the old offset, it drew the bar where
/// the pointer gate no longer let the pointer through - a bar that could be seen but neither
/// hovered nor dragged, until the next start.
#[tauri::command]
pub fn end_drag(
    state: State<'_, AppState>,
    window: WebviewWindow,
    gate: State<'_, PointerGate>,
) -> std::result::Result<SettingsView, ErrorView> {
    let settled = settle(state.inner(), &window);
    // Cleared whether or not the drag settled. A gate that stayed shut would keep the strip
    // swallowing clicks for good; one that reopens over a stale rectangle costs at worst a bar
    // the pointer cannot find, and the tray can still open the panel.
    gate.update(|reach| reach.dragging = false);
    settled.map_err(ErrorView::from)
}

fn settle(state: &AppState, window: &WebviewWindow) -> Result<SettingsView> {
    let scale = window.scale_factor().map_err(|_| unreadable_window())?;
    let (edge, offset) = state.read_document(|document| {
        (
            document.settings.dock_edge,
            document.settings.vertical_offset,
        )
    });
    let landed = window::settle(
        &window::TauriDock::new(window),
        window::current_placement(window)?,
        edge,
        offset,
        scale,
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

/// Stores what docking actually did, when it differs from what was asked.
///
/// The monitor may be a fallback for one that is no longer attached, and the offset may have
/// been clamped to fit a shorter one. Either way the settings should say where the bar is, not
/// where it was asked to be: the stylesheet places the bar from the stored offset, and the
/// interface reads that offset from the same settings.
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

/// Tells the pointer gate whether the panel is open.
///
/// While it is, the whole window is surface and every pointer event reaches it; while it is not,
/// only the bar does and the rest of the strip lets clicks through to the desktop. This replaced
/// resizing the window for the open panel: a window that grows leftward shows its
/// old picture at its new origin for a frame, and no ordering of calls could hide that.
#[tauri::command]
pub fn set_dock_expansion(gate: State<'_, PointerGate>, expanded: bool) {
    gate.update(|reach| reach.expanded = expanded);
}

/// Puts the interface's own summary line into the tray menu.
///
/// The text is formatted by the interface rather than here. It already owns percentage rounding,
/// the compact reset form and the three-state rules; formatting them again in Rust would create a
/// second source of truth that could disagree with the panel about the same number.
///
/// Length-capped at the boundary: a menu item is not a place to put an unbounded string, and the
/// cap is what stops one from getting there.
#[tauri::command]
pub fn set_tray_summary(app: AppHandle, summary: String) {
    window::set_summary(&app, &capped(&summary, MAX_SUMMARY));
}

/// The longest a menu line may be. A summary carries an account name, which the user chose; the
/// labels are the interface's own copy, so they need far less room.
const MAX_SUMMARY: usize = 120;
const MAX_LABEL: usize = 60;

/// Relabels the tray menu in the language the interface is showing.
///
/// The wording comes from the interface's dictionary rather than a second one here. A menu item
/// reading `Show Toglet` beside a panel reading `显示 Toglet` is the failure this avoids, and it
/// is the kind that only shows up on the machine of someone who does not read English.
///
/// Capped by characters rather than bytes: a cap that counted bytes would cut a Chinese label
/// mid-character.
#[tauri::command]
pub fn set_tray_labels(app: AppHandle, labels: window::TrayLabels) {
    window::set_labels(
        &app,
        &window::TrayLabels {
            show: capped(&labels.show, MAX_LABEL),
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
        // Three-byte characters. A byte cap would have produced invalid text, or panicked.
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
