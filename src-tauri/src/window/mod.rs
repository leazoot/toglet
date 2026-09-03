//! Edge docking, multi-monitor placement and always-on-top behaviour.
//!
//! Depends on: `storage`, `diagnostics`.
//!
//! The window is a fixed strip against one screen edge (`geometry`), the bar's position inside it
//! is a stored offset, and the strip's transparent part lets clicks through whenever the pointer
//! is not over the bar (`pointer`). The tray menu is `tray`.
//!
//! This module holds no credential and no account state. Its whole job is to answer "where does
//! the window go", which is why the geometry is a pure function and the platform sits behind a
//! trait.

mod geometry;
mod platform;
mod pointer;
mod tray;

pub use geometry::{
    BAR_HEIGHT, BAR_WIDTH, DockTarget, EXPANDED_WIDTH, HIT_BUFFER, Placement, ROOM_ABOVE,
    ROOM_BELOW, ROOM_INWARD, Selection, Snap, WINDOW_WIDTH, WorkArea, bar_rect, clamp_offset,
    monitor_key, place, select, snap,
};
pub use platform::{DockPlatform, TauriDock};
pub use pointer::{PointerGate, Reach, watch as watch_pointer};
pub use tray::{
    TRAY_REFRESH_EVENT, TRAY_SETTINGS_EVENT, TRAY_SHOW_EVENT, TrayLabels, install as install_tray,
    set_labels, set_summary,
};

use tauri::{Manager, Runtime, WebviewWindow};

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};
use crate::storage::{AppSettings, DockEdge};

/// Where the window ended up, on which monitor, and where the bar is inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct DockOutcome {
    /// The monitor the window is on. The caller persists this so the next start returns here.
    pub display_id: String,
    pub edge: DockEdge,
    /// `true` when the remembered monitor was not attached and the window was moved somewhere
    /// visible instead.
    pub moved_to_fallback: bool,
    pub placement: Placement,
    /// The offset the bar was actually given: the stored one, clamped so the bar fits this
    /// monitor. The caller persists it when it differs, so what is stored is always where the
    /// bar is.
    pub vertical_offset: i32,
    /// The bar's hover target on screen, for the pointer gate.
    pub bar: Placement,
}

/// Docks the window against the given edge of the remembered monitor.
///
/// Fails only when the platform reports no monitor at all. Every other shortfall - a monitor that
/// was unplugged, an offset that points off-screen - resolves to a visible position, because a
/// bar the user cannot reach is worse than one that is not where they left it.
pub fn dock(
    platform: &dyn DockPlatform,
    edge: DockEdge,
    display_id: Option<&str>,
    vertical_offset: i32,
    always_on_top: bool,
) -> Result<DockOutcome> {
    let targets = platform.monitors()?;
    let primary = platform.primary()?;
    let selection = select(&targets, display_id, primary.as_deref()).ok_or_else(no_display)?;
    let area = selection.target.area;

    let placement = place(area, edge);
    platform.apply(placement, always_on_top)?;

    let vertical_offset = clamp_offset(area, vertical_offset);
    Ok(DockOutcome {
        display_id: selection.target.id.clone(),
        edge,
        moved_to_fallback: !selection.remembered,
        placement,
        vertical_offset,
        bar: bar_rect(placement, edge, vertical_offset, area.scale),
    })
}

/// Docks a real Tauri window according to the stored settings, and tells the pointer gate where
/// the bar now is.
pub fn dock_window<R: Runtime>(
    window: &WebviewWindow<R>,
    settings: &AppSettings,
) -> Result<DockOutcome> {
    let outcome = dock(
        &TauriDock::new(window),
        settings.dock_edge,
        settings.display_id.as_deref(),
        settings.vertical_offset,
        settings.always_on_top,
    )?;
    // The gate learns the bar's rectangle from the same placement the window was given, so the
    // two cannot disagree about where the bar is.
    if let Some(gate) = window.app_handle().try_state::<PointerGate>() {
        gate.update(|reach| reach.bar = Some(outcome.bar));
    }
    Ok(outcome)
}

/// The window's rectangle right now, in physical pixels.
pub fn current_placement<R: Runtime>(window: &WebviewWindow<R>) -> Result<Placement> {
    let position = window.outer_position().map_err(|_| unreadable_window())?;
    let size = window.inner_size().map_err(|_| unreadable_window())?;
    Ok(Placement {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}

/// Decides where a dragged window belongs, against the monitors the platform reports.
///
/// `edge`, `vertical_offset` and `scale` describe where the bar sat inside the window when the
/// drag began. Split from the command so the decision can be exercised against a fake platform:
/// the command itself only reads a rectangle and writes settings, and neither of those is where
/// the interesting mistakes live.
pub fn settle<P: DockPlatform>(
    platform: &P,
    window: Placement,
    edge: DockEdge,
    vertical_offset: i32,
    scale: f64,
) -> Result<Snap> {
    let targets = platform.monitors()?;
    snap(&targets, window, edge, vertical_offset, scale).ok_or_else(no_display)
}

fn unreadable_window() -> TogletError {
    TogletError::new(ErrorCode::Internal, Phase::Dock, true, UserAction::Retry)
        .with_detail("the window did not report its current position")
}

fn no_display() -> TogletError {
    TogletError::new(
        ErrorCode::DisplayUnavailable,
        Phase::Dock,
        true,
        UserAction::Retry,
    )
}

#[cfg(test)]
mod tests {
    use super::platform::fake::RecordingDock;
    use super::*;

    fn target(id: &str, x: i32) -> DockTarget {
        DockTarget {
            id: id.to_owned(),
            area: WorkArea {
                x,
                y: 0,
                width: 1920,
                height: 1032,
                scale: 1.0,
            },
        }
    }

    /// The strip as it would sit on the right of monitor `b`, dragged by `(dx, dy)`.
    fn dragged(dx: i32, dy: i32) -> Placement {
        let placed = place(target("b", 1920).area, DockEdge::Right);
        Placement {
            x: placed.x + dx,
            y: placed.y + dy,
            ..placed
        }
    }

    #[test]
    fn a_drag_that_ends_over_a_second_monitor_settles_there() {
        // The pair of monitors is the case the geometry alone cannot demonstrate: `settle` has to
        // ask the platform which ones are attached before it can decide anything.
        let platform = RecordingDock::new(vec![target("a", 0), target("b", 1920)], Some("a"));

        // 2000 to the left puts the bar's centre at x=1810, on monitor `a`; the window's own
        // centre would already have been there 200 pixels earlier, which is the difference
        // between deciding by the bar and deciding by the window.
        let landed = settle(&platform, dragged(-2000, 0), DockEdge::Right, 0, 1.0)
            .expect("a monitor is attached");

        assert_eq!(landed.display_id, "a");
        assert_eq!(landed.edge, DockEdge::Right);
    }

    #[test]
    fn a_drag_decides_nothing_when_no_monitor_is_attached() {
        // Not a silent no-op: leaving the window wherever the drag left it would strand it, and
        // saying so is what lets the interface offer "move to primary display".
        let platform = RecordingDock::new(Vec::new(), None);

        let landed = settle(&platform, dragged(0, 0), DockEdge::Right, 0, 1.0);

        assert!(landed.is_err());
    }

    #[test]
    fn places_the_window_on_the_remembered_monitor() {
        let platform = RecordingDock::new(vec![target("a", 0), target("b", 1920)], Some("a"));

        let outcome =
            dock(&platform, DockEdge::Right, Some("b"), 0, true).expect("a monitor is attached");

        assert_eq!(outcome.display_id, "b");
        assert!(!outcome.moved_to_fallback);
        assert_eq!(outcome.placement.x + outcome.placement.width as i32, 3840);
    }

    #[test]
    fn moves_back_into_view_when_the_remembered_monitor_is_unplugged() {
        // The bar returns to somewhere visible instead of staying on a monitor that
        // is no longer there.
        let platform = RecordingDock::new(vec![target("a", 0)], Some("a"));

        let outcome =
            dock(&platform, DockEdge::Right, Some("b"), 0, true).expect("a monitor is attached");

        assert_eq!(outcome.display_id, "a");
        assert!(outcome.moved_to_fallback);
        assert!(outcome.placement.x < 1920, "still off to the right");
    }

    #[test]
    fn reports_the_offset_it_actually_used_when_the_stored_one_does_not_fit() {
        // An offset that was fine on a tall monitor may not fit a shorter one. What comes back
        // is where the bar is, so the caller can store that instead of the number that is not.
        let platform = RecordingDock::new(vec![target("a", 0)], Some("a"));

        let outcome =
            dock(&platform, DockEdge::Right, None, 10_000, true).expect("a monitor is attached");

        assert_eq!(
            outcome.vertical_offset,
            clamp_offset(target("a", 0).area, 10_000)
        );
        assert!(
            outcome.bar.y + outcome.bar.height as i32 <= 1032 - ROOM_BELOW as i32,
            "the bar the gate is told about is inside the work area"
        );
    }

    #[test]
    fn asks_the_platform_to_keep_the_bar_on_top_when_the_setting_says_so() {
        let platform = RecordingDock::new(vec![target("a", 0)], Some("a"));

        dock(&platform, DockEdge::Right, None, 0, true).expect("a monitor is attached");

        let applied = platform.applied.borrow();
        assert_eq!(applied.len(), 1);
        assert!(applied[0].1);
    }

    #[test]
    fn honours_always_on_top_being_turned_off() {
        let platform = RecordingDock::new(vec![target("a", 0)], Some("a"));

        dock(&platform, DockEdge::Right, None, 0, false).expect("a monitor is attached");

        assert!(!platform.applied.borrow()[0].1);
    }

    #[test]
    fn reports_a_display_failure_rather_than_placing_the_window_nowhere() {
        let platform = RecordingDock::new(Vec::new(), None);

        let error = dock(&platform, DockEdge::Right, None, 0, true).expect_err("no monitors");

        assert_eq!(error.code(), ErrorCode::DisplayUnavailable);
        assert!(
            platform.applied.borrow().is_empty(),
            "nothing should have been moved"
        );
    }

    #[test]
    fn docks_to_the_left_edge_when_that_is_the_stored_side() {
        let platform = RecordingDock::new(vec![target("a", 0)], Some("a"));

        let outcome =
            dock(&platform, DockEdge::Left, None, 0, true).expect("a monitor is attached");

        assert_eq!(outcome.placement.x, 0);
        assert_eq!(outcome.edge, DockEdge::Left);
        assert_eq!(outcome.bar.x, 0, "the hover target hugs the left edge too");
    }
}
