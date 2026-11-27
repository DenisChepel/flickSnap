//! Where the tray goes: choose the side of the selection, ask for the
//! strip's size in that orientation, and keep the whole box on the
//! monitor (tail wins when it cannot fit).
//!
//! The tray owns the selection's bottom and top edges and the drawing
//! strip its right and left (`drawbar::place`), so the two never stand
//! on one side. The tray hangs off the bottom-right corner — a row
//! beneath the selection, ending at its right edge — and when the room
//! beneath cannot hold it at its full distance it jumps to the top-right
//! corner rather than squeeze in closer. It never stands on its end
//! beside the selection: only a monitor too narrow for the row at all
//! gets a column, tucked inside the selection.
//!
//! Pure and in integer physical pixels, the same units the selection and
//! the monitor bounds are in. `show` turns the result into the
//! monitor-local point position egui's `Area` wants.

use flicksnap_shared::geometry::{RectExt, ScreenRect};

use super::theme::tokens;

/// Which way the strip runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// A row beneath the anchor, ending at its right edge.
    Below,
    /// A row above the anchor, ending at its right edge.
    Above,
    /// A row or column tucked inside the anchor's bottom-right corner.
    Inside(Axis),
}

impl Side {
    pub fn axis(self) -> Axis {
        match self {
            Side::Below | Side::Above => Axis::Row,
            Side::Inside(axis) => axis,
        }
    }
}

/// The LONGEST box the strip can become in one orientation: `len` along
/// it, `thick` across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Footprint {
    pub len: i32,
    pub thick: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fit {
    pub row: Footprint,
    pub col: Footprint,
}

impl Fit {
    pub fn scale_on(self, axis: Axis, bounds: ScreenRect) -> f32 {
        let (w, h) = match axis {
            Axis::Row => (self.row.len, self.row.thick),
            Axis::Column => (self.col.thick, self.col.len),
        };
        super::theme::fit_scale(
            egui::vec2(w as f32, h as f32),
            egui::vec2(bounds.width() as f32, bounds.height() as f32),
        )
    }

    pub fn fits_on(self, axis: Axis, bounds: ScreenRect) -> bool {
        match axis {
            Axis::Row => self.row.len <= bounds.width() && self.row.thick <= bounds.height(),
            Axis::Column => self.col.len <= bounds.height() && self.col.thick <= bounds.width(),
        }
    }
}

/// `min_distance` is clearance kept from the bounds edge in the fit test;
/// `max_distance` the gap between the anchor and the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Near {
    pub min_distance: i32,
    pub max_distance: i32,
}

impl Near {
    /// The two logical distances at one monitor's scale. Both are
    /// distances, so both round up, exactly as the retired kit's
    /// `Scale::space` did.
    pub fn at_dpi(dpi: f32) -> Self {
        Self {
            min_distance: (tokens::NEAR_MIN * dpi).ceil() as i32,
            max_distance: (tokens::NEAR_MAX * dpi).ceil() as i32,
        }
    }
}

/// Choose the side for `anchor` (already clipped to `bounds`), ask
/// `size_for` ONCE for that axis's `(width, height)` in px, put the box at
/// its origin and clamp both axes onto `bounds`.
///
/// `lock` pins the strip to one axis: the sides running the other way are
/// struck out of the cascade before the fit tests, so a set whose contents
/// only make sense along one axis (the scroll-picker's wrapped
/// instruction) can never be stood on its end.
pub fn place(
    anchor: ScreenRect,
    bounds: ScreenRect,
    fit: Fit,
    near: Near,
    lock: Option<Axis>,
    size_for: impl FnOnce(Axis) -> (i32, i32),
) -> (Side, ScreenRect) {
    let side = choose_side(anchor, bounds, fit, near, lock);
    let (w, h) = size_for(side.axis());
    let (x, y) = origin(side, anchor, (w, h), near);
    // Keep the WHOLE box on the bounds, both axes (a shadow may clip).
    // A side outside the anchor is only taken with room for the box at its
    // full distance, so this moves one only along the selection's edge;
    // only when nothing fits can the box still be longer than the bounds,
    // and then the tail stays on the bounds and the head overflows.
    let rect = ScreenRect::from_xy_size(
        clamp_span(x, w, bounds.left(), bounds.right()),
        clamp_span(y, h, bounds.top(), bounds.bottom()),
        w,
        h,
    );
    (side, rect)
}

const CASCADE: [Side; 4] = [Side::Below, Side::Above, Side::Inside(Axis::Row), Side::Inside(Axis::Column)];

/// A candidate fits when the free space on its side holds the box's
/// THICKNESS at the full distance from the anchor, still clear of the
/// bounds edge, and the full footprint fits the monitor. Inside, prefer a
/// row whenever it fits. When no candidate fully fits, the free space
/// across the strip alone decides.
fn choose_side(anchor: ScreenRect, bounds: ScreenRect, fit: Fit, near: Near, lock: Option<Axis>) -> Side {
    // What a side outside the anchor has to hold besides the box itself.
    let around = near.max_distance + near.min_distance;
    let below = bounds.bottom() - anchor.bottom() - around;
    let above = anchor.top() - bounds.top() - around;
    let across = |side: Side| match side {
        Side::Below => below >= fit.row.thick,
        Side::Above => above >= fit.row.thick,
        Side::Inside(_) => true,
    };
    let allowed = |s: Side| lock.is_none_or(|axis| s.axis() == axis);
    CASCADE
        .into_iter()
        .filter(|&s| allowed(s))
        .find(|&s| across(s) && fit.fits_on(s.axis(), bounds))
        .or_else(|| {
            CASCADE
                .into_iter()
                .filter(|&s| allowed(s))
                .find(|&s| across(s))
        })
        .unwrap_or(Side::Inside(lock.unwrap_or(Axis::Row)))
}

/// Top-left of a `size` box on `side`, before the bounds clamp: always the
/// full distance from the anchor, never pulled in closer.
fn origin(side: Side, anchor: ScreenRect, size: (i32, i32), near: Near) -> (i32, i32) {
    let (w, h) = size;
    let gap = near.max_distance;
    match side {
        Side::Below => (anchor.right() - w, anchor.bottom() + gap),
        Side::Above => (anchor.right() - w, anchor.top() - gap - h),
        Side::Inside(_) => (anchor.right() - w - 2 * gap, anchor.bottom() - h - 2 * gap),
    }
}

/// `pos` moved so `[pos, pos + len)` lies in `[lo, hi)`; the high edge wins
/// when `len` exceeds the span.
fn clamp_span(pos: i32, len: i32, lo: i32, hi: i32) -> i32 {
    pos.max(lo).min(hi - len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const FIT: Fit = Fit {
        row: Footprint {
            len: 600,
            thick: 56,
        },
        col: Footprint {
            len: 500,
            thick: 92,
        },
    };

    fn near() -> Near {
        Near::at_dpi(1.0)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> ScreenRect {
        ScreenRect::from_xy_size(x, y, w, h)
    }

    /// The `FIT` box for `axis`, as `size_for` returns it.
    fn full(axis: Axis) -> (i32, i32) {
        match axis {
            Axis::Row => (FIT.row.len, FIT.row.thick),
            Axis::Column => (FIT.col.thick, FIT.col.len),
        }
    }

    fn placed(anchor: ScreenRect, bounds: ScreenRect) -> (ScreenRect, Side) {
        let (side, rect) = place(anchor, bounds, FIT, near(), None, full);
        (rect, side)
    }

    const HD: ScreenRect = ScreenRect {
        origin: euclid::Point2D::new(0, 0),
        size: euclid::Size2D::new(1920, 1080),
    };

    #[test]
    fn below_when_it_fits() {
        assert_eq!(placed(rect(500, 300, 100, 100), HD), (rect(0, 412, 600, 56), Side::Below));
    }

    /// No room beneath: the row goes to the top-right corner, never onto
    /// either flank — those are the drawing strip's.
    #[test]
    fn above_when_below_is_short() {
        assert_eq!(
            placed(rect(500, 1000, 100, 60), HD),
            (rect(0, 1000 - 12 - 56, 600, 56), Side::Above)
        );
        assert_eq!(placed(rect(1800, 1000, 100, 60), HD).1, Side::Above);
    }

    #[test]
    fn inside_when_neither_edge_has_room() {
        assert_eq!(
            placed(HD, HD),
            (rect(1920 - 24 - 600, 1080 - 24 - 56, 600, 56), Side::Inside(Axis::Row))
        );
    }

    /// Only a monitor too narrow for the row at all gets a column, and
    /// then inside the selection, not beside it.
    #[test]
    fn a_monitor_too_narrow_for_the_row_gets_a_column_inside() {
        let narrow = rect(0, 0, 500, 1080);
        assert_eq!(placed(rect(100, 300, 300, 300), narrow).1, Side::Inside(Axis::Column));
    }

    /// The room beneath has to hold the whole gap as well as the row: one
    /// pixel short and the tray jumps to the top rather than sit closer to
    /// the selection. Wherever it lands outside, it is the full gap away.
    #[test]
    fn the_gap_never_shrinks_the_tray_jumps_instead() {
        let fits = 1080 - 2 - 12 - 56;
        let (r, side) = placed(rect(500, 300, 100, fits - 300), HD);
        assert_eq!(side, Side::Below);
        assert_eq!(r.top(), fits + 12);
        assert_eq!(r.bottom(), 1080 - 2);

        let (r, side) = placed(rect(500, 300, 100, fits - 300 + 1), HD);
        assert_eq!(side, Side::Above);
        assert_eq!(r.bottom(), 300 - 12);
    }

    #[test]
    fn min_distance_is_subtracted_before_the_compare() {
        let anchor = rect(500, 100, 100, 1080 - 100 - 12 - 56);
        let loose = Near {
            min_distance: 0,
            ..near()
        };
        assert_eq!(place(anchor, HD, FIT, loose, None, full).0, Side::Below);
        assert_eq!(place(anchor, HD, FIT, near(), None, full).0, Side::Above);
    }

    /// A row that cannot fit the monitor's width but has room beneath is
    /// still put there; the clamp keeps its tail on the monitor and lets
    /// its head overflow.
    #[test]
    fn clamp_keeps_the_high_edge_when_the_box_is_longer_than_the_bounds() {
        let small = rect(0, 0, 500, 400);
        let (r, side) = placed(rect(100, 50, 100, 100), small);
        assert_eq!((side, r.left(), r.right()), (Side::Below, -100, 500), "the head overflows the left");
    }

    /// A row on a selection near the monitor's left edge slides along the
    /// edge onto the monitor; it never moves across it.
    #[test]
    fn a_row_slides_along_the_selection_onto_the_monitor() {
        let (r, side) = placed(rect(0, 300, 100, 100), HD);
        assert_eq!((side, r.left(), r.top()), (Side::Below, 0, 412));
        let (r, side) = placed(rect(0, 1000, 100, 60), HD);
        assert_eq!((side, r.left(), r.bottom()), (Side::Above, 0, 1000 - 12));
    }

    /// A strip locked to a row is never stood on its end, even where only
    /// a column would fit.
    #[test]
    fn a_row_lock_never_yields_a_column() {
        let narrow = rect(0, 0, 500, 1080);
        let (side, _) = place(rect(100, 300, 300, 300), narrow, FIT, near(), Some(Axis::Row), full);
        assert_eq!(side.axis(), Axis::Row);
    }

    #[test]
    fn negative_origin_bounds_are_a_pure_translation() {
        let shifted = rect(-1920, 0, 1920, 1080);
        for anchor in [rect(500, 300, 100, 100), rect(1800, 1000, 100, 60), HD] {
            let (here, side) = placed(anchor, HD);
            let (there, side2) = placed(anchor.translate(euclid::vec2(-1920, 0)), shifted);
            assert_eq!(side, side2);
            assert_eq!(there, here.translate(euclid::vec2(-1920, 0)), "{side:?}");
        }
    }

    /// A row ends at the anchor's right edge whatever its width, so the
    /// tray and the drawing strip meet at the selection's corner.
    #[test]
    fn a_row_ends_at_the_anchors_right_edge() {
        for (aw, w) in [(100, 600), (101, 600), (700, 601), (1200, 400)] {
            for y in [300, 1000] {
                let anchor = rect(700, y, aw, 60);
                let fit = Fit {
                    row: Footprint {
                        len: w,
                        thick: 56,
                    },
                    ..FIT
                };
                let (side, r) = place(anchor, HD, fit, near(), None, |_| (w, 56));
                assert_eq!(side.axis(), Axis::Row);
                assert_eq!(r.right(), anchor.right(), "anchor {aw} box {w} at {y}");
            }
        }
    }

    #[test]
    fn size_for_is_called_once_with_the_chosen_axis() {
        let calls = Cell::new(0);
        let axis = Cell::new(None);
        let (side, _) = place(rect(100, 300, 300, 300), rect(0, 0, 500, 1080), FIT, near(), None, |a| {
            calls.set(calls.get() + 1);
            axis.set(Some(a));
            full(a)
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(axis.get(), Some(side.axis()));
        assert_eq!(side.axis(), Axis::Column);
    }

    /// Each strip is placed with its OWN width and ends where every other
    /// one does, so a set swap leaves the strip's right end where it was.
    #[test]
    fn each_strip_keeps_its_right_end_whatever_its_width() {
        let anchor = rect(900, 300, 400, 100);
        let ends = [400, 520].map(|w| {
            place(anchor, HD, FIT, near(), None, |_| (w, 56))
                .1
                .right()
        });
        assert_eq!(ends, [anchor.right(); 2]);
    }

    #[test]
    fn near_scales_by_ceil_at_each_dpi() {
        for (dpi, min, max) in [(1.0, 2, 12), (1.25, 3, 15), (1.5, 3, 18), (2.0, 4, 24)] {
            let near = Near::at_dpi(dpi);
            assert_eq!((near.min_distance, near.max_distance), (min, max), "at {dpi}");
        }
    }
}
