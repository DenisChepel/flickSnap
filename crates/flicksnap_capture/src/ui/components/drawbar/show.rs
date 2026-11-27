//! The drawing strip and its colour popup as egui `Area`s.
//!
//! The strip is a column on the action tray's chassis: the four tool
//! tiles, a rule, the colour tile and UNDO, every tile the capture
//! strip's bare icon square (`tokens::KEY_TILE`). No emblem and no key letters — the
//! strip has no accelerators, so its hover chips name the tool alone.
//! The picked tool wears the accent fill, as draw-on-screen's does. On a
//! monitor too low for the column it lies down as a row ([`axis_on`]).
//!
//! The colour tile opens a popup of draw-on-screen's eight swatches
//! lying alongside the strip, kept off the action tray ([`popup_rect`]).
//! Picking one closes it; so do Escape and a press anywhere but the popup
//! (`app.rs` owns that half, since it is the one that sees the keys and
//! the presses the popup never gets).

use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, Vec2};

use super::place;
use crate::draw::{DrawState, DrawTool, PALETTE, PALETTE_NAMES};
use crate::ui::command::Command;
use crate::ui::components::panel::assets::{self, Svg};
use crate::ui::components::panel::place::{Axis, Near};
use crate::ui::components::panel::show::{show_tip, HoveredTip};
use crate::ui::components::panel::theme::{self, tokens};
use crate::ui::components::panel::widgets::{self, StackedButton};
use crate::ui::egui_host::PanelOutcome;
use crate::ui::shared::UiMonitor;
use flicksnap_shared::geometry::{RectExt, ScreenRect};

/// The strip's Area id.
pub const DRAWBAR_ID: &str = "flicksnap-drawbar";
/// The strip's tooltip chip, and the key its hover clock is kept under.
pub const DRAWBAR_TIP_ID: &str = "flicksnap-drawbar-tip";
/// The colour popup's Area id.
pub const PALETTE_ID: &str = "flicksnap-drawbar-palette";
/// The popup's tooltip chip.
pub const PALETTE_TIP_ID: &str = "flicksnap-drawbar-palette-tip";

/// White at 10 %: the hover on the picked tool's accent fill, the C#
/// strips' `WhiteVeil10` over an active `TrayButton`.
const ACTIVE_HOVER_VEIL: f32 = 0.10;
/// How faint a button with nothing to do is drawn: the C# strips'
/// `:disabled` opacity.
const DISABLED_OPACITY: f32 = 0.4;
/// The colour dot in the colour tile and in every swatch
/// (`TrayTokens.SwatchDot`, at the strips' scale), and the hairline just
/// inside its edge that keeps a dark colour readable on the graphite
/// chassis (`TrayTokens.DotHairline`, white at 18 %).
const DOT: f32 = tokens::scaled(16.0);
const DOT_HAIRLINE: Stroke = Stroke {
    width: 1.0,
    color: Color32::from_rgba_premultiplied(46, 46, 46, 46),
};
/// A swatch's tile (`TrayTokens.MiniButtonSize`), the gap between two
/// swatches (draw-on-screen's `DotSpacing`) and the selection ring: a
/// `SwatchRing`-wide line a `SwatchGap` off the dot — all at the strips'
/// scale.
const SWATCH: f32 = tokens::scaled(26.0);
const SWATCH_SPACING: f32 = tokens::scaled(2.0);
const SWATCH_GAP: f32 = tokens::scaled(2.0);
const SWATCH_RING: f32 = 1.5 * tokens::SCALE;
/// Between the strip and the popup (`DrawColorPaletteWindow.ToolbarGap`).
const POPUP_GAP: f32 = 10.0;
/// How far, in points, two edges computed to meet may miss each other
/// and still be taken as meeting: the arithmetic that lines the popup up
/// with the tray or the screen's edge is a few float roundings deep.
const EDGE_SLACK: f32 = 0.01;

/// Everything the strip's contents depend on.
#[derive(Clone, Copy, PartialEq)]
pub struct DrawbarInputs {
    pub state: DrawState,
    /// Whether UNDO has a mark to take back.
    pub can_undo: bool,
}

/// One tile of the strip. Top to bottom: every [`DrawTool`], then — after
/// the rule — [`Tile::AFTER_RULE`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Tile {
    Tool(DrawTool),
    Palette,
    Undo,
}

impl Tile {
    const AFTER_RULE: [Tile; 2] = [Tile::Palette, Tile::Undo];

    fn tip(self) -> &'static str {
        match self {
            Tile::Tool(DrawTool::Brush) => "Brush",
            Tile::Tool(DrawTool::Line) => "Line",
            Tile::Tool(DrawTool::Arrow) => "Arrow",
            Tile::Tool(DrawTool::Rectangle) => "Rectangle",
            Tile::Palette => "Color",
            Tile::Undo => "Undo",
        }
    }

    fn icon(self) -> Option<&'static Svg> {
        match self {
            Tile::Tool(DrawTool::Brush) => Some(&assets::DRAW_BRUSH),
            Tile::Tool(DrawTool::Line) => Some(&assets::DRAW_LINE),
            Tile::Tool(DrawTool::Arrow) => Some(&assets::DRAW_ARROW),
            Tile::Tool(DrawTool::Rectangle) => Some(&assets::DRAW_RECTANGLE),
            Tile::Palette => None,
            Tile::Undo => Some(&assets::DRAW_UNDO),
        }
    }

    fn command(self) -> Command {
        match self {
            Tile::Tool(tool) => Command::DrawTool(tool),
            Tile::Palette => Command::DrawPalette,
            Tile::Undo => Command::DrawUndo,
        }
    }
}

/// The strip's content size in points along `axis` (the chassis padding
/// not included): one tile across, the two runs of tiles flush with a rule
/// between them.
fn content_size(axis: Axis) -> Vec2 {
    let tiles = (DrawTool::ALL.len() + Tile::AFTER_RULE.len()) as f32 * tokens::KEY_TILE;
    let along = tiles + tokens::DIVIDER_BLOCK + 2.0 * tokens::GAP;
    match axis {
        Axis::Column => vec2(tokens::KEY_TILE, along),
        Axis::Row => vec2(along, tokens::KEY_TILE),
    }
}

/// The strip box in physical pixels, rounded once per axis the way the
/// tray's is.
fn outer_px(axis: Axis, ppp: f32) -> (i32, i32) {
    let outer = content_size(axis) + Vec2::splat(2.0 * tokens::PAD);
    ((outer.x * ppp).round() as i32, (outer.y * ppp).round() as i32)
}

/// Which way the strip runs on `monitor`: a column, unless the column
/// would not fit the monitor's height (a low monitor at a high scale:
/// 1080 px at 600 % is 180 pt tall), and then a row, which is the same
/// strip lying down.
fn axis_on(monitor: UiMonitor) -> Axis {
    let ppp = monitor.dpi_scale.max(0.1);
    let (_, column_h) = outer_px(Axis::Column, ppp);
    if column_h + 2 * Near::at_dpi(ppp).min_distance <= monitor.bounds.height() {
        Axis::Column
    } else {
        Axis::Row
    }
}

/// The popup's size in points, chassis included: the eight swatches in a
/// run along `axis` — the strip's, so the popup lies alongside it.
fn popup_size(axis: Axis) -> Vec2 {
    let n = PALETTE.len() as f32;
    let along = n * SWATCH + (n - 1.0) * SWATCH_SPACING;
    let content = match axis {
        Axis::Column => vec2(SWATCH, along),
        Axis::Row => vec2(along, SWATCH),
    };
    content + Vec2::splat(2.0 * tokens::PAD)
}

/// Where the popup goes, in points, against the strip at `bar`.
///
/// The flanks it can stand on, in order of preference: across a column,
/// the side away from the selection first (right of a strip beside the
/// selection's right edge, left of any other — a strip tucked inside the
/// selection has its interior to the left), then the other side, then
/// below and above it; along a row, below and above first. On each flank
/// it is lined up with the colour tile at `tile`, and failing that slid
/// along the strip to clear `avoid` (the action tray): ending just short
/// of it, then starting just past it — but only while it still runs
/// alongside some of the strip, so it reads as the strip's. The first
/// place wholly on `screen` and clear of `avoid` wins; with none clear,
/// the first on the screen; with none on it either, the first slid onto
/// it.
///
/// Every place starts on a whole pixel at `ppp`, rounded toward the
/// screen's top-left. egui pins an Area to the nearest pixel, and the
/// popup is a fractional number of pixels wide at most scales, so a place
/// computed flush with the screen's far edge could otherwise be pushed
/// half a pixel past it.
fn popup_rect(side: place::Side, axis: Axis, bar: Rect, tile: Rect, screen: Rect, avoid: Rect, ppp: f32) -> (Rect, f32) {
    #[derive(Clone, Copy)]
    enum At {
        Right,
        Left,
        Below,
        Above,
    }
    let size = popup_size(axis);
    let scale = theme::fit_scale(size, screen.size());
    let size = size * scale;
    let slide = |pos: f32, len: f32, lo: f32, hi: f32| pos.min(hi - len).max(lo);
    let on_pixel = |min: egui::Pos2| Rect::from_min_size(pos2((min.x * ppp).floor() / ppp, (min.y * ppp).floor() / ppp), size);

    // Where the popup can start along a flank: level with the tile, then
    // ending short of the tray, then starting past it. Slid onto the
    // screen along the flank, never across it.
    let starts = |tile_mid: f32, len: f32, avoid_lo: f32, avoid_hi: f32, lo: f32, hi: f32| {
        [tile_mid - len / 2.0, avoid_lo - POPUP_GAP - len, avoid_hi + POPUP_GAP].map(|p| slide(p, len, lo, hi))
    };
    let ys = starts(tile.center().y, size.y, avoid.top(), avoid.bottom(), screen.top(), screen.bottom());
    let xs = starts(tile.center().x, size.x, avoid.left(), avoid.right(), screen.left(), screen.right());
    let places = |at: At| {
        (0..3).map(move |i| {
            on_pixel(match at {
                At::Right => pos2(bar.right() + POPUP_GAP, ys[i]),
                At::Left => pos2(bar.left() - POPUP_GAP - size.x, ys[i]),
                At::Below => pos2(xs[i], bar.bottom() + POPUP_GAP),
                At::Above => pos2(xs[i], bar.top() - POPUP_GAP - size.y),
            })
        })
    };
    // Still the strip's: some of the popup runs alongside some of it.
    let alongside = |r: &Rect| r.x_range().intersects(bar.x_range()) || r.y_range().intersects(bar.y_range());
    let order = match (axis, side) {
        (Axis::Column, place::Side::Right) => [At::Right, At::Left, At::Below, At::Above],
        (Axis::Column, _) => [At::Left, At::Right, At::Below, At::Above],
        (Axis::Row, _) => [At::Below, At::Above, At::Right, At::Left],
    };
    // Edges may touch, give or take float noise: the gap is already in
    // `keep_off`, a place flush with the screen's edge is on it, and
    // egui's own tests count a shared edge as an overlap.
    let keep_off = avoid.expand(POPUP_GAP).shrink(EDGE_SLACK);
    let clear = |r: &Rect| !r.intersects(keep_off);
    let room = screen.expand(EDGE_SLACK);
    let on_screen = order
        .into_iter()
        .flat_map(places)
        .filter(|r| room.contains_rect(*r) && alongside(r));
    let rect = on_screen
        .clone()
        .find(clear)
        .or_else(|| on_screen.clone().next())
        .unwrap_or_else(|| {
            let r = places(order[0])
                .next()
                .unwrap_or(Rect::from_min_size(tile.min, size));
            on_pixel(pos2(
                slide(r.left(), size.x, screen.left(), screen.right()),
                slide(r.top(), size.y, screen.top(), screen.bottom()),
            ))
        });
    (rect, scale)
}

/// Build the strip (and the popup, when it is open) for one monitor and
/// report what the pointer found on them.
///
/// `anchor` is the selection clipped to this monitor, `accent` the
/// picked tool's fill, `tray` the action tray's box in physical px — the
/// strip and the popup keep clear of it. `visible` is the Q toggle, honoured the way
/// the tray honours it: laid out and hit-tested, painted at opacity 0.
pub fn show(
    ctx: &egui::Context,
    d: &DrawbarInputs,
    anchor: ScreenRect,
    accent: Color32,
    tray: ScreenRect,
    monitor: UiMonitor,
    visible: bool,
) -> PanelOutcome {
    let ppp = monitor.dpi_scale.max(0.1);
    let axis = axis_on(monitor);
    let (w, h) = outer_px(axis, ppp);
    let scale = theme::fit_scale(
        vec2(w as f32, h as f32),
        vec2(monitor.bounds.width() as f32, monitor.bounds.height() as f32),
    );
    let (side, rect_px) = place::place(anchor, monitor.bounds, outer_px(axis, ppp * scale), tray, Near::at_dpi(ppp));
    // This monitor's points, where egui works.
    let local = |x: i32, y: i32| pos2((x - monitor.bounds.left()) as f32 / ppp, (y - monitor.bounds.top()) as f32 / ppp);
    let tray = Rect::from_min_max(local(tray.left(), tray.top()), local(tray.right(), tray.bottom()));

    let mut out = PanelOutcome::default();
    let mut hovered: Option<HoveredTip> = None;
    let mut palette_tile = Rect::NOTHING;
    let (area, transform) = theme::tray_area(ctx, DRAWBAR_ID, local(rect_px.left(), rect_px.top()), scale);
    let inner = area.show(ctx, |ui| {
        if !visible {
            ui.set_opacity(0.0);
        }
        theme::tray_frame().show(ui, |ui| {
            let strip = |ui: &mut egui::Ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for tool in DrawTool::ALL {
                    strip_tile(ui, d, Tile::Tool(tool), accent, &mut out, &mut hovered);
                }
                ui.add_space(tokens::GAP);
                widgets::divider(ui, axis == Axis::Row, tokens::KEY_TILE);
                ui.add_space(tokens::GAP);
                for tile in Tile::AFTER_RULE {
                    let r = strip_tile(ui, d, tile, accent, &mut out, &mut hovered);
                    if tile == Tile::Palette {
                        palette_tile = r;
                    }
                }
            };
            match axis {
                Axis::Column => ui.vertical(strip),
                Axis::Row => ui.horizontal(strip),
            };
        });
    });
    out.over_tray |= inner.response.contains_pointer();
    let bar = transform * inner.response.rect;
    let hovered = hovered.map(|(id, rect, job)| (id, transform * rect, job));
    show_tip(ctx, DRAWBAR_TIP_ID, axis, bar, hovered, visible);

    if d.state.palette_open {
        let (popup, scale) = popup_rect(side, axis, bar, transform * palette_tile, ctx.viewport_rect(), tray, ppp);
        let (area, transform) = theme::tray_area(ctx, PALETTE_ID, popup.min, scale);
        popup_area(ctx, d, axis, (area, transform), accent, visible, &mut out);
    }
    out
}

/// One tile of the strip. The colour tile paints its own face; every
/// other tile is the capture strip's icon square. Returns the tile's rect.
fn strip_tile(
    ui: &mut egui::Ui,
    d: &DrawbarInputs,
    tile: Tile,
    accent: Color32,
    out: &mut PanelOutcome,
    hovered: &mut Option<HoveredTip>,
) -> Rect {
    let id = egui::Id::unique(("drawbar", tile));
    let size = Vec2::splat(tokens::KEY_TILE);
    let enabled = tile != Tile::Undo || d.can_undo;
    let active = matches!(tile, Tile::Tool(tool) if d.state.tool == Some(tool));
    let response = match tile.icon() {
        Some(icon) => {
            let (base, veil) = if active {
                (accent, ACTIVE_HOVER_VEIL)
            } else if enabled {
                (tokens::TRAY_FILL, tokens::HOVER_VEIL_BARE)
            } else {
                (tokens::TRAY_FILL, 0.0)
            };
            let button = StackedButton {
                id,
                icon: icon.source(),
                text: None,
                min_size: size,
                direction: egui::Direction::LeftToRight,
                gap: 0.0,
                pad_h: tokens::KEY_PAD_H,
                base,
                veil,
                align: egui::Align2::CENTER_CENTER,
            };
            if enabled {
                button.show(ui)
            } else {
                ui.scope(|ui| {
                    ui.multiply_opacity(DISABLED_OPACITY);
                    button.show(ui)
                })
                .inner
            }
        }
        None => color_tile(ui, id, size, d.state.ink(), d.state.palette_open),
    };
    if enabled {
        out.over_button |= response.contains_pointer();
        if response.clicked() {
            out.clicked = Some(tile.command());
        }
        // The colour tile's chip would sit under the popup it opened.
        let tip_shown = !(tile == Tile::Palette && d.state.palette_open);
        if response.contains_pointer() && tip_shown {
            *hovered = Some((id, response.rect, widgets::tip_text_job(tile.tip(), None)));
        }
    }
    response.rect
}

/// The colour tile: the current ink as a dot on a bare tile. It never
/// lights up the way the other tiles do; the pointer on it — or its popup
/// open — rings the dot in white instead, as draw-on-screen's colour
/// swatch is ringed.
fn color_tile(ui: &mut egui::Ui, id: egui::Id, size: Vec2, ink: Color32, open: bool) -> egui::Response {
    let rect = ui.allocate_space(size).1;
    let response = ui.interact(rect, id, Sense::CLICK);
    if response.hovered() || open {
        ring(ui.painter(), rect.center(), tokens::FG);
    }
    dot(ui.painter(), rect.center(), ink);
    response
}

/// The ring a swatch wears around its dot: a `SwatchRing`-wide line a
/// `SwatchGap` off it, as `TraySwatch` draws its `RingBrush`.
fn ring(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let radius = DOT / 2.0 + SWATCH_GAP + SWATCH_RING / 2.0;
    painter.circle_stroke(center, radius, Stroke::new(SWATCH_RING, color));
}

/// A colour dot with its hairline, as `TrayDots.Paint` draws one with
/// `alwaysEdged`.
fn dot(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let r = DOT / 2.0;
    painter.circle_filled(center, r, color);
    painter.circle_stroke(center, r - DOT_HAIRLINE.width / 2.0, DOT_HAIRLINE);
}

/// The popup: the eight swatches alongside the strip on the tray's
/// chassis, in a column beside a column and a row beside a row. The
/// picked colour wears a white ring, a hovered one an accent ring.
fn popup_area(
    ctx: &egui::Context,
    d: &DrawbarInputs,
    axis: Axis,
    (area, transform): (egui::Area, egui::emath::TSTransform),
    accent: Color32,
    visible: bool,
    out: &mut PanelOutcome,
) {
    let mut hovered: Option<HoveredTip> = None;
    let inner = area.show(ctx, |ui| {
        if !visible {
            ui.set_opacity(0.0);
        }
        theme::tray_frame().show(ui, |ui| {
            let swatches = |ui: &mut egui::Ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(SWATCH_SPACING);
                for (i, color) in PALETTE.iter().enumerate() {
                    let id = egui::Id::unique(("drawbar-swatch", i));
                    let (r, response) = ui.allocate_exact_size(Vec2::splat(SWATCH), Sense::CLICK);
                    let ring_color = if i == d.state.color {
                        Some(tokens::FG)
                    } else if response.hovered() {
                        Some(accent)
                    } else {
                        None
                    };
                    if let Some(color) = ring_color {
                        ring(ui.painter(), r.center(), color);
                    }
                    dot(ui.painter(), r.center(), *color);
                    out.over_button |= response.contains_pointer();
                    if response.clicked() {
                        out.clicked = Some(Command::DrawColor(i));
                    }
                    if response.contains_pointer() {
                        hovered = Some((id, r, widgets::tip_text_job(PALETTE_NAMES[i], None)));
                    }
                }
            };
            match axis {
                Axis::Column => ui.vertical(swatches),
                Axis::Row => ui.horizontal(swatches),
            };
        });
    });
    let over = inner.response.contains_pointer();
    out.over_tray |= over;
    out.over_palette |= over;
    let hovered = hovered.map(|(id, rect, job)| (id, transform * rect, job));
    show_tip(ctx, PALETTE_TIP_ID, axis, transform * inner.response.rect, hovered, visible);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::intersect_rects;
    use crate::ui::components::panel::model::{ButtonStyle, PanelButtonSet, PanelFeatures};
    use crate::ui::components::panel::show::PANEL_ID;
    use crate::ui::components::{compose, OverlayInputs};
    use crate::ui::egui_host::{HostInputs, PanelInputs};
    use egui::Pos2;

    const ACCENT: Color32 = Color32::from_rgb(0x2F, 0x7C, 0xAE);

    fn monitor(dpi: f32) -> UiMonitor {
        UiMonitor {
            bounds: ScreenRect::from_xy_size(0, 0, 1920, 1080),
            dpi_scale: dpi,
            is_primary: true,
        }
    }

    fn inputs(monitor: UiMonitor, selection: ScreenRect, state: DrawState) -> HostInputs {
        HostInputs {
            panel: Some(PanelInputs {
                set: PanelButtonSet::Normal,
                features: PanelFeatures::ALL,
                style: ButtonStyle::KeyHint,
                accent: ACCENT,
                word_count: 0,
                anchor: intersect_rects(monitor.bounds, selection).expect("the selection is on the monitor"),
            }),
            drawbar: Some(DrawbarInputs {
                state,
                can_undo: false,
            }),
            debug: None,
            pointer: None,
            overlays: OverlayInputs {
                overlays_visible: true,
                ..Default::default()
            },
        }
    }

    /// A styled, fonted context running the whole of `compose`, so the
    /// strip is laid out beside a real tray, the way a host lays it out.
    struct Harness {
        ctx: egui::Context,
        monitor: UiMonitor,
    }

    impl Harness {
        fn new(monitor: UiMonitor) -> Self {
            let ctx = egui::Context::default();
            egui_extras::install_image_loaders(&ctx);
            ctx.set_fonts(crate::ui::fonts::font_definitions(&[]));
            theme::apply_style(&ctx);
            Self {
                ctx,
                monitor,
            }
        }

        fn raw(&self, events: Vec<egui::Event>) -> egui::RawInput {
            let ppp = self.monitor.dpi_scale;
            let b = self.monitor.bounds;
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(b.width() as f32, b.height() as f32) / ppp)),
                viewports: [(
                    egui::ViewportId::ROOT,
                    egui::ViewportInfo {
                        native_pixels_per_point: Some(ppp),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                events,
                focused: true,
                ..Default::default()
            }
        }

        /// Three passes, the press (if any) on the second — as the host
        /// feeds one — with `clicked` ORed over them.
        fn tick(&mut self, inputs: &HostInputs, pointer: Option<Pos2>, press: bool) -> PanelOutcome {
            let mut acc = PanelOutcome::default();
            for pass in 0..3 {
                let mut events = vec![match pointer {
                    Some(p) => egui::Event::PointerMoved(p),
                    None => egui::Event::PointerGone,
                }];
                if let (Some(pos), true, 1) = (pointer, press, pass) {
                    for pressed in [true, false] {
                        events.push(egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                let raw = self.raw(events);
                let monitor = self.monitor;
                let mut out = PanelOutcome::default();
                self.ctx
                    .run_ui(raw, |ui| out = compose(ui.ctx(), inputs, monitor))
                    .drop_without_applying_deltas();
                acc = PanelOutcome {
                    clicked: acc.clicked.or(out.clicked),
                    ..out
                };
            }
            acc
        }

        fn area(&self, id: &str) -> Option<Rect> {
            let id = egui::Id::unique(id);
            let layer = egui::LayerId::new(egui::Order::Foreground, id);
            let transform = self
                .ctx
                .layer_transform_to_global(layer)
                .unwrap_or_default();
            self.ctx
                .memory(|m| m.area_rect(id))
                .map(|rect| transform * rect)
        }

        /// The `side`-square interactive rects inside `area`, top to
        /// bottom, then left to right.
        fn tiles_in(&self, area: Rect, side: f32) -> Vec<Rect> {
            let mut tiles: Vec<Rect> = self
                .ctx
                .interactive_rects_last_pass()
                .into_iter()
                .filter(|r| area.contains_rect(*r) && (r.width() - side).abs() <= 0.5 && (r.height() - side).abs() <= 0.5)
                .collect();
            tiles.sort_by(|a, b| {
                a.top()
                    .total_cmp(&b.top())
                    .then(a.left().total_cmp(&b.left()))
            });
            tiles
        }
    }

    fn selection() -> ScreenRect {
        ScreenRect::from_xy_size(500, 300, 600, 400)
    }

    /// Right of the selection, level with its bottom, one tile wide: the
    /// six tiles in a column with the rule between the tools and the rest,
    /// at exactly the size the placement was computed for. The selection is
    /// taller than the strip at every DPI, so nothing pulls it off that
    /// corner.
    #[test]
    fn the_strip_stands_right_of_the_selection_as_one_column_of_six_tiles() {
        let tall = ScreenRect::from_xy_size(500, 200, 600, 700);
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            let mon = monitor(dpi);
            let mut h = Harness::new(mon);
            let p = inputs(mon, tall, DrawState::default());
            h.tick(&p, None, false);
            let bar = h
                .area(DRAWBAR_ID)
                .expect("the strip has an area");
            let tol = 1.0 / dpi + 1e-3;
            let left = (tall.right() + Near::at_dpi(dpi).max_distance) as f32 / dpi;
            assert!((bar.left() - left).abs() <= tol, "{bar:?} at {dpi}");
            assert!((bar.bottom() - tall.bottom() as f32 / dpi).abs() <= tol, "{bar:?} at {dpi}");
            let (w, ht) = outer_px(Axis::Column, dpi);
            assert!(
                (bar.width() - w as f32 / dpi).abs() <= tol && (bar.height() - ht as f32 / dpi).abs() <= tol,
                "{bar:?} at {dpi}"
            );

            let tiles = h.tiles_in(bar, tokens::KEY_TILE);
            assert_eq!(tiles.len(), 6, "at {dpi}: {tiles:?}");
            for (i, pair) in tiles.windows(2).enumerate() {
                let gap = pair[1].top() - pair[0].bottom();
                let want = if i == 3 { 2.0 * tokens::GAP + tokens::DIVIDER_BLOCK } else { 0.0 };
                assert!((gap - want).abs() <= 0.5, "at {dpi}, between {i} and {}: {gap}", i + 1);
            }
        }
    }

    /// Both strips hang off the selection's bottom-right corner: the tray
    /// beneath it ends at its right edge, the drawing strip beside it ends
    /// at its bottom edge.
    #[test]
    fn both_strips_meet_at_the_selections_bottom_right_corner() {
        for dpi in [1.0, 1.25, 1.5, 2.0] {
            let mon = monitor(dpi);
            let mut h = Harness::new(mon);
            h.tick(&inputs(mon, selection(), DrawState::default()), None, false);
            let tray = h.area(PANEL_ID).expect("a tray");
            let bar = h.area(DRAWBAR_ID).expect("a strip");
            let corner = pos2(selection().right() as f32, selection().bottom() as f32) / dpi;
            let tol = 1.0 / dpi + 1e-3;
            assert!((tray.right() - corner.x).abs() <= tol, "at {dpi}: tray {tray:?}, corner {corner:?}");
            assert!(tray.top() > corner.y, "at {dpi}: the tray is beneath the selection");
            assert!((bar.bottom() - corner.y).abs() <= tol, "at {dpi}: strip {bar:?}, corner {corner:?}");
            assert!(bar.left() > corner.x, "at {dpi}: the strip is right of the selection");
        }
    }

    /// A selection with no room beneath it: the tray jumps to the top-right
    /// corner, the full gap above the selection, while the drawing strip
    /// keeps the right flank. The two never share a side.
    #[test]
    fn with_no_room_beneath_the_tray_goes_to_the_top_right_corner() {
        for dpi in [1.0, 1.25, 1.5] {
            let mon = monitor(dpi);
            let sel = ScreenRect::from_xy_size(500, 600, 600, 450);
            let mut h = Harness::new(mon);
            h.tick(&inputs(mon, sel, DrawState::default()), None, false);
            let tray = h.area(PANEL_ID).expect("a tray");
            let bar = h.area(DRAWBAR_ID).expect("a strip");
            let gap = Near::at_dpi(dpi).max_distance as f32 / dpi;
            let tol = 1.0 / dpi + 1e-3;
            assert!(
                (tray.bottom() - (sel.top() as f32 / dpi - gap)).abs() <= tol,
                "at {dpi}: {tray:?} is not the gap above the selection"
            );
            assert!((tray.right() - sel.right() as f32 / dpi).abs() <= tol, "at {dpi}: {tray:?}");
            assert!(bar.left() > sel.right() as f32 / dpi, "at {dpi}: the strip keeps the right flank");
            assert!(!bar.intersects(tray));
        }
    }

    /// Each tool tile answers with its tool and the colour tile with the
    /// popup; UNDO, with nothing to take back, answers with nothing and is
    /// not a button the pointer can be on.
    #[test]
    fn every_tile_answers_with_its_command_and_undo_waits_for_a_mark() {
        let mon = monitor(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(mon, selection(), DrawState::default());
        h.tick(&p, None, false);
        let tiles = h.tiles_in(h.area(DRAWBAR_ID).expect("a strip"), tokens::KEY_TILE);
        let want = [
            Some(Command::DrawTool(DrawTool::Brush)),
            Some(Command::DrawTool(DrawTool::Arrow)),
            Some(Command::DrawTool(DrawTool::Line)),
            Some(Command::DrawTool(DrawTool::Rectangle)),
            Some(Command::DrawPalette),
            None,
        ];
        assert_eq!(tiles.len(), want.len());
        for (tile, want) in tiles.iter().zip(want) {
            let out = h.tick(&p, Some(tile.center()), true);
            assert_eq!(out.clicked, want, "{tile:?}");
            assert_eq!(out.over_button, want.is_some(), "{tile:?}");
            assert!(out.over_tray, "the strip owns the press: {tile:?}");
        }
    }

    /// With a mark drawn, UNDO is a button like any other and answers with
    /// its command.
    #[test]
    fn undo_answers_once_there_is_a_mark_to_take_back() {
        let mon = monitor(1.0);
        let mut h = Harness::new(mon);
        let mut p = inputs(mon, selection(), DrawState::default());
        if let Some(d) = &mut p.drawbar {
            d.can_undo = true;
        }
        h.tick(&p, None, false);
        let undo = *h
            .tiles_in(h.area(DRAWBAR_ID).expect("a strip"), tokens::KEY_TILE)
            .last()
            .expect("UNDO ends the strip");
        let out = h.tick(&p, Some(undo.center()), true);
        assert_eq!(out.clicked, Some(Command::DrawUndo));
        assert!(out.over_button);
    }

    /// The popup: eight swatches in a column beside the strip, each answering
    /// with its own colour, the whole of it reported as the popup.
    #[test]
    fn the_open_popup_offers_the_eight_colours_beside_the_strip() {
        let mon = monitor(1.0);
        let mut h = Harness::new(mon);
        let open = DrawState {
            palette_open: true,
            ..DrawState::default()
        };
        let p = inputs(mon, selection(), open);
        h.tick(&p, None, false);
        let bar = h.area(DRAWBAR_ID).expect("a strip");
        let popup = h.area(PALETTE_ID).expect("the popup is up");
        assert!((popup.left() - (bar.right() + POPUP_GAP)).abs() <= 0.5, "{popup:?} beside {bar:?}");
        assert!(!popup.intersects(bar));

        let swatches = h.tiles_in(popup, SWATCH);
        assert_eq!(swatches.len(), PALETTE.len(), "{swatches:?}");
        for (i, swatch) in swatches.iter().enumerate() {
            let out = h.tick(&p, Some(swatch.center()), true);
            assert_eq!(out.clicked, Some(Command::DrawColor(i)), "{swatch:?}");
            assert!(out.over_palette && out.over_tray);
        }
        let out = h.tick(&p, Some(bar.center()), false);
        assert!(!out.over_palette, "the strip is not the popup");
    }

    /// A shut popup is not laid out at all.
    #[test]
    fn a_shut_popup_is_not_there() {
        let mon = monitor(1.0);
        let mut h = Harness::new(mon);
        h.tick(&inputs(mon, selection(), DrawState::default()), None, false);
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(PALETTE_ID));
        assert!(!h
            .ctx
            .memory(|m| m.areas().visible_last_frame(&layer)));
    }

    /// Wherever the selection is, the strip and the tray never sit on each
    /// other and the strip stays on the monitor: a short narrow selection
    /// (the tray reaches under the strip), one hugging the bottom (the
    /// tray stands on its end to the right), one at the right edge, the
    /// whole monitor.
    #[test]
    fn the_strip_never_sits_on_the_tray() {
        let cases = [
            ScreenRect::from_xy_size(800, 500, 100, 60),
            ScreenRect::from_xy_size(500, 1000, 300, 60),
            ScreenRect::from_xy_size(1500, 300, 400, 400),
            ScreenRect::from_xy_size(0, 0, 1920, 1080),
            selection(),
            // At a high scale the tray outgrows a narrow selection near
            // the monitor's left edge and reaches out under the strip's
            // flank.
            ScreenRect::from_xy_size(0, 96, 300, 336),
            ScreenRect::from_xy_size(40, 500, 200, 120),
        ];
        for dpi in [1.0, 1.5, 2.0, 3.0] {
            for sel in cases {
                let mon = monitor(dpi);
                let mut h = Harness::new(mon);
                h.tick(&inputs(mon, sel, DrawState::default()), None, false);
                let bar = h.area(DRAWBAR_ID).expect("a strip");
                let tray = h.area(PANEL_ID).expect("a tray");
                assert!(!bar.intersects(tray), "{sel:?} at {dpi}: {bar:?} on {tray:?}");
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0) / dpi);
                assert!(screen.contains_rect(bar), "{sel:?} at {dpi}: {bar:?}");
            }
        }
    }

    /// The picked tool is the one tile wearing the accent.
    #[test]
    fn the_picked_tool_wears_the_accent() {
        let mon = monitor(1.0);
        let mut h = Harness::new(mon);
        let picked = DrawState {
            tool: Some(DrawTool::Line),
            ..DrawState::default()
        };
        let p = inputs(mon, selection(), picked);
        let accent_fills = |h: &mut Harness| {
            let raw = h.raw(Vec::new());
            let output = h.ctx.run_ui(raw, |ui| {
                compose(ui.ctx(), &p, mon);
            });
            let n = output
                .shapes
                .iter()
                .filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == ACCENT))
                .count();
            output.drop_without_applying_deltas();
            n
        };
        accent_fills(&mut h);
        assert_eq!(accent_fills(&mut h), 1);
    }

    #[test]
    fn the_popup_flips_to_the_side_with_room_and_stays_on_the_monitor() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0));
        let bar = Rect::from_min_size(pos2(1000.0, 300.0), vec2(48.0, 265.0));
        let tile = Rect::from_min_size(pos2(1004.0, 481.0), Vec2::splat(40.0));
        let no_tray = Rect::from_min_size(pos2(0.0, 1070.0), Vec2::splat(5.0));
        let at = |side, bar, tile| popup_rect(side, Axis::Column, bar, tile, screen, no_tray, 1.0).0;
        assert_eq!(at(place::Side::Right, bar, tile).left(), bar.right() + POPUP_GAP);
        assert_eq!(at(place::Side::Left, bar, tile).right(), bar.left() - POPUP_GAP);
        assert_eq!(at(place::Side::Inside, bar, tile).right(), bar.left() - POPUP_GAP);

        let shift = vec2(1920.0 - 52.0 - 1000.0, 0.0);
        let flipped = at(place::Side::Right, bar.translate(shift), tile.translate(shift));
        assert_eq!(flipped.right(), bar.translate(shift).left() - POPUP_GAP);

        // A strip ending near the bottom: the popup, centred on its colour
        // tile, would run off the monitor, and is slid back onto it.
        let low_bar = Rect::from_min_size(pos2(1000.0, 810.0), vec2(48.0, 265.0));
        let low_tile = Rect::from_min_size(pos2(1004.0, 991.0), Vec2::splat(40.0));
        let slid = at(place::Side::Right, low_bar, low_tile);
        assert_eq!(slid.left(), low_bar.right() + POPUP_GAP);
        assert_eq!(slid.bottom(), screen.bottom());
        assert_eq!(slid.size(), popup_size(Axis::Column));
    }

    /// The colour tile never lights up like the other tiles: the pointer
    /// on it rings the dot in white, and that is all.
    #[test]
    fn hovering_the_colour_tile_rings_the_dot_and_lights_no_tile() {
        let mon = monitor(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(mon, selection(), DrawState::default());
        h.tick(&p, None, false);
        let tile = h.tiles_in(h.area(DRAWBAR_ID).expect("a strip"), tokens::KEY_TILE)[4];
        let paint = |h: &mut Harness, pointer: Option<Pos2>| {
            let mut shapes = Vec::new();
            for _ in 0..3 {
                let raw = h.raw(vec![match pointer {
                    Some(p) => egui::Event::PointerMoved(p),
                    None => egui::Event::PointerGone,
                }]);
                let output = h.ctx.run_ui(raw, |ui| {
                    compose(ui.ctx(), &p, mon);
                });
                shapes = output
                    .shapes
                    .iter()
                    .map(|c| c.shape.clone())
                    .collect::<Vec<_>>();
                output.drop_without_applying_deltas();
            }
            let ring = shapes
                .iter()
                .any(|s| matches!(s, egui::Shape::Circle(c) if c.stroke.color == tokens::FG && tile.contains(c.center)));
            let tile_fill = shapes
                .iter()
                .any(|s| matches!(s, egui::Shape::Rect(r) if r.rect == tile));
            (ring, tile_fill)
        };
        assert_eq!(paint(&mut h, None), (false, false));
        assert_eq!(paint(&mut h, Some(tile.center())), (true, false));
    }

    /// The tray under the popup's own side and the other side off the
    /// monitor: the popup leaves the strip's flank and goes below it.
    #[test]
    fn the_popup_goes_below_the_strip_when_both_flanks_are_taken() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0));
        let bar = Rect::from_min_size(pos2(1870.0, 300.0), vec2(48.0, 265.0));
        let tile = Rect::from_min_size(pos2(1874.0, 481.0), Vec2::splat(40.0));
        // Beside the whole height of the strip on the left, so no slide
        // along that flank clears it either.
        let tray = Rect::from_min_max(pos2(1190.0, 250.0), pos2(1850.0, 620.0));
        let (popup, _) = popup_rect(place::Side::Right, Axis::Column, bar, tile, screen, tray, 1.0);
        assert!(screen.contains_rect(popup), "{popup:?}");
        assert!(!popup.intersects(tray) && !popup.intersects(bar), "{popup:?}");
        assert_eq!(popup.top(), bar.bottom() + POPUP_GAP);
    }

    /// The reviewer's case, end to end: a selection whose strip ends up
    /// near the monitor's right edge, beside a tray with every button,
    /// with the popup open. The popup must cover neither the tray nor the
    /// strip, and stay on the monitor — here and across DPIs and nearby
    /// selections.
    #[test]
    fn the_open_popup_never_covers_the_tray_or_the_strip() {
        let open = DrawState {
            palette_open: true,
            ..DrawState::default()
        };
        let cases = [
            ScreenRect::from_xy_size(1100, 300, 700, 169),
            ScreenRect::from_xy_size(1150, 300, 650, 120),
            ScreenRect::from_xy_size(900, 600, 900, 300),
            ScreenRect::from_xy_size(100, 300, 700, 169),
            ScreenRect::from_xy_size(800, 500, 100, 60),
            selection(),
        ];
        for dpi in [1.0, 1.25, 1.5] {
            for sel in cases {
                let mon = monitor(dpi);
                let mut h = Harness::new(mon);
                h.tick(&inputs(mon, sel, open), None, false);
                let tray = h.area(PANEL_ID).expect("a tray");
                let bar = h.area(DRAWBAR_ID).expect("a strip");
                let popup = h.area(PALETTE_ID).expect("the popup is up");
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0) / dpi);
                assert!(!popup.intersects(tray), "{sel:?} at {dpi}: {popup:?} on the tray {tray:?}");
                assert!(!popup.intersects(bar), "{sel:?} at {dpi}: {popup:?} on the strip {bar:?}");
                assert!(screen.contains_rect(popup), "{sel:?} at {dpi}: {popup:?}");
            }
        }
    }

    /// A scale at which 1080 px is shorter than the column: the strip lies
    /// down as a row, all six tiles side by side, on the monitor. At a
    /// scale with room, the column stays a column.
    #[test]
    fn a_monitor_too_low_for_the_column_gets_a_row() {
        let column_pt = outer_px(Axis::Column, 1.0).1 as f32;
        let roomy = 0.8 * 1080.0 / column_pt;
        let low = 1.2 * 1080.0 / column_pt;
        assert_eq!(axis_on(monitor(roomy)), Axis::Column);
        assert_eq!(axis_on(monitor(low)), Axis::Row);

        let mon = monitor(low);
        let mut h = Harness::new(mon);
        let p = inputs(mon, ScreenRect::from_xy_size(200, 100, 800, 500), DrawState::default());
        h.tick(&p, None, false);
        let bar = h.area(DRAWBAR_ID).expect("a strip");
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0) / low);
        assert!(screen.contains_rect(bar), "{bar:?} off {screen:?}");
        let tiles = h.tiles_in(bar, tokens::KEY_TILE);
        assert_eq!(tiles.len(), 6, "{tiles:?}");
        assert!(
            tiles
                .iter()
                .all(|t| (t.top() - tiles[0].top()).abs() <= 0.5),
            "one row: {tiles:?}"
        );
    }

    #[test]
    fn the_scaled_palette_keeps_all_swatches_on_screen_and_clickable() {
        let mon = UiMonitor {
            bounds: ScreenRect::from_xy_size(0, 0, 720, 640),
            ..monitor(4.0)
        };
        let mut h = Harness::new(mon);
        let p = inputs(
            mon,
            ScreenRect::from_xy_size(150, 100, 360, 420),
            DrawState {
                palette_open: true,
                ..DrawState::default()
            },
        );
        h.tick(&p, None, false);

        let screen = Rect::from_min_size(Pos2::ZERO, vec2(180.0, 160.0));
        let bar = h.area(DRAWBAR_ID).expect("a strip");
        let popup = h.area(PALETTE_ID).expect("an open palette");
        assert!(screen.expand(0.25).contains_rect(bar), "{bar:?} off {screen:?}");
        assert!(screen.expand(0.25).contains_rect(popup), "{popup:?} off {screen:?}");
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(PALETTE_ID));
        let transform = h
            .ctx
            .layer_transform_to_global(layer)
            .expect("the palette was scaled");
        let swatches = h.tiles_in(popup, SWATCH * transform.scaling);
        assert_eq!(swatches.len(), PALETTE.len());
        for (i, swatch) in swatches.iter().enumerate() {
            assert_eq!(
                h.tick(&p, Some(swatch.center()), true)
                    .clicked,
                Some(Command::DrawColor(i))
            );
        }
    }
}
