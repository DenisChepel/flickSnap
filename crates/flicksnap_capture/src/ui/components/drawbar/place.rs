//! Where the drawing strip goes: beside the selection's right edge, level
//! with its bottom — the corner the action tray hangs off too — and never
//! over the tray.
//!
//! Pure and in integer physical pixels, like `panel::place`, whose
//! [`Near`] distances it shares: the strip stands as far from the
//! selection as the tray does. The tray is placed first and handed in as
//! `avoid`, so the two can never be laid over each other where there is
//! any way to keep them apart.

use flicksnap_shared::geometry::{RectExt, ScreenRect};

use crate::ui::components::panel::place::Near;

/// Which side of the selection the strip went to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Outside the selection's right edge: the strip's home.
    Right,
    /// Outside its left edge, when the right has no room.
    Left,
    /// Inside its bottom-right corner, when neither outside edge has room
    /// — a capture of the whole monitor.
    Inside,
}

/// The strip's box for a `size` = `(width, height)` strip beside `anchor`
/// (the selection, already clipped to `bounds`), kept on `bounds` and off
/// `avoid`.
///
/// The sides are tried in [`Side`] order. A side is taken when the strip
/// fits across on it and some place on it stays clear of `avoid`: level
/// with the anchor's bottom first, then just above `avoid`, then just
/// below it, each pulled back onto `bounds`. Beside the selection those
/// are tried twice — the full gap from the selection, then, when the tray
/// reaches out under that flank (a wide tray at a high scale, held onto
/// the monitor past the selection's edge), the full gap past the tray's
/// own end on that side. When no side manages any of that, the first side
/// the strip fits across on wins level with the anchor's bottom, and the
/// overlap is the price of a monitor that leaves no room.
pub fn place(anchor: ScreenRect, bounds: ScreenRect, size: (i32, i32), avoid: ScreenRect, near: Near) -> (Side, ScreenRect) {
    let (w, h) = size;
    let gap = near.max_distance;
    let x_of = |side: Side| match side {
        Side::Right => anchor.right() + gap,
        Side::Left => anchor.left() - gap - w,
        Side::Inside => anchor.right() - gap - w,
    };
    let top_of = |side: Side| match side {
        Side::Right | Side::Left => anchor.bottom() - h,
        Side::Inside => anchor.bottom() - gap - h,
    };
    // Room across at `x`: a box outside the anchor must stay
    // `min_distance` off the monitor's edge. Inside always has room; it is
    // the last resort.
    let fits_at = |side: Side, x: i32| match side {
        Side::Right => x + w <= bounds.right() - near.min_distance,
        Side::Left => x >= bounds.left() + near.min_distance,
        Side::Inside => true,
    };
    let fits_across = |side: Side| fits_at(side, x_of(side));
    // Where along the flank the strip may stand: the full gap from the
    // selection, then the full gap past the tray's end on that side, when
    // that is further out and still on the monitor.
    let xs = |side: Side| {
        let home = x_of(side);
        let past_tray = match side {
            Side::Right => Some(avoid.right() + gap).filter(|&x| x > home),
            Side::Left => Some(avoid.left() - gap - w).filter(|&x| x < home),
            Side::Inside => None,
        };
        std::iter::once(home).chain(past_tray.filter(|&x| fits_at(side, x)))
    };
    let on_bounds = |x: i32, y: i32| {
        ScreenRect::from_xy_size(
            clamp_span(x, w, bounds.left(), bounds.right()),
            clamp_span(y, h, bounds.top(), bounds.bottom()),
            w,
            h,
        )
    };
    let clear = |r: &ScreenRect| !r.intersects(&avoid.inflate(gap, gap));

    const SIDES: [Side; 3] = [Side::Right, Side::Left, Side::Inside];
    for side in SIDES.into_iter().filter(|&s| fits_across(s)) {
        let heights = [top_of(side), avoid.top() - gap - h, avoid.bottom() + gap];
        let found = xs(side)
            .flat_map(|x| heights.map(|y| on_bounds(x, y)))
            .find(clear);
        if let Some(rect) = found {
            return (side, rect);
        }
    }
    let side = SIDES
        .into_iter()
        .find(|&s| fits_across(s))
        .unwrap_or(Side::Inside);
    (side, on_bounds(x_of(side), top_of(side)))
}

/// `pos` moved so `[pos, pos + len)` lies in `[lo, hi)`; the low edge wins
/// when `len` exceeds the span, so a strip taller than the monitor keeps
/// its first tools on screen.
fn clamp_span(pos: i32, len: i32, lo: i32, hi: i32) -> i32 {
    pos.min(hi - len).max(lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> ScreenRect {
        ScreenRect::from_xy_size(x, y, w, h)
    }

    const HD: ScreenRect = ScreenRect {
        origin: euclid::Point2D::new(0, 0),
        size: euclid::Size2D::new(1920, 1080),
    };

    /// 48 x 265: the strip at 100 %.
    const SIZE: (i32, i32) = (48, 265);

    fn near() -> Near {
        Near::at_dpi(1.0)
    }

    /// A tray nowhere near the strip.
    fn far_tray() -> ScreenRect {
        rect(0, 1000, 10, 10)
    }

    #[test]
    fn right_of_the_selection_level_with_its_bottom() {
        let (side, r) = place(rect(500, 300, 600, 400), HD, SIZE, far_tray(), near());
        assert_eq!((side, r), (Side::Right, rect(1112, 700 - 265, 48, 265)));
    }

    #[test]
    fn left_when_the_right_has_no_room() {
        let (side, r) = place(rect(1500, 300, 400, 400), HD, SIZE, far_tray(), near());
        assert_eq!((side, r), (Side::Left, rect(1440, 700 - 265, 48, 265)));
    }

    #[test]
    fn inside_the_bottom_right_corner_of_a_whole_monitor_capture() {
        let (side, r) = place(HD, HD, SIZE, far_tray(), near());
        assert_eq!((side, r), (Side::Inside, rect(1920 - 12 - 48, 1080 - 12 - 265, 48, 265)));
    }

    /// A selection shorter than the strip near the monitor's top: the strip
    /// would rise off the monitor, and is pulled back down onto it.
    #[test]
    fn a_strip_near_the_top_is_pulled_back_onto_the_monitor() {
        let (side, r) = place(rect(500, 20, 600, 100), HD, SIZE, far_tray(), near());
        assert_eq!(side, Side::Right);
        assert_eq!(r.top(), 0);
    }

    /// A tray lying across the strip's home beside the selection (as an
    /// inside tray does on a whole-monitor capture): the strip climbs above
    /// it rather than sit on it, still the full gap away.
    #[test]
    fn the_strip_climbs_above_a_tray_in_its_way() {
        let anchor = rect(800, 300, 300, 400);
        let tray = rect(700, 600, 600, 48);
        let (side, r) = place(anchor, HD, SIZE, tray, near());
        assert_eq!(side, Side::Right);
        assert!(!r.intersects(&tray), "{r:?} over {tray:?}");
        assert_eq!(r.bottom(), tray.top() - 12);
    }

    /// The reviewer's case in physical px — 1080p at 300 %, a narrow
    /// selection at the monitor's left edge: the tray is wider than
    /// everything left of the selection's right edge, so it is held onto
    /// the monitor reaching out under the right flank, with no room for
    /// the column above or below it. The strip steps out past the tray's
    /// end instead of lying on it.
    #[test]
    fn the_strip_steps_past_a_tray_reaching_under_its_flank() {
        let anchor = rect(0, 96, 300, 336);
        let tray = rect(0, 468, 1209, 114);
        let size = (114, 633);
        let (side, r) = place(anchor, HD, size, tray, Near::at_dpi(3.0));
        assert_eq!(side, Side::Right);
        assert!(!r.intersects(&tray), "{r:?} over {tray:?}");
        assert_eq!(r.left(), tray.right() + Near::at_dpi(3.0).max_distance);
    }

    /// Whatever holds the whole right flank out to the monitor's edge —
    /// the tray never stands there, but the rule does not lean on that —
    /// leaves no room above, below or past it, so the strip goes to the
    /// left instead.
    #[test]
    fn a_tray_holding_the_right_edge_sends_the_strip_left() {
        let anchor = rect(800, 100, 300, 950);
        let tray = rect(1112, 100, 1920 - 1112, 950);
        let (side, r) = place(anchor, HD, SIZE, tray, near());
        assert_eq!(side, Side::Left);
        assert!(!r.intersects(&tray));
    }

    #[test]
    fn with_nowhere_clear_the_first_side_with_room_wins() {
        let anchor = rect(800, 100, 300, 950);
        let everywhere = HD;
        let (side, r) = place(anchor, HD, SIZE, everywhere, near());
        assert_eq!((side, r), (Side::Right, rect(1112, 1050 - 265, 48, 265)));
    }

    #[test]
    fn negative_origin_bounds_are_a_pure_translation() {
        let shift = euclid::vec2(-1920, 0);
        let shifted = HD.translate(shift);
        for anchor in [rect(500, 300, 600, 400), rect(1500, 300, 400, 400), HD] {
            let (side, here) = place(anchor, HD, SIZE, far_tray(), near());
            let (side2, there) = place(anchor.translate(shift), shifted, SIZE, far_tray().translate(shift), near());
            assert_eq!(side, side2);
            assert_eq!(there, here.translate(shift), "{side:?}");
        }
    }
}
