//! Where the window sits on a monitor and where the bar sits inside it, as pure functions.
//!
//! The window never changes size: a window growing leftward gets a new origin, and for one frame
//! the compositor shows the old picture there. The transparent strip this leaves over the desktop
//! lets clicks through via [`super::pointer`].

use sha2::{Digest, Sha256};

use crate::storage::{DockEdge, DockShape};

/// The collapsed bar, in logical pixels.
pub const BAR_WIDTH: f64 = 60.0;
pub const BAR_HEIGHT: f64 = 168.0;
/// The ring form: as wide as the bar and this tall. Must equal the stylesheet's
/// `--tg-ring-form-height`; `tokens.test.ts` holds the two equal.
pub const RING_HEIGHT: f64 = 72.0;

/// The collapsed surface's height in logical pixels; the width is [`BAR_WIDTH`] for both shapes.
pub fn bar_height(shape: DockShape) -> f64 {
    match shape {
        DockShape::Bar => BAR_HEIGHT,
        DockShape::Ring => RING_HEIGHT,
    }
}

/// Inward hit padding the stylesheet draws on the hover target, counted so the pointer gate matches.
pub const HIT_BUFFER: f64 = 8.0;

/// The open surface: panel 348 + gap 9 + bar 60.
pub const EXPANDED_WIDTH: f64 = 417.0;

/// Room kept around the surface for its shadow blur, which a window edge would cut into a hard band.
///
/// Must equal the stylesheet's `--tg-window-room-above` / `-below`; a test holds them equal.
pub const ROOM_ABOVE: f64 = 24.0;
pub const ROOM_BELOW: f64 = 58.0;
/// Inward room: the blur of the panel's `0 18px 40px` shadow.
pub const ROOM_INWARD: f64 = 40.0;

/// The window's width, in logical pixels: the open surface plus the room for its shadow.
pub const WINDOW_WIDTH: f64 = EXPANDED_WIDTH + ROOM_INWARD;

/// A monitor's area minus the taskbar, dock and menu bar, in physical pixels.
///
/// `scale` converts logical pixels into physical ones.
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
    /// `false` when the remembered monitor is not attached and this is a fallback.
    pub remembered: bool,
}

/// Places the window flush against `edge` of `area`, the work area's full height and
/// [`WINDOW_WIDTH`] wide; the work area keeps it off the taskbar and menu bar.
pub fn place(area: WorkArea, edge: DockEdge) -> Placement {
    // A work area narrower than the strip must not leave it hanging off the far side.
    let width = to_physical(WINDOW_WIDTH, area.scale).min(area.width.max(1));

    // i64 so the subtraction cannot overflow on extreme multi-monitor layouts.
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

/// The nearest offset at which the bar and its shadow room fit inside `area`.
///
/// Offsets are logical pixels from the work area's vertical centre to the bar's, positive
/// downward. The stylesheet applies the same clamp to the same stored number.
pub fn clamp_offset(area: WorkArea, shape: DockShape, vertical_offset: i32) -> i32 {
    let height = f64::from(area.height) / usable_scale(area.scale);
    let half_bar = bar_height(shape) / 2.0;
    let lowest = (ROOM_ABOVE + half_bar - height / 2.0).ceil();
    let highest = (height / 2.0 - ROOM_BELOW - half_bar).floor();
    let clamped = if lowest > highest {
        // Too short for both bounds: the top wins, so the bar is clipped at the bottom rather
        // than sliding under the menu bar.
        lowest
    } else {
        f64::from(vertical_offset).clamp(lowest, highest)
    };
    clamp_to_i32(clamped as i64)
}

/// The bar plus its inward hit buffer in physical pixels: the rectangle the pointer gate uses.
pub fn bar_rect(
    window: Placement,
    edge: DockEdge,
    shape: DockShape,
    vertical_offset: i32,
    scale: f64,
) -> Placement {
    let width = to_physical(BAR_WIDTH + HIT_BUFFER, scale);
    let height = to_physical(bar_height(shape), scale);
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

/// The remembered monitor, else the primary, else the first; `None` only when none is attached.
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
/// Hashed because platform names are device paths (`\\.\DISPLAY1`), which metadata must not hold.
/// Nameless monitors are keyed by geometry, so moving one in the layout loses its position.
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
#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    pub display_id: String,
    pub edge: DockEdge,
    /// Same units as [`clamp_offset`], so snapping and placing round-trip.
    pub vertical_offset: i32,
}

/// Decides where a window released at `window` should dock.
///
/// The bar's centre decides, not the much wider window's. `edge`, `vertical_offset` and `scale`
/// locate the bar inside the window.
pub fn snap(
    targets: &[DockTarget],
    window: Placement,
    edge: DockEdge,
    shape: DockShape,
    vertical_offset: i32,
    scale: f64,
) -> Option<Snap> {
    let bar = bar_rect(window, edge, shape, vertical_offset, scale);
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
    // An exact tie goes to the left.
    let edge = if centre_x - left <= right - centre_x {
        DockEdge::Left
    } else {
        DockEdge::Right
    };

    let area_centre = i64::from(area.y) + i64::from(area.height) / 2;
    // Clamped now so the stored offset is where the bar will actually be at the next start.
    let offset = clamp_offset(area, shape, to_logical(centre_y - area_centre, area.scale));

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

/// Treats a non-positive or non-finite scale as 1.0 rather than failing placement.
fn usable_scale(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn to_logical(physical: i64, scale: f64) -> i32 {
    clamp_to_i32((physical as f64 / usable_scale(scale)).round() as i64)
}

/// Rounded, not truncated, so fractional scales leave no one-pixel gap at the screen edge.
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

    /// Two side-by-side monitors with a gap, where a released window belongs to neither.
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
        // Exactly the work area's height: room to drag the bar anywhere, never over the taskbar.
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
        // 457 is not a whole number of physical pixels at 125%, so it cannot be exact.
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let placement = place(work_area(scale), DockEdge::Right);

            let error = (f64::from(placement.width) - WINDOW_WIDTH * scale).abs();
            assert!(error <= 0.5, "off by {error} physical px at {scale}x");
        }
    }

    #[test]
    fn is_the_same_size_whether_the_panel_is_open_or_not() {
        // `place` takes no size. Do not add one: a growing window shows its old picture at the
        // new origin for a frame.
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
        assert_eq!(clamp_offset(work_area(1.0), DockShape::Bar, 0), 0);
        assert_eq!(clamp_offset(work_area(1.0), DockShape::Bar, 120), 120);
        assert_eq!(clamp_offset(work_area(1.0), DockShape::Bar, -120), -120);
    }

    #[test]
    fn keeps_the_bar_and_the_room_above_it_inside_the_work_area() {
        let area = work_area(1.0);

        let offset = clamp_offset(area, DockShape::Bar, -10_000);
        let bar = bar_rect(
            place(area, DockEdge::Right),
            DockEdge::Right,
            DockShape::Bar,
            offset,
            1.0,
        );

        assert_eq!(bar.y, area.y + ROOM_ABOVE as i32);
    }

    #[test]
    fn keeps_the_bar_and_the_room_below_it_off_the_taskbar() {
        let area = work_area(1.0);

        let offset = clamp_offset(area, DockShape::Bar, 10_000);
        let bar = bar_rect(
            place(area, DockEdge::Right),
            DockEdge::Right,
            DockShape::Bar,
            offset,
            1.0,
        );

        assert_eq!(
            bar.y + bar.height as i32,
            area.y + area.height as i32 - ROOM_BELOW as i32
        );
    }

    #[test]
    fn the_bounds_are_not_symmetric_because_the_room_is_not() {
        let area = work_area(1.0);

        let highest = clamp_offset(area, DockShape::Bar, 10_000);
        let lowest = clamp_offset(area, DockShape::Bar, -10_000);

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

        let offset = clamp_offset(area, DockShape::Bar, 0);
        let bar = bar_rect(
            place(area, DockEdge::Right),
            DockEdge::Right,
            DockShape::Bar,
            offset,
            1.0,
        );

        assert_eq!(bar.y, 100 + ROOM_ABOVE as i32);
    }

    #[test]
    fn clamps_in_logical_pixels_whatever_the_scale() {
        // The same desk at 200% has the same room for the bar in logical pixels.
        assert_eq!(
            clamp_offset(work_area(2.0), DockShape::Bar, 10_000),
            clamp_offset(work_area(1.0), DockShape::Bar, 10_000)
        );
    }

    #[test]
    fn centres_the_bar_on_the_work_area_when_no_offset_is_stored() {
        let area = work_area(1.0);

        let bar = bar_rect(
            place(area, DockEdge::Right),
            DockEdge::Right,
            DockShape::Bar,
            0,
            1.0,
        );

        let (_, centre_y) = bar_centre(bar);
        assert!((centre_y - (area.y + area.height as i32 / 2)).abs() <= 1);
        assert_eq!(bar.height, BAR_HEIGHT as u32);
    }

    #[test]
    fn the_hover_target_is_the_bar_plus_its_inward_buffer() {
        // The 8px buffer on the inward side is part of what opens the panel, so the
        // pointer gate has to stop letting clicks through there, not only over the bar.
        let window = place(work_area(1.0), DockEdge::Right);

        let bar = bar_rect(window, DockEdge::Right, DockShape::Bar, 0, 1.0);

        assert_eq!(bar.width, (BAR_WIDTH + HIT_BUFFER) as u32);
        assert_eq!(bar.x + bar.width as i32, 1920, "flush against the edge");
    }

    #[test]
    fn mirrors_the_hover_target_for_the_left_edge() {
        let window = place(work_area(1.0), DockEdge::Left);

        let bar = bar_rect(window, DockEdge::Left, DockShape::Bar, 0, 1.0);

        assert_eq!(bar.x, 0);
        assert_eq!(bar.width, (BAR_WIDTH + HIT_BUFFER) as u32);
    }

    #[test]
    fn keeps_the_bar_sixty_by_one_hundred_and_sixty_eight_logical_pixels_at_every_scale() {
        // The bar stays 60 logical pixels wide at 100 / 125 / 150 / 200%.
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let area = work_area(scale);
            let bar = bar_rect(
                place(area, DockEdge::Right),
                DockEdge::Right,
                DockShape::Bar,
                0,
                scale,
            );

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

        let resting = bar_rect(window, DockEdge::Right, DockShape::Bar, 0, 2.0);
        let lowered = bar_rect(window, DockEdge::Right, DockShape::Bar, 100, 2.0);

        assert_eq!(lowered.y - resting.y, 200);
    }

    #[test]
    fn a_point_on_the_bar_is_inside_and_one_beside_it_is_not() {
        let bar = bar_rect(
            place(work_area(1.0), DockEdge::Right),
            DockEdge::Right,
            DockShape::Bar,
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
            DockShape::Bar,
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
            DockShape::Bar,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.edge, DockEdge::Right);
        assert_eq!(snapped.display_id, "left-screen");
    }

    #[test]
    fn the_bar_decides_the_edge_rather_than_the_window() {
        // Window centre left of the screen's middle, bar still right of it: the bar wins.
        let window = dragged_from_right(-(1920 / 2 - 457 / 2 + 20), 0);
        let window_centre = window.x + window.width as i32 / 2;
        assert!(
            window_centre < 960,
            "the premise: window centre left of middle"
        );

        let snapped = snap(
            &two_monitors(),
            window,
            DockEdge::Right,
            DockShape::Bar,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.edge, DockEdge::Right);
    }

    #[test]
    fn a_window_dragged_onto_the_second_monitor_stays_there() {
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(600, 0),
            DockEdge::Right,
            DockShape::Bar,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.display_id, "right-screen");
        assert_eq!(snapped.edge, DockEdge::Left);
    }

    #[test]
    fn a_window_let_go_in_the_gap_goes_to_the_nearer_monitor() {
        // The bar is 50 past the left screen and 30 short of the right one; the nearer one wins
        // rather than the first in the list.
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(80, 0),
            DockEdge::Right,
            DockShape::Bar,
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
            DockShape::Bar,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(
            snapped.vertical_offset,
            clamp_offset(area, DockShape::Bar, 10_000)
        );
    }

    #[test]
    fn a_window_cannot_be_dragged_off_the_top() {
        let area = work_area(1.0);

        let snapped = snap(
            &two_monitors(),
            dragged_from_right(0, -5000),
            DockEdge::Right,
            DockShape::Bar,
            0,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(
            snapped.vertical_offset,
            clamp_offset(area, DockShape::Bar, -10_000)
        );
    }

    #[test]
    fn a_drag_that_moves_nothing_changes_nothing() {
        // Round trip: a zero-distance drag stores what was already there.
        let snapped = snap(
            &two_monitors(),
            dragged_from_right(0, 0),
            DockEdge::Right,
            DockShape::Bar,
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
            DockShape::Bar,
            120,
            1.0,
        )
        .expect("a monitor");

        assert_eq!(snapped.vertical_offset, 170);
    }

    #[test]
    fn the_offset_a_drag_stores_is_in_logical_pixels() {
        // A physical offset would move the bar twice as far at 200% on the next start.
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

        let snapped =
            snap(&targets, window, DockEdge::Right, DockShape::Bar, 0, 2.0).expect("a monitor");

        assert_eq!(snapped.vertical_offset, 100);
    }

    #[test]
    fn a_drag_with_no_monitor_attached_decides_nothing() {
        assert_eq!(
            snap(
                &[],
                dragged_from_right(0, 0),
                DockEdge::Right,
                DockShape::Bar,
                0,
                1.0
            ),
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
        // The key is persisted, and metadata must not hold device paths.
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
