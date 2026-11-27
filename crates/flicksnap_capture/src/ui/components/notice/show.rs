//! The transient OCR notice pill ("no text found"): feedback for the OCR
//! press the user just made, centred just above the selection on the one
//! monitor that holds the selection's centre.

use egui::{pos2, vec2, Color32, Context, Painter, Rect, Vec2};

use crate::interaction::{notice_alpha, OcrNotice};
use crate::ui::components::pill;
use crate::ui::components::{InputCtx, Local};
use crate::ui::egui_host::REPAINT_FLOOR;
use crate::ui::shared::{pick_monitor_containing_center, UiMonitor};

/// Gap between the selection's top edge and the pill above it.
const NOTICE_GAP_Y: f32 = 8.0;

/// The notice, on the one host that shows it. Built only while it is still
/// visible, so the memo catches the tick it disappears.
#[derive(Clone, Copy, PartialEq)]
pub struct NoticeInputs {
    pub notice: OcrNotice,
    /// The selection in local points: the pill sits above its top edge.
    pub selection: Rect,
}

/// One copy of the notice, on the host whose monitor holds the selection's
/// centre — the same rule that places the tray, so the pill and the button
/// that raised it share a monitor.
///
/// Deliberately not gated on the overlay toggle: the notice is feedback for
/// a press the user just made, and it has to reach them whatever else is
/// hidden. The one thing that does suppress it is the scroll-point picker,
/// which owns the whole overlay — with the OS pointer hidden and a reticle
/// in its place, a pill fading in beside the aiming instruction would read
/// as part of it.
pub fn inputs(index: usize, monitor: &UiMonitor, c: &InputCtx<'_>) -> Option<NoticeInputs> {
    let _ = index;
    if c.input.scroll_pick_mode {
        return None;
    }
    let n = c.input.ocr_notice.filter(|n| n.visible())?;
    let sel = c.input.selection?;
    let target = pick_monitor_containing_center(c.monitors, sel)?;
    if target.bounds != monitor.bounds {
        return None;
    }
    Some(NoticeInputs {
        notice: n,
        selection: Local::of(monitor).rect(sel.to_f32()),
    })
}

/// Paint the notice, fading off its own anchor.
pub fn show(ctx: &Context, p: &Painter, n: &NoticeInputs, screen: Rect) {
    let a = notice_alpha(n.notice.anchor.elapsed().as_secs_f32());
    let galley = p.layout_no_wrap(n.notice.kind.message().to_owned(), pill::font(), Color32::PLACEHOLDER);
    let text = galley.size();
    let rect = notice_rect(n.selection, pill_size(text), screen);
    p.add(pill::shadow(rect, a));
    p.add(pill::body(rect, pill::RADIUS, a, true));
    p.galley(
        pos2(rect.left() + pill::PAD_H, rect.center().y - text.y / 2.0),
        galley,
        pill::TEXT.gamma_multiply(a),
    );
    ctx.request_repaint_after(REPAINT_FLOOR);
}

/// The pill that holds `text`.
fn pill_size(text: Vec2) -> Vec2 {
    vec2(2.0 * pill::PAD_H + text.x, 2.0 * pill::PAD_V + text.y)
}

/// Centred over the selection, just above its top edge, clamped fully
/// inside the screen. A selection at the very top pushes the pill down over
/// itself rather than off screen.
fn notice_rect(selection: Rect, size: Vec2, screen: Rect) -> Rect {
    let x = selection.center().x - size.x / 2.0;
    let y = selection.top() - NOTICE_GAP_Y - size.y;
    let x = x.clamp(screen.left(), (screen.right() - size.x).max(screen.left()));
    let y = y.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));
    Rect::from_min_size(pos2(x, y), size)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use egui::Pos2;

    use super::*;
    use crate::interaction::{InteractionState, OcrNoticeKind};
    use crate::ui::shared::monitor_index_at;
    use flicksnap_shared::geometry::{RectExt, ScreenPointF, ScreenRect};

    /// The primary at 100 % and its right-hand neighbour at 150 %.
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

    /// The notice belongs to the selection's monitor, and disappears when
    /// it expires rather than lingering as a transparent pill.
    #[test]
    fn notice_goes_to_the_selection_centre_host_only() {
        let m = monitors();
        let mut input = InteractionState::new();
        input.virtual_cursor = ScreenPointF::new(300.0, 300.0);
        input.captured = true;
        input.selection = Some(ScreenRect::from_xy_size(2000, 200, 400, 400));
        input.ocr_notice = Some(OcrNotice {
            anchor: Instant::now(),
            kind: OcrNoticeKind::NoText,
        });

        let c = ctx(&input, &m);
        assert!(inputs(0, &m[0], &c).is_none(), "the selection is on the neighbour");
        let there = inputs(1, &m[1], &c).expect("the selection's own host");
        assert_eq!(there.notice.kind, OcrNoticeKind::NoText);
        assert_eq!(there.selection.min, pos2((2000.0 - 1920.0) / 1.5, 200.0 / 1.5));

        // Q does not hide it: it is feedback for a press just made.
        input.overlays_visible = false;
        assert!(inputs(1, &m[1], &ctx(&input, &m)).is_some());
        input.overlays_visible = true;

        // The scroll-point picker does: it owns the overlay alone.
        input.scroll_pick_mode = true;
        assert!(inputs(1, &m[1], &ctx(&input, &m)).is_none());
        input.scroll_pick_mode = false;

        // Expired notices are simply not built.
        let expired = Instant::now().checked_sub(std::time::Duration::from_secs(30));
        if let Some(anchor) = expired {
            input.ocr_notice = Some(OcrNotice {
                anchor,
                kind: OcrNoticeKind::NoText,
            });
            assert!(inputs(1, &m[1], &ctx(&input, &m)).is_none());
        }
    }

    /// A selection at the very top of the screen pushes the notice down
    /// over itself rather than off the edge.
    #[test]
    fn notice_rect_at_the_top_edge_stays_inside() {
        let s = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0));
        let c = vec2(120.0, 28.0);

        let middle = notice_rect(Rect::from_min_size(pos2(600.0, 400.0), vec2(500.0, 300.0)), c, s);
        assert_eq!(middle.bottom(), 400.0 - NOTICE_GAP_Y);
        assert_eq!(middle.center().x, 850.0);

        let top = notice_rect(Rect::from_min_size(pos2(600.0, 2.0), vec2(500.0, 300.0)), c, s);
        assert_eq!(top.top(), s.top());
        assert!(s.contains_rect(top));
    }
}
