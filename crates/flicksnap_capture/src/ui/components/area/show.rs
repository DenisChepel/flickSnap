//! The selection's dimensions, anchored to its visible top-left corner.

use egui::emath::GuiRounding as _;
use egui::{pos2, vec2, Color32, FontId, Painter, Rect, Stroke, StrokeKind, Vec2};

use crate::ui::components::{InputCtx, Local};
use crate::ui::fonts;
use crate::ui::shared::UiMonitor;
use flicksnap_shared::geometry::RectExt;

/// One host's copy of the indicator, already in that monitor's points.
#[derive(Clone, Copy, PartialEq)]
pub struct AreaInputs {
    /// The UNCLIPPED selection size: the label prints this, so a rect
    /// straddling two monitors keeps reporting its true dimensions.
    pub size: (i32, i32),
    /// The displayed selection intersected with this monitor, in local points.
    pub clipped: Rect,
    pub accent: Color32,
}

const FONT_PT: f32 = 13.0;
const PADDING: f32 = 8.0;
const BORDER_W: f32 = 2.0;
const GAP: f32 = 4.0;

/// Only the monitor containing the topmost, then leftmost visible part of
/// the selection draws the pill. This also handles corners in monitor gaps.
pub fn inputs(index: usize, monitor: &UiMonitor, c: &InputCtx<'_>) -> Option<AreaInputs> {
    let i = c.input;
    if !i.overlays_visible || !(i.dragging || i.captured) || i.ocr.active() {
        return None;
    }
    let sel = i.selection?;
    let displayed = sel.to_f32();
    let (host, clipped) = c
        .monitors
        .iter()
        .enumerate()
        .filter_map(|(index, m)| {
            displayed
                .intersection(&m.bounds.to_f32())
                .map(|rect| (index, rect))
        })
        .min_by(|(_, a), (_, b)| {
            a.top()
                .total_cmp(&b.top())
                .then(a.left().total_cmp(&b.left()))
        })?;
    if host != index {
        return None;
    }
    Some(AreaInputs {
        size: (sel.width(), sel.height()),
        clipped: Local::of(monitor).rect(clipped),
        accent: c.accent,
    })
}

/// Prefer the space above the top edge. When it is too short, inset the
/// pill inside the corner as far as the selection allows.
fn pill_rect(clipped: Rect, text_size: Vec2, screen: Rect) -> Rect {
    let size = text_size + vec2(2.0 * PADDING, PADDING);
    let above = clipped.top() - size.y - GAP;
    let position = if above >= screen.top() {
        pos2(clipped.left(), above)
    } else {
        let inset = (clipped.size() - size).clamp(Vec2::ZERO, Vec2::splat(GAP));
        clipped.min + inset
    };
    let max = (screen.max - size).max(screen.min);
    Rect::from_min_size(position.clamp(screen.min, max), size)
}

pub fn show(p: &Painter, a: &AreaInputs, screen: Rect) {
    let ppp = p.pixels_per_point();
    // Fractional display scaling can turn a whole point size into a
    // fractional pixel size. Snap the size before rasterizing the glyphs.
    let font_size = FONT_PT.round_to_pixels(ppp);
    let galley = p.layout_no_wrap(
        format!("{} \u{00D7} {}", a.size.0, a.size.1),
        FontId::new(font_size, fonts::SEMIBOLD.clone()),
        Color32::BLACK,
    );
    let rect = pill_rect(a.clipped, galley.size(), screen);
    p.rect(
        rect,
        rect.height() / 2.0,
        Color32::WHITE,
        Stroke::new(BORDER_W.round_to_pixels(ppp), a.accent),
        StrokeKind::Inside,
    );
    p.galley(rect.min + vec2(PADDING, PADDING / 2.0), galley, Color32::BLACK);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::InteractionState;
    use crate::ui::shared::monitor_index_at;
    use egui::Pos2;
    use flicksnap_shared::geometry::{ScreenPointF, ScreenRect};

    fn screen() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0))
    }

    fn text() -> Vec2 {
        vec2(64.0, 14.0)
    }

    /// The mixed pair every straddling rule is checked against: the
    /// primary at 100 % and its right-hand neighbour at 150 %.
    fn monitors() -> [UiMonitor; 2] {
        [
            UiMonitor {
                bounds: ScreenRect::from_xy_size(0, 0, 1920, 1080),
                dpi_scale: 1.0,
                is_primary: true,
            },
            UiMonitor {
                bounds: ScreenRect::from_xy_size(1920, 0, 1920, 1080),
                dpi_scale: 1.5,
                is_primary: false,
            },
        ]
    }

    fn ctx<'a>(input: &'a InteractionState, monitors: &'a [UiMonitor]) -> InputCtx<'a> {
        InputCtx {
            input,
            monitors,
            cursor_index: monitor_index_at(monitors, input.virtual_cursor),
            accent: Color32::RED,
            hovered_monitor_name: None,
        }
    }

    #[test]
    fn pill_aligns_with_the_left_edge_above_even_a_small_selection() {
        let s = screen();
        let tiny = Rect::from_min_size(pos2(400.0, 200.0), vec2(6.0, 8.0));
        let pill = pill_rect(tiny, text(), s);
        assert_eq!(pill.left(), tiny.left());
        assert_eq!(pill.bottom(), tiny.top() - GAP);

        let right = tiny.translate(vec2(1514.0, 0.0));
        let pill = pill_rect(right, text(), s);
        assert_eq!(pill.right(), s.right());
        assert_eq!(pill.bottom(), right.top() - GAP);
    }

    #[test]
    fn pill_falls_inside_at_the_top_and_moves_out_after_resizing() {
        let s = screen();
        let full = pill_rect(s, text(), s);
        assert_eq!(full.min, pos2(GAP, GAP));
        assert!(s.contains_rect(full));

        let inset = Rect::from_min_max(pos2(150.0, 10.0), s.max);
        let inside = pill_rect(inset, text(), s);
        assert_eq!(inside.min, inset.min + Vec2::splat(GAP));
        assert!(inset.contains_rect(inside));

        let resized = Rect::from_min_max(pos2(200.0, 100.0), s.max);
        let above = pill_rect(resized, text(), s);
        assert_eq!(above.left(), resized.left());
        assert_eq!(above.bottom(), resized.top() - GAP);
    }

    #[test]
    fn area_indicator_follows_the_selection_across_monitors_after_confirmation() {
        let m = monitors();
        let mut input = InteractionState::new();
        input.virtual_cursor = ScreenPointF::new(2050.0, 400.0);
        input.selection = Some(ScreenRect::from_xy_size(1800, 120, 400, 600));
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none(), "no drag or confirmed selection");

        input.dragging = true;
        let shown = inputs(0, &m[0], &ctx(&input, &m)).expect("corner on primary");
        assert_eq!(shown.size, (400, 600));
        assert_eq!(shown.clipped, Rect::from_min_max(pos2(1800.0, 120.0), pos2(1920.0, 720.0)));
        assert!(inputs(1, &m[1], &ctx(&input, &m)).is_none(), "cursor is on the other monitor");

        input.dragging = false;
        input.captured = true;
        let confirmed = inputs(0, &m[0], &ctx(&input, &m)).expect("still on primary");
        assert_eq!(confirmed.clipped, shown.clipped);

        input.selection = Some(ScreenRect::from_xy_size(1980, 150, 220, 570));
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none(), "clear the previous host");
        let resized = inputs(1, &m[1], &ctx(&input, &m)).expect("new corner on the neighbour");
        assert_eq!(resized.clipped.min, pos2(40.0, 100.0));
        assert_eq!(resized.size, (220, 570));

        input.overlays_visible = false;
        assert!(inputs(1, &m[1], &ctx(&input, &m)).is_none(), "Q hides it");
        input.overlays_visible = true;

        input.selection = None;
        assert!(inputs(1, &m[1], &ctx(&input, &m)).is_none(), "nothing to measure");
    }
}
