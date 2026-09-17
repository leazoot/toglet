//! Lets clicks through the transparent part of the docked window.
//!
//! Transparent pixels otherwise swallow clicks, so the window ignores the cursor unless the panel
//! is open or the pointer is over the bar. An ignoring window gets no hover events, so the cursor
//! position is polled from outside the webview.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tauri::{Runtime, WebviewWindow};

use super::geometry::Placement;
use crate::diagnostics::{Level, LogRecord, Phase, log};

/// Well under the 120ms hover intent before the panel opens, and cheap.
const POLL: Duration = Duration::from_millis(40);

/// Polls before an unchanged decision is re-sent to the window (about once a second).
///
/// On macOS the call is dispatched to the main thread and reports success before it runs, so a
/// lost call (such as one made before the window is shown) would otherwise never be retried.
const REASSERT_EVERY: u32 = 25;

/// Where the pointer can reach the surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reach {
    /// The bar's hover target in physical screen pixels; `None` before the window is placed.
    pub bar: Option<Placement>,
    /// The panel is open, so the panel's rectangle is surface too.
    pub expanded: bool,
    /// The open panel's rectangle in physical screen pixels, as the interface measured it.
    ///
    /// `None` while closed, and also while open if no measurement arrived: the window is a
    /// full-height strip, so treating "open" as "the whole window" blanks a column of the screen.
    pub panel: Option<Placement>,
    /// `bar` is stale while dragging, so nothing is let through until the drag settles.
    pub dragging: bool,
}

impl Reach {
    /// Whether a pointer at `(x, y)`, in physical screen pixels, should reach the window.
    pub fn reaches(&self, x: f64, y: f64) -> bool {
        if self.dragging {
            return true;
        }
        if self.expanded {
            // The window is a full-height strip, so an open panel must claim its own rectangle
            // rather than the whole window - the rest of that column belongs to other apps.
            // Without a measurement, the old whole-window behaviour: an unreachable panel is
            // worse than a swallowed click.
            return match self.panel {
                Some(panel) => {
                    panel.contains(x, y) || self.bar.is_some_and(|bar| bar.contains(x, y))
                }
                None => true,
            };
        }
        // No rectangle yet: an unreachable bar is worse than a swallowed click.
        self.bar.is_none_or(|bar| bar.contains(x, y))
    }
}

/// The shared decision the poll reads and the commands write.
#[derive(Clone, Default)]
pub struct PointerGate(Arc<Mutex<Reach>>);

impl PointerGate {
    pub fn update(&self, change: impl FnOnce(&mut Reach)) {
        change(&mut self.lock());
    }

    pub fn snapshot(&self) -> Reach {
        *self.lock()
    }

    fn lock(&self) -> MutexGuard<'_, Reach> {
        // A poisoned lock still holds a fully written `Reach`; there is no half-state to recover.
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Starts watching the pointer for `window`, for as long as the process lives.
///
/// Failure to start is logged, not returned: the strip then swallows clicks, but the rest works.
pub fn watch<R: Runtime>(window: WebviewWindow<R>, gate: PointerGate) {
    let started = thread::Builder::new()
        .name("toglet-pointer-gate".to_owned())
        .spawn(move || run(&window, &gate));
    if started.is_err() {
        log(&LogRecord::new(Level::Warn, "pointer_gate_not_started").with_phase(Phase::Dock));
    }
}

fn run<R: Runtime>(window: &WebviewWindow<R>, gate: &PointerGate) {
    // `None` until the first decision, so the first pass always applies it.
    let mut ignoring: Option<bool> = None;
    let mut polls_since_applied: u32 = 0;
    loop {
        thread::sleep(POLL);
        let Some((x, y)) = cursor(window) else {
            // A transient read failure (such as window teardown) changes nothing.
            continue;
        };
        let ignore = !gate.snapshot().reaches(x, y);
        polls_since_applied += 1;
        if ignoring == Some(ignore) && polls_since_applied < REASSERT_EVERY {
            continue;
        }
        if window.set_ignore_cursor_events(ignore).is_ok() {
            ignoring = Some(ignore);
            polls_since_applied = 0;
        }
    }
}

/// The pointer's position in physical screen pixels.
///
/// Read from Win32 directly: the runtime's reading round-trips through the event loop, which is
/// blocked while any main-thread command runs.
#[cfg(windows)]
fn cursor<R: Runtime>(_window: &WebviewWindow<R>) -> Option<(f64, f64)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a live, writable `POINT` for the duration of the call.
    let read = unsafe { GetCursorPos(&mut point) };
    (read != 0).then(|| (f64::from(point.x), f64::from(point.y)))
}

/// Unverified on macOS: uses the runtime's reading, which goes through the event loop.
#[cfg(not(windows))]
fn cursor<R: Runtime>(window: &WebviewWindow<R>) -> Option<(f64, f64)> {
    window
        .cursor_position()
        .ok()
        .map(|position| (position.x, position.y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> Placement {
        Placement {
            x: 1852,
            y: 640,
            width: 68,
            height: 168,
        }
    }

    #[test]
    fn a_pointer_over_the_bar_reaches_the_window() {
        let reach = Reach {
            bar: Some(bar()),
            ..Reach::default()
        };

        assert!(reach.reaches(1880.0, 700.0));
    }

    #[test]
    fn a_pointer_over_the_transparent_strip_does_not() {
        // The strip is 457 wide and the bar 68 of it; clicks on the rest must pass through.
        let reach = Reach {
            bar: Some(bar()),
            ..Reach::default()
        };

        assert!(!reach.reaches(1600.0, 700.0));
        assert!(!reach.reaches(1880.0, 200.0));
    }

    fn panel() -> Placement {
        Placement {
            x: 1463,
            y: 500,
            width: 348,
            height: 300,
        }
    }

    #[test]
    fn an_open_panel_takes_its_own_rectangle_and_the_bar() {
        // Clicks through the open panel would land on the desktop behind it.
        let reach = Reach {
            bar: Some(bar()),
            panel: Some(panel()),
            expanded: true,
            dragging: false,
        };

        assert!(reach.reaches(1600.0, 700.0), "inside the panel");
        assert!(reach.reaches(1880.0, 700.0), "the bar stays reachable");
    }

    #[test]
    fn an_open_panel_lets_the_rest_of_the_strip_through() {
        // The window is a full-height strip: everything in that column outside the panel and the
        // bar belongs to whatever app is behind it.
        let reach = Reach {
            bar: Some(bar()),
            panel: Some(panel()),
            expanded: true,
            dragging: false,
        };

        assert!(!reach.reaches(1600.0, 60.0), "above the panel");
        assert!(!reach.reaches(1600.0, 1000.0), "below the panel");
    }

    #[test]
    fn an_open_panel_that_was_never_measured_still_takes_the_whole_window() {
        let reach = Reach {
            bar: Some(bar()),
            panel: None,
            expanded: true,
            dragging: false,
        };

        assert!(reach.reaches(1600.0, 60.0));
    }

    #[test]
    fn nothing_is_let_through_while_the_window_is_being_dragged() {
        // The rectangle is where the bar was; letting the pointer through mid-drag loses capture.
        let reach = Reach {
            bar: Some(bar()),
            panel: None,
            expanded: false,
            dragging: true,
        };

        assert!(reach.reaches(100.0, 100.0));
    }

    #[test]
    fn nothing_is_let_through_before_the_window_has_been_placed() {
        assert!(Reach::default().reaches(0.0, 0.0));
    }

    #[test]
    fn the_gate_hands_back_what_was_written() {
        let gate = PointerGate::default();

        gate.update(|reach| reach.expanded = true);
        gate.update(|reach| reach.bar = Some(bar()));

        assert_eq!(
            gate.snapshot(),
            Reach {
                bar: Some(bar()),
                panel: None,
                expanded: true,
                dragging: false,
            }
        );
    }
}
