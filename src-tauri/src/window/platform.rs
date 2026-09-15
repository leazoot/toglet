//! The platform boundary for docking.
//!
//! Every call that reaches a real display server or a real window goes through [`DockPlatform`],
//! so the docking logic above it stays testable and free of `cfg(target_os)` branches.

#[cfg(not(windows))]
use tauri::{PhysicalPosition, PhysicalSize};
use tauri::{Runtime, WebviewWindow};

use super::geometry::{DockTarget, Placement, WorkArea, monitor_key};
use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// What docking needs from the windowing system.
pub trait DockPlatform {
    /// Attached monitors, each with its area minus the taskbar, dock and menu bar.
    fn monitors(&self) -> Result<Vec<DockTarget>>;

    /// The primary monitor's key, when the platform names one.
    fn primary(&self) -> Result<Option<String>>;

    /// Moves and resizes the window, and sets whether it floats above other windows.
    fn apply(&self, placement: Placement, always_on_top: bool) -> Result<()>;
}

/// [`DockPlatform`] over a real Tauri window.
pub struct TauriDock<'a, R: Runtime> {
    window: &'a WebviewWindow<R>,
}

impl<'a, R: Runtime> TauriDock<'a, R> {
    pub fn new(window: &'a WebviewWindow<R>) -> Self {
        Self { window }
    }
}

impl<R: Runtime> DockPlatform for TauriDock<'_, R> {
    fn monitors(&self) -> Result<Vec<DockTarget>> {
        let monitors = self.window.available_monitors().map_err(display_error)?;
        Ok(monitors.iter().map(target).collect())
    }

    fn primary(&self) -> Result<Option<String>> {
        let primary = self.window.primary_monitor().map_err(display_error)?;
        Ok(primary.as_ref().map(|monitor| target(monitor).id))
    }

    #[cfg(windows)]
    fn apply(&self, placement: Placement, always_on_top: bool) -> Result<()> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
        };

        // Position and size in one call: as two calls, a frame shows the resized window before it
        // moves, partly past the screen edge, which flashes.
        let handle = self.window.hwnd().map_err(display_error)?;
        let width = i32::try_from(placement.width).unwrap_or(i32::MAX);
        let height = i32::try_from(placement.height).unwrap_or(i32::MAX);
        // SAFETY: `handle` is the live top-level window this struct borrows; the only pointer
        // argument, insert-after, is null and ignored under `SWP_NOZORDER`.
        let moved = unsafe {
            SetWindowPos(
                handle.0,
                std::ptr::null_mut(),
                placement.x,
                placement.y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
        if moved == 0 {
            return Err(display_unavailable());
        }
        self.window
            .set_always_on_top(always_on_top)
            .map_err(display_error)
    }

    /// Unverified on macOS; one call would be better than two, as on Windows.
    #[cfg(not(windows))]
    fn apply(&self, placement: Placement, always_on_top: bool) -> Result<()> {
        // Size before position: a window resized after positioning grows away from its anchor and
        // leaves a gap at the screen edge.
        self.window
            .set_size(PhysicalSize::new(placement.width, placement.height))
            .map_err(display_error)?;
        self.window
            .set_position(PhysicalPosition::new(placement.x, placement.y))
            .map_err(display_error)?;
        self.window
            .set_always_on_top(always_on_top)
            .map_err(display_error)
    }
}

fn target(monitor: &tauri::Monitor) -> DockTarget {
    let work_area = monitor.work_area();
    let area = WorkArea {
        x: work_area.position.x,
        y: work_area.position.y,
        width: work_area.size.width,
        height: work_area.size.height,
        scale: monitor.scale_factor(),
    };
    DockTarget {
        id: monitor_key(monitor.name().map(String::as_str), area),
        area,
    }
}

/// Drops the runtime's error, whose `Display` form can name a window handle and an OS path.
fn display_error(_: tauri::Error) -> TogletError {
    display_unavailable()
}

fn display_unavailable() -> TogletError {
    TogletError::new(
        ErrorCode::DisplayUnavailable,
        Phase::Dock,
        true,
        UserAction::Retry,
    )
}

#[cfg(test)]
pub(crate) mod fake {
    use std::cell::RefCell;

    use super::{DockPlatform, DockTarget, Placement, Result};

    /// An in-memory [`DockPlatform`] that records what it was asked to do.
    pub(crate) struct RecordingDock {
        pub monitors: Vec<DockTarget>,
        pub primary: Option<String>,
        pub applied: RefCell<Vec<(Placement, bool)>>,
    }

    impl RecordingDock {
        pub fn new(monitors: Vec<DockTarget>, primary: Option<&str>) -> Self {
            Self {
                monitors,
                primary: primary.map(str::to_owned),
                applied: RefCell::new(Vec::new()),
            }
        }
    }

    impl DockPlatform for RecordingDock {
        fn monitors(&self) -> Result<Vec<DockTarget>> {
            Ok(self.monitors.clone())
        }

        fn primary(&self) -> Result<Option<String>> {
            Ok(self.primary.clone())
        }

        fn apply(&self, placement: Placement, always_on_top: bool) -> Result<()> {
            self.applied.borrow_mut().push((placement, always_on_top));
            Ok(())
        }
    }
}
