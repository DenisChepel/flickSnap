//! The Tips & Hotkeys panel, measured and painted by egui.
//!
//! The rule that decides which host draws it (the cursor's monitor, while
//! T has it up and nothing is captured), the pure layout the
//! old `tips/layout.rs` held, and the painting. The layout is computed
//! from galleys measured in the same pass rather than by an `egui::Area`,
//! because the bottom-right/bottom-left corner rule needs the panel's own
//! size and an `Area` only learns its rect one pass late.

use std::sync::Arc;

use egui::{pos2, vec2, Color32, FontFamily, FontId, Galley, Painter, Pos2, Rect};

use crate::ui::components::tips::model;
use crate::ui::components::{InputCtx, Local};
use crate::ui::fonts;
use crate::ui::shared::UiMonitor;

/// One host's copy of the panel: every row already rendered to a string,
/// so the pass only has to measure and paint.
#[derive(Clone, PartialEq)]
pub struct TipsInputs {
    /// The rows' descriptions. Their hotkeys are static and come from
    /// [`model::TIPS`].
    pub rows: [String; model::TIPS.len()],
    /// The cursor in local points: the panel jumps to the other corner
    /// when it would otherwise sit under it.
    pub cursor: Pos2,
    pub accent: Color32,
}

/// Title font size in points. `DxScreenCapture.cpp:435`.
pub const TITLE_PT: f32 = 14.0;
/// Body font size in points. `DxScreenCapture.cpp:436`.
pub const BODY_PT: f32 = 12.0;
/// Distance from the screen edge. `DEBUGBOX_MARGIN` in `pch.h:53`.
pub const SCREEN_MARGIN: f32 = 50.0;
/// Half the inner padding. `paddingHalf` in `DxScreenCapture.cpp:768`.
pub const PADDING_HALF: f32 = 10.0;
/// Full inner padding.
pub const PADDING: f32 = PADDING_HALF * 2.0;
/// Width floor. `DxScreenCapture.cpp:771`.
pub const MIN_PANEL_WIDTH: f32 = 400.0;
/// Body and title bar opacity; the shadow strips keep their own alpha.
pub const OPACITY: f32 = 0.70;
/// The hard-edged drop shadow, black at the old 0.30.
pub const SHADOW: Color32 = Color32::from_black_alpha(77);
/// Distance between the hotkey and description columns.
pub const DESC_INDENT: f32 = ROW_H * 1.75;
/// One body row's height — the old `body_row_height` of `font * 1.4`.
pub const ROW_H: f32 = BODY_PT * 1.4;
/// Cap height of the title, which is what the title bar is sized around.
/// The old code fed swash's `measure_line("Hg").height` here; Inter's cap
/// height is 0.73 x the font size.
pub const TITLE_H: f32 = TITLE_PT * 0.73;

/// Whether this monitor draws the panel, and what it draws.
///
/// The rule is the old `shared::tips_visibility`: the host holding the
/// cursor shows it, while T has it up, the overlays are on and nothing is
/// captured, the mouse is up and the debug panels are down.
pub fn inputs(index: usize, monitor: &UiMonitor, c: &InputCtx<'_>) -> Option<TipsInputs> {
    let i = c.input;
    let shown = i.overlays_visible && !i.captured && !i.mouse_down && i.tips_visible && !i.debug_visible;
    if !shown || c.cursor_index != Some(index) {
        return None;
    }
    let row = |r: &model::TipRow| model::render_description(r.description_template, c.hovered_monitor_name);
    let local = Local::of(monitor);
    Some(TipsInputs {
        rows: std::array::from_fn(|k| row(&model::TIPS[k])),
        cursor: local.pos(i.virtual_cursor.x, i.virtual_cursor.y),
        accent: c.accent,
    })
}

/// Where the panel and everything inside it goes, all in this host's
/// points. Rects and positions are absolute; the hotkey column offset is
/// panel-local, as it was in the old layout.
pub struct TipsLayout {
    pub panel: Rect,
    pub title_bar: Rect,
    pub col_hotkey_x: f32,
    pub col_desc_x: f32,
    pub rows_y: f32,
}

/// The old `tips::layout::compute_layout` with the DPI scaling gone: egui
/// works in points and rounds to pixels itself.
///
/// `longest_body` is the widest measured body row, measured from the
/// description column, and `title_w` the measured title, both in points.
pub fn compute_layout(screen: Rect, cursor: Pos2, longest_body: f32, title_w: f32) -> TipsLayout {
    let panel_w = ((longest_body + DESC_INDENT).max(title_w) + 2.0 * PADDING).max(MIN_PANEL_WIDTH);
    let body_h = ROW_H * model::TIPS.len() as f32 + 2.0 * PADDING;
    let title_h = TITLE_H + PADDING;
    let panel_h = title_h + body_h;

    // Bottom-right by default, bottom-left when the cursor sits in the
    // zone the panel would otherwise cover (`DxScreenCapture.cpp:775-779`).
    let right_left = screen.right() - SCREEN_MARGIN - panel_w;
    let top = screen.bottom() - SCREEN_MARGIN - panel_h;
    let left = if cursor.x > right_left - 2.0 * SCREEN_MARGIN && cursor.y > top - 2.0 * SCREEN_MARGIN {
        screen.left() + SCREEN_MARGIN
    } else {
        right_left
    };
    let panel = Rect::from_min_size(pos2(left, top), vec2(panel_w, panel_h));

    TipsLayout {
        panel,
        title_bar: Rect::from_min_size(panel.min, vec2(panel_w, title_h)),
        col_hotkey_x: PADDING,
        col_desc_x: PADDING + DESC_INDENT,
        rows_y: title_h + PADDING,
    }
}

pub fn show(p: &Painter, t: &TipsInputs, screen: Rect) {
    let body = |s: &str| p.layout_no_wrap(s.to_owned(), FontId::new(BODY_PT, FontFamily::Proportional), Color32::BLACK);
    let title = p.layout_no_wrap(
        model::TITLE.to_owned(),
        FontId::new(TITLE_PT, fonts::SEMIBOLD.clone()),
        Color32::WHITE,
    );
    let rows: Vec<_> = t.rows.iter().map(|s| body(s)).collect();
    let keys: Vec<_> = model::TIPS
        .iter()
        .map(|r| body(r.hotkey))
        .collect();
    let longest = rows
        .iter()
        .map(|g| g.size().x)
        .fold(0.0, f32::max);
    let l = compute_layout(screen, t.cursor, longest, title.size().x);

    let pr = l.panel;
    let s = PADDING_HALF;
    // Two literal strips rather than one offset rect: the body is 70 %
    // translucent, so a full shadow rect underneath would tint it. The
    // right strip owns the corner; the bottom one stops where it starts.
    p.rect_filled(
        Rect::from_min_max(pos2(pr.right(), pr.top() + s), pos2(pr.right() + s, pr.bottom() + s)),
        0u8,
        SHADOW,
    );
    p.rect_filled(
        Rect::from_min_max(pos2(pr.left() + s, pr.bottom()), pos2(pr.right(), pr.bottom() + s)),
        0u8,
        SHADOW,
    );
    p.rect_filled(
        Rect::from_min_max(pos2(pr.left(), l.title_bar.bottom()), pr.max),
        0u8,
        Color32::WHITE.gamma_multiply(OPACITY),
    );
    p.rect_filled(l.title_bar, 0u8, t.accent.gamma_multiply(OPACITY));
    p.galley(l.title_bar.center() - title.size() / 2.0, title, Color32::WHITE);

    let x = pr.left() + l.col_hotkey_x;
    let dx = pr.left() + l.col_desc_x;
    let row = |y: f32, key: &Arc<Galley>, desc: &Arc<Galley>| {
        p.galley(pos2(x, pr.top() + y), key.clone(), Color32::BLACK);
        p.galley(pos2(dx, pr.top() + y), desc.clone(), Color32::BLACK);
    };
    for (k, (key, desc)) in keys.iter().zip(&rows).enumerate() {
        row(l.rows_y + k as f32 * ROW_H, key, desc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::InteractionState;
    use crate::ui::shared::monitor_index_at;
    use flicksnap_shared::geometry::{RectExt, ScreenPointF, ScreenRect};

    fn screen() -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0))
    }

    /// The mixed pair every per-host rule is checked against: the primary
    /// at 100 % and its right-hand neighbour at 150 %.
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
            hovered_monitor_name: Some("DELL U2723"),
        }
    }

    /// A state with the tips panel up: the one thing the neutral fixture
    /// does not already have.
    fn tips_state() -> InteractionState {
        let mut input = InteractionState::new();
        input.tips_visible = true;
        input.virtual_cursor = ScreenPointF::new(300.0, 300.0);
        input
    }

    /// The panel's home corner: inset by the margin from the bottom and
    /// the right, with the cursor far away from it.
    #[test]
    fn layout_anchors_bottom_right_at_the_margin() {
        let s = screen();
        let l = compute_layout(s, pos2(10.0, 10.0), 200.0, 100.0);
        assert_eq!(l.panel.right(), s.right() - SCREEN_MARGIN);
        assert_eq!(l.panel.bottom(), s.bottom() - SCREEN_MARGIN);
        assert!((l.title_bar.height() - (TITLE_H + PADDING)).abs() < 1e-3);
        assert_eq!(l.title_bar.width(), l.panel.width());
        // The rows start under the title bar and fill the body.
        assert!((l.rows_y - (l.title_bar.height() + PADDING)).abs() < 1e-3);
        let body = l.panel.height() - l.title_bar.height();
        assert!((body - (ROW_H * model::TIPS.len() as f32 + 2.0 * PADDING)).abs() < 1e-3);
    }

    /// The cursor in the bottom-right corner zone sends the panel to the
    /// other side, so it never hides what the user is pointing at.
    #[test]
    fn layout_flips_left_when_the_cursor_is_in_the_corner_zone() {
        let s = screen();
        let home = compute_layout(s, pos2(10.0, 10.0), 200.0, 100.0).panel;
        let flipped = compute_layout(s, pos2(s.right() - 20.0, s.bottom() - 20.0), 200.0, 100.0).panel;
        assert_eq!(flipped.left(), s.left() + SCREEN_MARGIN);
        assert_eq!(flipped.width(), home.width());
        assert_eq!(flipped.top(), home.top());

        // Just outside the zone on either axis keeps the home corner.
        let x_only = compute_layout(s, pos2(s.right() - 20.0, 10.0), 200.0, 100.0).panel;
        assert_eq!(x_only.left(), home.left());
        let y_only = compute_layout(s, pos2(10.0, s.bottom() - 20.0), 200.0, 100.0).panel;
        assert_eq!(y_only.left(), home.left());
    }

    /// Short rows never shrink the panel below the floor; long ones grow
    /// it by the hotkey column and the padding on both sides.
    #[test]
    fn layout_width_is_at_least_the_minimum() {
        let s = screen();
        assert_eq!(
            compute_layout(s, Pos2::ZERO, 10.0, 10.0)
                .panel
                .width(),
            MIN_PANEL_WIDTH
        );
        let wide = compute_layout(s, Pos2::ZERO, 600.0, 100.0)
            .panel
            .width();
        assert_eq!(wide, 600.0 + DESC_INDENT + 2.0 * PADDING);
        // The title can be the widest thing on the panel.
        let by_title = compute_layout(s, Pos2::ZERO, 100.0, 700.0)
            .panel
            .width();
        assert_eq!(by_title, 700.0 + 2.0 * PADDING);
    }

    /// The old `shared::tips_visibility` rule, now with the host question
    /// answered: the cursor's monitor draws it, alone.
    #[test]
    fn tips_hide_during_capture_mouse_down_or_debug() {
        let m = monitors();
        let mut input = tips_state();

        let shown = inputs(0, &m[0], &ctx(&input, &m)).expect("the cursor is on the primary");
        assert_eq!(shown.cursor, pos2(300.0, 300.0));
        assert!(shown.rows[0].contains("DELL U2723"), "{}", shown.rows[0]);
        assert!(inputs(1, &m[1], &ctx(&input, &m)).is_none(), "only the cursor's host draws it");

        input.mouse_down = true;
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none());
        input.mouse_down = false;

        input.debug_visible = true;
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none());
        input.debug_visible = false;

        input.captured = true;
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none());
        input.captured = false;

        input.overlays_visible = false;
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none(), "Q hides it");
        input.overlays_visible = true;

        input.tips_visible = false;
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none(), "T put it away");
        input.tips_visible = true;

        // On the 150 % neighbour the panel is that host's, and the cursor
        // arrives in its own points.
        input.virtual_cursor = ScreenPointF::new(2520.0, 300.0);
        assert!(inputs(0, &m[0], &ctx(&input, &m)).is_none());
        let far = inputs(1, &m[1], &ctx(&input, &m)).expect("the cursor moved to the neighbour");
        assert_eq!(far.cursor, pos2(400.0, 200.0));
    }
}
