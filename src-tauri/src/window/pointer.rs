//! Letting clicks through the transparent part of the window.
//!
//! The window is a strip much larger than the bar (see `geometry`), and every transparent pixel
//! of it would otherwise swallow a click on whatever is behind it - measured, not assumed: a
//! click on the strip with the window's styles untouched reached nothing behind it. So while the
//! panel is closed and the pointer is not over the bar, the window is told to ignore the cursor
//! altogether, which on Windows is the pair of extended styles that make hit-testing pass a
//! window by. That was checked with real input: with those styles set, a click on the strip
//! reached a window placed behind it, and clearing them brought the bar back.
//!
//! A window that ignores the cursor gets no hover either, so the decision cannot come from the
//! webview: something has to watch the pointer from outside. That is the poll here - the cursor's
//! position every few tens of milliseconds, against one rectangle. It is the only poll in
//! Toglet, and it exists because there is no event for "the pointer entered a window that is not
//! listening".

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use tauri::{Runtime, WebviewWindow};

use super::geometry::Placement;
use crate::diagnostics::{Level, LogRecord, Phase, log};

/// How often the pointer is looked at while the window is ignoring it.
///
/// Well under the 120ms of hover intent the design asks for before the panel opens,
/// so the gate is never what the user waits on; and slow enough that watching costs nothing
/// anyone could measure.
const POLL: Duration = Duration::from_millis(40);

/// Where the pointer can reach the surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reach {
    /// The bar's hover target in physical screen pixels, or `None` before the window has been
    /// placed.
    pub bar: Option<Placement>,
    /// The panel is open: the whole window is surface, and the pointer is over it or about to
    /// come back to it.
    pub expanded: bool,
    /// A drag is under way: the window is moving under the pointer and `bar` describes where it
    /// was, not where it is. Nothing may be let through until the drag has settled.
    pub dragging: bool,
}

impl Reach {
    /// Whether a pointer at `(x, y)`, in physical screen pixels, should reach the window.
    pub fn reaches(&self, x: f64, y: f64) -> bool {
        if self.expanded || self.dragging {
            return true;
        }
        // No rectangle yet means no basis for letting anything through: a bar that cannot be
        // reached is worse than a strip that swallows a click.
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
        // A poisoned lock holds a `Reach` that was fully written before the panic; there is no
        // half-state to recover from, so the value is taken as it is.
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Starts watching the pointer for `window`, for as long as the process lives.
///
/// Failure to start is recorded, not returned: without the watch the window swallows clicks on
/// the strip but everything else works, and the tray can still open the panel.
pub fn watch<R: Runtime>(window: WebviewWindow<R>, gate: PointerGate) {
    let started = thread::Builder::new()
        .name("toglet-pointer-gate".to_owned())
        .spawn(move || run(&window, &gate));
    if started.is_err() {
        log(&LogRecord::new(Level::Warn, "pointer_gate_not_started").with_phase(Phase::Dock));
    }
}

fn run<R: Runtime>(window: &WebviewWindow<R>, gate: &PointerGate) {
    // What the window was last told. `None` until the first decision, so the first pass sets
    // the styles either way rather than assuming the window started in a known state.
    let mut ignoring: Option<bool> = None;
    loop {
        thread::sleep(POLL);
        let Some((x, y)) = cursor(window) else {
            // The pointer could not be read this time - a window being torn down, or a platform
            // hiccup. Not a reason to change the styles, and not a reason to stop looking.
            continue;
        };
        let ignore = !gate.snapshot().reaches(x, y);
        if ignoring == Some(ignore) {
            continue;
        }
        if window.set_ignore_cursor_events(ignore).is_ok() {
            ignoring = Some(ignore);
        }
    }
}

/// The pointer's position in physical screen pixels.
///
/// Read from the platform directly rather than through the runtime: the runtime's answer is a
/// round trip through the event loop, and the event loop is busy for as long as any command on
/// the main thread is. Asked that way, the gate could not open the bar to a pointer that arrived
/// while something else was running.
#[cfg(windows)]
fn cursor<R: Runtime>(_window: &WebviewWindow<R>) -> Option<(f64, f64)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: `point` is a live, writable `POINT` for the duration of the call, which is all
    // `GetCursorPos` requires.
    let read = unsafe { GetCursorPos(&mut point) };
    (read != 0).then(|| (f64::from(point.x), f64::from(point.y)))
}

/// Not verified on a real macOS machine: the runtime's own reading, which goes through
/// the event loop.
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
        // This is the whole point: the strip is 457 wide and the bar 68 of it. A click on the
        // rest has to reach whatever is behind the window.
        let reach = Reach {
            bar: Some(bar()),
            ..Reach::default()
        };

        assert!(!reach.reaches(1600.0, 700.0));
        assert!(!reach.reaches(1880.0, 200.0));
    }

    #[test]
    fn an_open_panel_takes_the_whole_window() {
        // The panel is 348 wide beside the bar. Letting clicks through it would put them on the
        // desktop behind the account the user is about to pick.
        let reach = Reach {
            bar: Some(bar()),
            expanded: true,
            dragging: false,
        };

        assert!(reach.reaches(1600.0, 700.0));
    }

    #[test]
    fn nothing_is_let_through_while_the_window_is_being_dragged() {
        // The rectangle is where the bar *was*. Letting the pointer through mid-drag would take
        // the pointer capture, and the window, away from the user.
        let reach = Reach {
            bar: Some(bar()),
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
                expanded: true,
                dragging: false,
            }
        );
    }
}
