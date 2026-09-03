//! Where the window sits on a given monitor, and where the bar sits inside it.
//!
//! Everything here is a pure function over a work area. No window, no display server and no
//! platform call, so the rules the design fixes - flush against the edge, 60 logical pixels wide
//! at every scale factor, never over the taskbar - are checked by ordinary unit tests rather
//! than by looking at a screen.
//!
//! **The window never changes size.** It is a strip as tall as the work area and as wide as the
//! open panel plus the room its shadow needs, and it stays that way whether the panel is open or
//! not. Growing the window when the panel opened was the flash the user saw every time it did:
//! a window that grows leftward gets a new origin, and for the frame before the webview has
//! drawn at the new size the compositor shows the old picture at the new origin - the bar 373
//! pixels to the left of where it was, then back. A move and a repaint cannot be made to land on
//! the same frame, so the only way to have neither is to move nothing. What the strip
//! costs - a transparent area over the desktop - is paid for by [`super::pointer`], which lets
//! clicks through it whenever the pointer is not over the bar.

use sha2::{Digest, Sha256};

use crate::storage::DockEdge;

/// The collapsed bar, in logical pixels.
pub const BAR_WIDTH: f64 = 60.0;
pub const BAR_HEIGHT: f64 = 168.0;

/// The bar's hit area reaches this much further inward than the bar itself.
///
/// Drawn by the stylesheet as padding on the hover target, and counted here so the pointer gate
/// stops letting clicks through at the same line the stylesheet starts listening.
pub const HIT_BUFFER: f64 = 8.0;

/// The open surface: panel 348 + gap 9 + bar 60.
pub const EXPANDED_WIDTH: f64 = 417.0;

/// Room the window keeps around the surface for its shadow to fall into.
///
/// The design draws the bar and the panel on a desktop with nothing around them, so their shadows
/// (24 pixels of blur under the bar, 40 under the panel) simply fade into it. A window cut at the
/// surface's edge has nowhere for that blur to go, and what remains is a hard-edged grey band.
/// The bar is never placed closer than this to the top or bottom of the work area, and the panel
/// is kept the same distance in; the stylesheet carries the same two numbers
/// (`--tg-window-room-above` / `-below`), and a test holds them equal.
pub const ROOM_ABOVE: f64 = 24.0;
pub const ROOM_BELOW: f64 = 58.0;
/// Inward room: the blur of the panel's `0 18px 40px` shadow.
pub const ROOM_INWARD: f64 = 40.0;

/// The window's width, in logical pixels: the open surface plus the room for its shadow.
pub const WINDOW_WIDTH: f64 = EXPANDED_WIDTH + ROOM_INWARD;

/// A monitor's usable area: the full monitor minus the taskbar, dock and menu bar.
///
/// Position and size are physical pixels, which is what the platform reports and what a window
/// is positioned in. `scale` converts the design's logical pixels into them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// A rectangle in physical screen pixels: the window's, or the bar's within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Placement {
    /// Whether a point in physical screen pixels falls inside.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let left = f64::from(self.x);
        let top = f64::from(self.y);
        x >= left
            && y >= top
            && x < left + f64::from(self.width)
            && y < top + f64::from(self.height)
    }
}

/// One monitor Toglet could dock to.
#[derive(Debug, Clone, PartialEq)]
pub struct DockTarget {
    /// Stable key for "the same monitor as last time". See [`monitor_key`].
    pub id: String,
    pub area: WorkArea,
}

/// The monitor that was chosen, and whether it is the one the user last docked to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection<'a> {
    pub target: &'a DockTarget,
    /// `false` when the remembered monitor was not among the ones attached, so this is a
    /// fallback. The bar moves back into a visible area rather than staying put.
    pub remembered: bool,
}

/// Places the window against `edge` of `area`: the full height of the work area, flush against
/// the edge, [`WINDOW_WIDTH`] wide.
///
/// The height is the work area's own, which is what keeps the bar off the Windows taskbar and
/// the macOS menu bar without either of them being named here.
pub fn place(area: WorkArea, edge: DockEdge) -> Placement {
    // A work area narrower than the strip is not a reason to hang off the far side of it.
    let width = to_physical(WINDOW_WIDTH, area.scale).min(area.width.max(1));

    // i64 throughout: a work area's right edge can exceed i32 only on absurd multi-monitor
    // layouts, but the subtraction below must not be the place that finds out.
    let left = i64::from(area.x);
    let right = left + i64::from(area.width);
    let x = match edge {
        DockEdge::Left => left,
        DockEdge::Right => right - i64::from(width),
    };

    Placement {
        x: clamp_to_i32(x),
        y: area.y,
        width,
        height: area.height,
    }
}

/// The nearest offset to `vertical_offset` at which the whole bar, with the room its shadow
/// needs, fits inside `area`.
///
/// `vertical_offset` is logical pixels from the work area's vertical centre to the bar's centre,
/// positive downward - the number the settings store. The bounds are not symmetric, because the
/// room below the bar is not the room above it. The stylesheet applies the same clamp to the same
/// stored number, so the two sides agree about where the bar is without being told.
pub fn clamp_offset(area: WorkArea, vertical_offset: i32) -> i32 {
    let height = f64::from(area.height) / usable_scale(area.scale);
    let half_bar = BAR_HEIGHT / 2.0;
    let lowest = (ROOM_ABOVE + half_bar - height / 2.0).ceil();
    let highest = (height / 2.0 - ROOM_BELOW - half_bar).floor();
    let clamped = if lowest > highest {
        // A work area shorter than the bar cannot satisfy both bounds; the top wins, so the bar
        // is clipped at the bottom rather than sliding up under the menu bar.
        lowest
    } else {
        f64::from(vertical_offset).clamp(lowest, highest)
    };
    clamp_to_i32(clamped as i64)
}

/// Where the bar's hover target is on screen: the bar plus its inward hit buffer, in physical
/// pixels, for a window at `window` with the bar `vertical_offset` below the window's centre.
///
/// This is the one rectangle the pointer gate needs. It is worked out here, beside [`place`],
/// so the same arithmetic the stylesheet does in logical pixels is done once in physical ones.
pub fn bar_rect(window: Placement, edge: DockEdge, vertical_offset: i32, scale: f64) -> Placement {
    let width = to_physical(BAR_WIDTH + HIT_BUFFER, scale);
    let height = to_physical(BAR_HEIGHT, scale);
    let x = match edge {
        DockEdge::Left => i64::from(window.x),
        DockEdge::Right => i64::from(window.x) + i64::from(window.width) - i64::from(width),
    };
    let centre = i64::from(window.y)
        + i64::from(window.height) / 2
        + (f64::from(vertical_offset) * usable_scale(scale)).round() as i64;
    Placement {
        x: clamp_to_i32(x),
        y: clamp_to_i32(centre - i64::from(height) / 2),
        width,
        height,
    }
}

/// Picks the monitor to dock to: the remembered one, else the primary, else the first attached.
///
/// Returns `None` only when no monitor is attached at all, which is the one case that has no
/// honest placement.
pub fn select<'a>(
    targets: &'a [DockTarget],
    remembered: Option<&str>,
    primary: Option<&str>,
) -> Option<Selection<'a>> {
    if let Some(target) = remembered.and_then(|id| find(targets, id)) {
        return Some(Selection {
            target,
            remembered: true,
        });
    }

    let fallback = primary
        .and_then(|id| find(targets, id))
        .or_else(|| targets.first())?;
    Some(Selection {
        target: fallback,
        remembered: false,
    })
}

/// Derives the key a monitor is remembered by.
///
/// Hashed rather than stored verbatim because the platform's own name is a device path
/// (`\\.\DISPLAY1`), and metadata files hold no path-shaped strings. Equality is the only thing
/// the key is ever used for, so nothing is lost. The digest is written here rather than reused
/// from `accounts` because `window` may not depend on a business module, and the domain
/// separator keeps the two from colliding anyway.
///
/// Falls back to the monitor's geometry when the platform reports no name. That key changes if
/// the monitor is moved in the desktop layout, which costs the user a remembered position - an
/// acceptable outcome, and better than treating two nameless monitors as one.
pub fn monitor_key(name: Option<&str>, area: WorkArea) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"toglet.monitor-key.v1");
    match name {
        Some(name) => {
            hasher.update(b"name");
            hasher.update(name.as_bytes());
        }
        None => {
            hasher.update(b"geometry");
            hasher.update(format!(
                "{},{},{}x{}",
                area.x, area.y, area.width, area.height
            ));
        }
    }
    hasher
        .finalize()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Where a dragged window comes to rest.
///
/// The three things the drag decides, and the three the settings remember. Nothing else about the
/// window is stored: a drag chooses a place, not a size.
#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    pub display_id: String,
    pub edge: DockEdge,
    /// Logical pixels from the vertical centre, positive downward - the same units
    /// [`clamp_offset`] takes, so snapping a placement and placing the snap is a round trip.
    pub vertical_offset: i32,
}

/// Decides where a window released at `window` should dock.
///
/// A pure function over work areas, so "drops onto the nearer edge", "follows the pointer to a
/// second monitor" and "cannot be dragged off the bottom" are unit tests rather than something
/// only a real desktop can show.
///
/// **The bar decides, not the window.** The window is a strip much wider than the bar, so its
/// centre is a couple of hundred pixels in from the bar the user is actually dragging; the bar's
/// centre is where the user is looking. `edge`, `vertical_offset` and `scale` say where the bar
/// was inside the window when the drag began, which is where it still is.
pub fn snap(
    targets: &[DockTarget],
    window: Placement,
    edge: DockEdge,
    vertical_offset: i32,
    scale: f64,
) -> Option<Snap> {
    let bar = bar_rect(window, edge, vertical_offset, scale);
    let half_bar = i64::from(to_physical(BAR_WIDTH, scale)) / 2;
    let centre_x = match edge {
        DockEdge::Left => i64::from(bar.x) + half_bar,
        DockEdge::Right => i64::from(bar.x) + i64::from(bar.width) - half_bar,
    };
    let centre_y = i64::from(bar.y) + i64::from(bar.height) / 2;

    let target = targets
        .iter()
        .min_by_key(|target| distance_squared(target.area, centre_x, centre_y))?;
    let area = target.area;

    let left = i64::from(area.x);
    let right = left + i64::from(area.width);
    // A tie goes to the left, which only happens on an exactly even work area with the bar
    // exactly centred. Either answer is defensible; picking one keeps the function total.
    let edge = if centre_x - left <= right - centre_x {
        DockEdge::Left
    } else {
        DockEdge::Right
    };

    let area_centre = i64::from(area.y) + i64::from(area.height) / 2;
    // Clamped here rather than left for later: an offset that would be clamped later is an
    // offset that does not describe where the bar will actually be, and it is what gets stored
    // and shown at the next start.
    let offset = clamp_offset(area, to_logical(centre_y - area_centre, area.scale));

    Some(Snap {
        display_id: target.id.clone(),
        edge,
        vertical_offset: offset,
    })
}

/// Zero when the point is inside, so a monitor containing the point always beats one that does not.
fn distance_squared(area: WorkArea, x: i64, y: i64) -> i64 {
    let dx = gap(
        x,
        i64::from(area.x),
        i64::from(area.x) + i64::from(area.width),
    );
    let dy = gap(
        y,
        i64::from(area.y),
        i64::from(area.y) + i64::from(area.height),
    );
    dx * dx + dy * dy
}

fn gap(value: i64, low: i64, high: i64) -> i64 {
    if value < low {
        low - value
    } else if value > high {
        value - high
    } else {
        0
    }
}

fn find<'a>(targets: &'a [DockTarget], id: &str) -> Option<&'a DockTarget> {
    targets.iter().find(|target| target.id == id)
}

/// A non-positive or non-finite scale factor is not something to propagate as an error - the
/// window still has to be placed. 1.0 is the only defensible reading of "unknown scale".
fn usable_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

/// Converts a physical length back to logical pixels.
fn to_logical(physical: i64, scale: f64) -> i32 {
    clamp_to_i32((physical as f64 / usable_scale(scale)).round() as i64)
}

/// Converts a logical length to physical pixels.
///
/// Rounded, never truncated: a bar 59 pixels wide at 125% would leave a one pixel gap at the
/// screen edge, and the design requires the edge to be flush.
fn to_physical(logical: f64, scale: f64) -> u32 {
    (logical * usable_scale(scale)).round().max(1.0) as u32
}

fn clamp_to_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1920x1080 with a 48 pixel Windows taskbar along the bottom.
    fn work_area(scale: f64) -> WorkArea {
        WorkArea {
            x: 0,
            y: 0,
            width: (1920.0 * scale) as u32,
            height: (1080.0 * scale) as u32 - (48.0 * scale) as u32,
            scale,
        }
    }

    /// One monitor at the origin, and a second one to its right with a gap between them - the
    /// gap is what a real side-by-side pair of different heights leaves, and it is where a
    /// released window can genuinely belong to neither.
    fn two_monitors() -> Vec<DockTarget> {
        vec![
            DockTarget {
                id: "left-screen".into(),
                area: work_area(1.0),
            },
            DockTarget {
                id: "right-screen".into(),
                area: WorkArea {
                    x: 2000,
                    y: 0,
                    width: 1920,
                    height: 1032,
                    scale: 1.0,
                },
            },
        ]
    }

    /// The window docked on the right of the first monitor, then dragged by `(dx, dy)`.
    fn dragged_from_right(dx: i32, dy: i32) -> Placement {
        let placed = place(work_area(1.0), DockEdge::Right);
        Placement {
            x: placed.x + dx,
            y: placed.y + dy,
            ..placed
        }
    }

    fn bar_centre(bar: Placement) -> (i32, i32) {
        (bar.x + bar.width as i32 / 2, bar.y + bar.height as i32 / 2)
    }

    #[test]
    fn docks_flush_against_the_right_edge() {
        let placement = place(work_area(1.0), DockEdge::Right);

        assert_eq!(placement.x + placement.width as i32, 1920);
    }

    #[test]
    fn docks_flush_against_the_left_edge() {
        let placement = place(work_area(1.0), DockEdge::Left);

        assert_eq!(placement.x, 0);
    }

    #[test]
    fn spans_the_work_area_from_top_to_bottom_and_no_further() {
        // The strip is the work area's height exactly: tall enough for the bar to be dragged
        // anywhere in it, and never over the taskbar below it.
        let area = work_area(1.0);

        let placement = place(area, DockEdge::Right);

        assert_eq!(placement.y, area.y);
        assert_eq!(placement.height, area.height);
    }

    #[test]
    fn is_the_open_surface_plus_the_room_for_its_shadow_wide() {
        // 417 = panel 348 + gap 9 + bar 60, and 40 more for the panel's shadow.
        let placement = place(work_area(1.0), DockEdge::Right);

        assert_eq!(placement.width, WINDOW_WIDTH as u32);
        assert_eq!(WINDOW_WIDTH, 457.0);
    }

    #[test]
    fn keeps_the_window_within_half_a_pixel_of_its_design_width_at_every_scale() {
        // Unlike the bar's 60, 457 is not a whole number of physical pixels at 125%, so it cannot
        // be exact. The requirement is that it is never off by a visible amount.
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let placement = place(work_area(scale), DockEdge::Right);

            let error = (f64::from(placement.width) - WINDOW_WIDTH * scale).abs();
            assert!(error <= 0.5, "off by {error} physical px at {scale}x");
        }
    }

    #[test]
    fn is_the_same_size_whether_the_panel_is_open_or_not() {
        // There is no second size to compare against: `place` takes no size at all. The test
        // is here so that the day someone adds one, the reason it was removed is beside it: a
        // window that grows is a window whose old picture is shown at the new origin for a frame.
        let placement = place(work_area(1.0), DockEdge::Right);

        assert_eq!(
            placement,
            place(work_area(1.0), DockEdge::Right),
            "placement depends on nothing but the area and the edge"
        );
    }

    #[test]
    fn respects_a_work_area_that_does_not_start_at_the_origin() {
        // A second monitor to the left of the primary one, and a macOS menu bar on top.
        let area = WorkArea {
            x: -1920,
            y: 25,
            width: 1920,
            height: 1055,
            scale: 1.0,
        };

        let placement = place(area, DockEdge::Left);

        assert_eq!(placement.x, -1920);
        assert_eq!(placement.y, 25);
    }

    #[test]
    fn does_not_hang_off_a_work_area_narrower_than_itself() {
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 300,
            height: 600,
            scale: 1.0,
        };

        let placement = place(area, DockEdge::Right);

        assert_eq!(placement.x, 0);
        assert_eq!(placement.width, 300);
    }

    #[test]
    fn falls_back_to_one_when_the_platform_reports_a_nonsense_scale() {
        for scale in [0.0, -1.0, f64::NAN] {
            let area = WorkArea {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                scale,
            };

            assert_eq!(place(area, DockEdge::Right).width, WINDOW_WIDTH as u32);
        }
    }

    #[test]
    fn leaves_an_offset_that_fits_alone() {
        assert_eq!(clamp_offset(work_area(1.0), 0), 0);
        assert_eq!(clamp_offset(work_area(1.0), 120), 120);
        assert_eq!(clamp_offset(work_area(1.0), -120), -120);
    }

    #[test]
    fn keeps_the_bar_and_the_room_above_it_inside_the_work_area() {
        let area = work_area(1.0);

        let offset = clamp_offset(area, -10_000);
        let bar = bar_rect(place(area, DockEdge::Right), DockEdge::Right, offset, 1.0);

        assert_eq!(bar.y, area.y + ROOM_ABOVE as i32);
    }

    #[test]
    fn keeps_the_bar_and_the_room_below_it_off_the_taskbar() {
        let area = work_area(1.0);

        let offset = clamp_offset(area, 10_000);
        let bar = bar_rect(place(area, DockEdge::Right), DockEdge::Right, offset, 1.0);

        assert_eq!(
            bar.y + bar.height as i32,
            area.y + area.height as i32 - ROOM_BELOW as i32
        );
    }

    #[test]
    fn the_bounds_are_not_symmetric_because_the_room_is_not() {
        let area = work_area(1.0);

        let highest = clamp_offset(area, 10_000);
        let lowest = clamp_offset(area, -10_000);

        assert_eq!(highest + lowest, (ROOM_ABOVE - ROOM_BELOW) as i32);
    }

    #[test]
    fn top_aligns_rather_than_sliding_off_a_work_area_shorter_than_the_bar() {
        let area = WorkArea {
            x: 0,
            y: 100,
            width: 800,
            height: 100,
            scale: 1.0,
        };

        let offset = clamp_offset(area, 0);
        let bar = bar_rect(place(area, DockEdge::Right), DockEdge::Right, offset, 1.0);

        assert_eq!(bar.y, 100 + ROOM_ABOVE as i32);
    }

    #[test]
    fn clamps_in_logical_pixels_whatever_the_scale() {
        // The same desk at 200% has the same room for the bar in logical pixels.
        assert_eq!(
            clamp_offset(work_area(2.0), 10_000),
            clamp_offset(work_area(1.0), 10_000)
        );
    }

    #[test]
    fn centres_the_bar_on_the_work_area_when_no_offset_is_stored() {
        let area = work_area(1.0);

        let bar = bar_rect(place(area, DockEdge::Right), DockEdge::Right, 0, 1.0);

        let (_, centre_y) = bar_centre(bar);
        assert!((centre_y - (area.y + area.height as i32 / 2)).abs() <= 1);
        assert_eq!(bar.height, BAR_HEIGHT as u32);
    }

    #[test]
    fn the_hover_target_is_the_bar_plus_its_inward_buffer() {
        // The 8px buffer on the inward side is part of what opens the panel, so the
        // pointer gate has to stop letting clicks through there, not only over the bar.
        let window = place(work_area(1.0), DockEdge::Right);

        let bar = bar_rect(window, DockEdge::Right, 0, 1.0);

        assert_eq!(bar.width, (BAR_WIDTH + HIT_BUFFER) as u32);
        assert_eq!(bar.x + bar.width as i32, 1920, "flush against the edge");
    }

    #[test]
    fn mirrors_the_hover_target_for_the_left_edge() {
        let window = place(work_area(1.0), DockEdge::Left);

        let bar = bar_rect(window, DockEdge::Left, 0, 1.0);

        assert_eq!(bar.x, 0);
        assert_eq!(bar.width, (BAR_WIDTH + HIT_BUFFER) as u32);
    }

    #[test]
    fn keeps_the_bar_sixty_by_one_hundred_and_sixty_eight_logical_pixels_at_every_scale() {
        // The bar stays 60 logical pixels wide at 100 / 125 / 150 / 200%.
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let area = work_area(scale);
            let bar = bar_rect(place(area, DockEdge::Right), DockEdge::Right, 0, scale);

            assert_eq!(
                f64::from(bar.width) / scale,
                BAR_WIDTH + HIT_BUFFER,
                "at {scale}x"
            );
            assert_eq!(f64::from(bar.height) / scale, BAR_HEIGHT, "at {scale}x");
        }
    }

    #[test]
    fn applies_the_offset_in_physical_pixels() {
        // Stored logical, drawn physical: 100 logical pixels down is 200 physical at 200%.
        let area = work_area(2.0);
        let window = place(area, DockEdge::Right);

        let resting = bar_rect(window, DockEdge::Right, 0, 2.0);
        let lowered = bar_rect(window, DockEdge::Right, 100, 2.0);

        assert_eq!(lowered.y - resting.y, 200);
    }

    #[test]
    fn a_point_on_the_bar_is_inside_and_one_beside_it_is_not() {
        let bar = bar_rect(
            place(work_area(1.0), DockEdge::Right),
            DockEdge::Right,
            0,
            1.0,
        );

        assert!(bar.contains(f64::from(bar.x) + 30.0, f64::from(bar.y) + 84.0));
        assert!(!bar.contains(f64::from(bar.x) - 1.0, f64::from(bar.y) + 84.0));
        assert!(!bar.contains(
            f64::from(bar.x) + 30.0,
            f64::from(bar.y) + f64::from(bar.height)
        ));
    }

    #[test]
    fn a_window_let_go_near_the_left_lands_on_the_left_edge() {
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(-1400, 0),
            DockEdge::Right,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.edge, DockEdge::Left);
        assert_eq!(snapped.display_id, "left-screen");
    }

    #[test]
    fn a_window_let_go_near_the_right_stays_on_the_right_edge() {
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(-200, 0),
            DockEdge::Right,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.edge, DockEdge::Right);
        assert_eq!(snapped.display_id, "left-screen");
    }

    #[test]
    fn the_bar_decides_the_edge_rather_than_the_window() {
        // The window is 457 wide with the bar at its right. Dragged so that the window's centre
        // is left of the screen's but the bar is still right of it, the bar should win: it is
        // the thing the user has hold of.
        let window = dragged_from_right(-(1920 / 2 - 457 / 2 + 20), 0);
        let window_centre = window.x + window.width as i32 / 2;
        assert!(
            window_centre < 960,
            "the premise: window centre left of middle"
        );

        let snapped = snap(&two_monitors(), window, DockEdge::Right, 0, 1.0).expect("a monitor");

        assert_eq!(snapped.edge, DockEdge::Right);
    }

    #[test]
    fn a_window_dragged_onto_the_second_monitor_stays_there() {
        // The monitor is remembered as well as the edge, so this is what the next start
        // reads back.
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(600, 0),
            DockEdge::Right,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.display_id, "right-screen");
        assert_eq!(snapped.edge, DockEdge::Left);
    }

    #[test]
    fn a_window_let_go_in_the_gap_goes_to_the_nearer_monitor() {
        // Neither work area contains the bar: it is 50 past the left screen's edge and 30 short
        // of the right one's. Falling back to the first in the list would send the window to
        // the far side of the desk from where it was dropped.
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(80, 0),
            DockEdge::Right,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.display_id, "right-screen");
    }

    #[test]
    fn a_window_cannot_be_dragged_off_the_bottom() {
        let area = work_area(1.0);

        let snapped = snap(
            &two_monitors(),
            dragged_from_right(0, 5000),
            DockEdge::Right,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.vertical_offset, clamp_offset(area, 10_000));
    }

    #[test]
    fn a_window_cannot_be_dragged_off_the_top() {
        let area = work_area(1.0);

        let snapped = snap(
            &two_monitors(),
            dragged_from_right(0, -5000),
            DockEdge::Right,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.vertical_offset, clamp_offset(area, -10_000));
    }

    #[test]
    fn a_drag_that_moves_nothing_changes_nothing() {
        // The round trip is the whole contract: what a drag stores has to be what the next start
        // reproduces, and a drag of zero distance has to store what was already there.
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(0, 0),
            DockEdge::Right,
            120,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.edge, DockEdge::Right);
        assert_eq!(snapped.vertical_offset, 120);
    }

    #[test]
    fn a_drag_adds_to_the_offset_the_bar_already_had() {
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(0, 50),
            DockEdge::Right,
            120,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.vertical_offset, 170);
    }

    #[test]
    fn the_offset_a_drag_stores_is_in_logical_pixels() {
        // Stored logical, applied logical. A physical offset would move the bar twice as far on
        // the next start at 200%.
        let area = work_area(2.0);
        let targets = vec![DockTarget {
            id: "hidpi".into(),
            area,
        }];
        let placed = place(area, DockEdge::Right);
        let window = Placement {
            y: placed.y + 200,
            ..placed
        };

        let snapped = snap(&targets, window, DockEdge::Right, 0, 2.0).expect("a monitor");

        assert_eq!(snapped.vertical_offset, 100);
    }

    #[test]
    fn a_drag_with_no_monitor_attached_decides_nothing() {
        assert_eq!(
            snap(&[], dragged_from_right(0, 0), DockEdge::Right, 0, 1.0),
            None
        );
    }

    fn target(id: &str) -> DockTarget {
        DockTarget {
            id: id.to_owned(),
            area: work_area(1.0),
        }
    }

    #[test]
    fn returns_to_the_remembered_monitor_when_it_is_still_attached() {
        let targets = [target("a"), target("b")];

        let selection = select(&targets, Some("b"), Some("a")).expect("a monitor is attached");

        assert_eq!(selection.target.id, "b");
        assert!(selection.remembered);
    }

    #[test]
    fn moves_to_the_primary_monitor_when_the_remembered_one_is_gone() {
        // An unplugged monitor must not leave the bar somewhere invisible.
        let targets = [target("a"), target("b")];

        let selection = select(&targets, Some("gone"), Some("a")).expect("a monitor is attached");

        assert_eq!(selection.target.id, "a");
        assert!(!selection.remembered);
    }

    #[test]
    fn takes_the_first_monitor_when_the_platform_names_no_primary() {
        let targets = [target("a"), target("b")];

        let selection = select(&targets, None, None).expect("a monitor is attached");

        assert_eq!(selection.target.id, "a");
        assert!(!selection.remembered);
    }

    #[test]
    fn reports_no_selection_when_nothing_is_attached() {
        assert!(select(&[], Some("a"), Some("a")).is_none());
    }

    #[test]
    fn gives_two_monitors_with_different_names_different_keys() {
        let area = work_area(1.0);

        assert_ne!(
            monitor_key(Some("\\\\.\\DISPLAY1"), area),
            monitor_key(Some("\\\\.\\DISPLAY2"), area)
        );
    }

    #[test]
    fn gives_the_same_monitor_the_same_key_across_runs() {
        let area = work_area(1.0);

        assert_eq!(
            monitor_key(Some("\\\\.\\DISPLAY1"), area),
            monitor_key(Some("\\\\.\\DISPLAY1"), area)
        );
    }

    #[test]
    fn keeps_the_platform_name_out_of_the_key() {
        // The key is persisted, and a device path in a metadata file is forbidden.
        let key = monitor_key(Some("\\\\.\\DISPLAY1"), work_area(1.0));

        assert!(!key.contains("DISPLAY"));
        assert!(!key.contains('\\'));
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn distinguishes_nameless_monitors_by_where_they_are() {
        let left = WorkArea {
            x: -1920,
            ..work_area(1.0)
        };

        assert_ne!(monitor_key(None, left), monitor_key(None, work_area(1.0)));
    }
}
