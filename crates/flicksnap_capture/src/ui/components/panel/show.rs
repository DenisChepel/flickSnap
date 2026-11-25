//! The tray as one egui `Area`: measure the strip, place it beside the
//! selection, then lay it out along the chosen axis.
//!
//! One chassis for every set ([`StripMetrics`] / `single_strip`): the
//! emblem, the body (if the set has one) and the button groups along the
//! axis, flush inside a group, one `GAP` between groups and a hairline
//! rule before a group that asks for one. The capture strip is that run
//! with no body and every button an icon-only tile on the bare chassis,
//! the ways out ruled off at its end.
//!
//! A button hovered for 350 ms grows a tooltip chip saying what it does
//! — as the C# strips do — hung off the strip's far edge (below a row,
//! right of a column) and flipped to the other side when the monitor has
//! no room there. Once a chip is up, the next button's chip follows the
//! pointer without the wait, across the gap between groups too, until the
//! pointer has been off every button for the same 350 ms. The `below`
//! style is the one that raises none: see [`Presentation::wants_tip`].
//!
//! The strip's size is analytic — computed from the same numbers the
//! widgets are laid out with — because placement needs it before the Area
//! is positioned, and because each strip must be centred with its own
//! width for the set-swap guard's premise to hold. A test pins the
//! analytic size against what egui actually lays out at every DPI.

use egui::{pos2, vec2, Rect, Vec2};

use super::assets;
use super::model::{Body, ButtonDef, ButtonStyle, GroupTone, PanelButtonSet, PanelFeatures};
use super::place::{self, Axis, Fit, Footprint, Near};
use super::theme::{self, tokens};
use super::widgets;
use crate::ui::egui_host::{PanelInputs, PanelOutcome};
use crate::ui::shared::UiMonitor;
use flicksnap_shared::geometry::RectExt;

/// The Area id. One tray per context, so one id per context.
pub const PANEL_ID: &str = "flicksnap-tray";
/// The tooltip chip's Area id, and the key its hover clock is kept under.
pub const TIP_ID: &str = "flicksnap-tray-tip";

/// The tooltip's clock, in egui input time, kept in the context's temp
/// data so it survives between passes: which button the pointer is on and
/// since when, and when a chip was last on screen. The last-shown mark is
/// what makes the chip follow the pointer between buttons without a fresh
/// wait — and outlives the pointer leaving the strip by the grace period.
#[derive(Clone, Copy, Default)]
struct TipClock {
    button: Option<egui::Id>,
    since: f64,
    shown_at: Option<f64>,
}

/// The tooltip chip's rect for a button at `button` inside a strip at
/// `tray`, in points, kept inside `screen`. Below a row / right of a
/// column, flipped to the near side when the far side has no room; then
/// slid along the strip so it stays on the monitor.
fn tip_rect(axis: Axis, tray: Rect, button: Rect, chip: Vec2, screen: Rect) -> Rect {
    let rect = match axis {
        Axis::Row => {
            let top = tray.bottom() + tokens::TIP_GAP_BOTTOM;
            let top = if top + chip.y > screen.bottom() {
                tray.top() - tokens::TIP_GAP_BOTTOM - chip.y
            } else {
                top
            };
            Rect::from_min_size(pos2(button.center().x - chip.x / 2.0, top), chip)
        }
        Axis::Column => {
            let left = tray.right() + tokens::TIP_GAP_RIGHT;
            let left = if left + chip.x > screen.right() {
                tray.left() - tokens::TIP_GAP_RIGHT - chip.x
            } else {
                left
            };
            Rect::from_min_size(pos2(left, button.center().y - chip.y / 2.0), chip)
        }
    };
    let dx = (screen.left() - rect.left()).max(0.0) - (rect.right() - screen.right()).max(0.0);
    let dy = (screen.top() - rect.top()).max(0.0) - (rect.bottom() - screen.bottom()).max(0.0);
    rect.translate(vec2(dx, dy))
}

/// Run the hover clock for this pass and paint the chip once it has
/// waited long enough. `hovered` is the button under the pointer (its id,
/// rect and def), `None` when the pointer is on no button.
///
/// A new button restarts the wait unless a chip was on screen within the
/// last `TIP_DELAY_SECS`: then it shows at once, so a pointer sliding
/// along the strip — through the gap between two groups included — keeps
/// its chip. Only a pause off every button as long as the wait itself
/// brings the wait back.
fn show_tip(ctx: &egui::Context, axis: Axis, tray: Rect, hovered: Option<(egui::Id, Rect, &'static ButtonDef)>, visible: bool) {
    let key = egui::Id::unique(TIP_ID);
    let now = ctx.input(|i| i.time);
    let mut clock = ctx.data(|d| {
        d.get_temp::<TipClock>(key)
            .unwrap_or_default()
    });
    let Some((button, rect, def)) = hovered else {
        clock.button = None;
        ctx.data_mut(|d| d.insert_temp(key, clock));
        return;
    };
    if clock.button != Some(button) {
        clock.button = Some(button);
        let in_grace = clock
            .shown_at
            .is_some_and(|t| now - t <= tokens::TIP_DELAY_SECS);
        clock.since = if in_grace { now - tokens::TIP_DELAY_SECS } else { now };
    }
    let waited = now - clock.since;
    if waited < tokens::TIP_DELAY_SECS {
        ctx.data_mut(|d| d.insert_temp(key, clock));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(tokens::TIP_DELAY_SECS - waited));
        return;
    }
    clock.shown_at = Some(now);
    ctx.data_mut(|d| d.insert_temp(key, clock));
    let text = ctx.fonts_mut(|f| f.layout_job(widgets::tip_job(def)));
    // Whole points, so the chip's edges land where egui pins the Area.
    let chip = (text.size() + 2.0 * vec2(tokens::TIP_PAD_H, tokens::TIP_PAD_V)).ceil();
    let chip_rect = tip_rect(axis, tray, rect, chip, ctx.viewport_rect());
    egui::Area::new(key)
        .order(egui::Order::Tooltip)
        .fixed_pos(chip_rect.min)
        .constrain(false)
        .fade_in(false)
        .interactable(false)
        .show(ctx, |ui| {
            if !visible {
                ui.set_opacity(0.0);
            }
            let (r, _) = ui.allocate_exact_size(chip, egui::Sense::hover());
            widgets::tip(ui.painter(), r, def);
        });
}

/// One visible button as measured: table index, def, and its length
/// along a row.
pub type MeasuredButton = (usize, &'static ButtonDef, f32);

/// The strip's body as measured: which one it is, and — for the
/// instruction — the wrap width it was laid out at, so the pass that
/// paints it uses the width the pass that sized the tray measured.
#[derive(Clone, Copy, PartialEq)]
pub enum MeasuredBody {
    None,
    Readout(usize),
    Hint { text: &'static str, wrap: f32 },
}

/// The narrowest wrap width, in points, that fits `text` in
/// `tokens::HINT_LINES` lines, and the line count it achieved.
///
/// Narrowest is also best-balanced: at the width where the last line is
/// about to spill into a third, the two lines are as near equal as the
/// break points allow. The search starts from the width a perfectly even
/// split would need (half the unwrapped run) and steps up, so it costs a
/// handful of layouts of one short paragraph, once per measured pass.
///
/// A text that will not fit in `HINT_LINES` lines at any width up to its
/// full run returns that full width: one line, never a clipped box.
fn hint_wrap(ctx: &egui::Context, text: &'static str) -> (f32, usize) {
    let full = ctx
        .fonts_mut(|f| f.layout_job(widgets::hint_job(text, f32::INFINITY)))
        .size()
        .x;
    let lines = |wrap: f32| {
        ctx.fonts_mut(|f| f.layout_job(widgets::hint_job(text, wrap)))
            .rows
            .len()
    };
    let start = (full / tokens::HINT_LINES as f32).floor();
    let mut wrap = start;
    while wrap < full {
        let rows = lines(wrap);
        if rows <= tokens::HINT_LINES {
            return (wrap, rows);
        }
        wrap += tokens::HINT_WRAP_STEP;
    }
    (full, lines(full))
}

/// One visible group as measured: its tone, whether a rule is drawn
/// before it, and its buttons.
pub struct MeasuredGroup {
    pub tone: GroupTone,
    /// Already false for the first visible group: a rule with nothing
    /// before it is never drawn.
    pub ruled: bool,
    pub buttons: Vec<MeasuredButton>,
}

/// Every number the strip is laid out with, in points, measured before the
/// Area is positioned.
pub struct StripMetrics {
    /// Item thickness across the strip: the button's, and the readout's
    /// square side.
    pub thick: f32,
    /// How every button on this strip is drawn.
    pub kind: Presentation,
    /// What sits between the emblem and the buttons, measured: nothing,
    /// the readout, or the instruction with the wrap width it was
    /// measured at.
    pub body: MeasuredBody,
    /// The body slot's length along a row; 0 with no body.
    pub body_along: f32,
    /// The visible groups in strip order.
    pub groups: Vec<MeasuredGroup>,
}

impl StripMetrics {
    /// Measure one set under one feature switch set. Only valid inside a
    /// run: `Context::fonts_mut` panics before the first pass.
    pub fn measure(ctx: &egui::Context, set: PanelButtonSet, features: PanelFeatures, style: ButtonStyle, word_count: usize) -> Self {
        let kind = Presentation::of(set, style);
        let body = match set.body() {
            Body::None => MeasuredBody::None,
            Body::Readout => MeasuredBody::Readout(word_count),
            Body::Hint(text) => {
                let (wrap, _) = hint_wrap(ctx, text);
                MeasuredBody::Hint {
                    text,
                    wrap,
                }
            }
        };
        let thick = match kind {
            Presentation::Key | Presentation::Icon => tokens::KEY_TILE,
            Presentation::Below => tokens::BELOW_HEIGHT,
        };
        let mut groups: Vec<MeasuredGroup> = Vec::new();
        for g in set.visible_groups(features) {
            let buttons = g
                .members
                .into_iter()
                .map(|(i, def)| {
                    let along = match kind {
                        Presentation::Key | Presentation::Icon => tokens::KEY_TILE,
                        Presentation::Below => {
                            let label = ctx.fonts_mut(|f| f.layout_job(widgets::underlined_label(def)));
                            tokens::BUTTON_MIN_LENGTH.max(label.size().x + 2.0 * tokens::BELOW_PAD_H)
                        }
                    };
                    (i, def, along)
                })
                .collect();
            groups.push(MeasuredGroup {
                tone: g.tone,
                ruled: g.ruled && !groups.is_empty(),
                buttons,
            });
        }
        let body_along = match body {
            MeasuredBody::None => 0.0,
            MeasuredBody::Readout(word_count) => {
                // Reserve five tabular digits so the tray stays put as the count changes.
                let galley = ctx.fonts_mut(|f| f.layout_job(widgets::readout_job(word_count.max(99_999))));
                thick.max(galley.size().x + 2.0 * tokens::READOUT_PAD_H)
            }
            MeasuredBody::Hint {
                text,
                wrap,
            } => {
                let galley = ctx.fonts_mut(|f| f.layout_job(widgets::hint_job(text, wrap)));
                thick.max(galley.size().x + 2.0 * tokens::HINT_PAD_H)
            }
        };
        Self {
            thick,
            kind,
            body,
            body_along,
            groups,
        }
    }

    /// Every visible button, in strip order, with its length along a row.
    pub fn buttons(&self) -> impl Iterator<Item = &MeasuredButton> {
        self.groups
            .iter()
            .flat_map(|g| g.buttons.iter())
    }

    /// One group's length along `axis`: its buttons flush, no gaps.
    fn group_along(&self, axis: Axis, buttons: &[MeasuredButton]) -> f32 {
        match axis {
            Axis::Row => buttons
                .iter()
                .map(|(_, _, along)| *along)
                .sum(),
            Axis::Column => buttons.len() as f32 * self.thick,
        }
    }

    /// The strip's content size (the tray's padding not included) for one
    /// axis. A column is `col_inner` wide throughout, so a set swap or a
    /// feature change can never change its width.
    pub fn content_size(&self, axis: Axis, col_inner: f32) -> Vec2 {
        // The items after the emblem, one `GAP` between each pair: the
        // body (if any), every group, and a rule before each ruled group.
        let mut items: Vec<f32> = Vec::new();
        if !matches!(self.body, MeasuredBody::None) {
            items.push(match axis {
                Axis::Row => self.body_along,
                Axis::Column => self.thick,
            });
        }
        for g in &self.groups {
            if g.ruled {
                items.push(tokens::DIVIDER_BLOCK);
            }
            items.push(self.group_along(axis, &g.buttons));
        }
        let gaps = items.len().saturating_sub(1) as f32 * tokens::GAP;
        let along = tokens::EMBLEM_SLOT + tokens::EMBLEM_GAP + items.iter().sum::<f32>() + gaps;
        match axis {
            Axis::Row => vec2(along, self.thick),
            Axis::Column => vec2(col_inner, along),
        }
    }

    /// The width this strip wants when it stands on its end: the widest
    /// thing it has to hold. Its caller measures with every switch on, so
    /// flipping a feature can never move the tray's edge.
    pub fn col_inner(&self) -> f32 {
        let mut w = tokens::EMBLEM_SLOT.max(self.body_along);
        for (_, _, along) in self.buttons() {
            w = w.max(*along);
        }
        w
    }
}

/// The strip box in physical pixels: the content plus the tray's padding
/// on either side, rounded once per axis.
fn outer_px(content: Vec2, ppp: f32) -> (i32, i32) {
    let pad = 2.0 * tokens::PAD;
    (((content.x + pad) * ppp).round() as i32, ((content.y + pad) * ppp).round() as i32)
}

fn footprint(m: &StripMetrics, col_inner: f32, ppp: f32) -> Fit {
    let (row_w, row_h) = outer_px(m.content_size(Axis::Row, col_inner), ppp);
    let (col_w, col_h) = outer_px(m.content_size(Axis::Column, col_inner), ppp);
    Fit {
        row: Footprint {
            len: row_w,
            thick: row_h,
        },
        col: Footprint {
            len: col_h,
            thick: col_w,
        },
    }
}

/// The width one set draws at when the tray stands on its end: its own,
/// measured with every switch on, so flipping a feature can never move
/// the tray's edge. Two sets may want different widths — the capture
/// strip's grid is twice a tile wide and the OCR strip is one — the same
/// way they have always drawn rows of different lengths.
fn col_inner_for(ctx: &egui::Context, p: &PanelInputs, set: PanelButtonSet) -> f32 {
    StripMetrics::measure(ctx, set, PanelFeatures::ALL, p.style, p.word_count).col_inner()
}

/// The longest box either set can become in each orientation with every
/// feature on, so the side choice depends on neither which set is up nor
/// which switches are on.
fn union_fit(ctx: &egui::Context, p: &PanelInputs, ppp: f32) -> Fit {
    let mut fit = Fit {
        row: Footprint {
            len: 0,
            thick: 0,
        },
        col: Footprint {
            len: 0,
            thick: 0,
        },
    };
    for &set in PanelButtonSet::UNION {
        let m = StripMetrics::measure(ctx, set, PanelFeatures::ALL, p.style, p.word_count);
        let measured = footprint(&m, m.col_inner(), ppp);
        fit.row.len = fit.row.len.max(measured.row.len);
        fit.row.thick = fit.row.thick.max(measured.row.thick);
        fit.col.len = fit.col.len.max(measured.col.len);
        fit.col.thick = fit.col.thick.max(measured.col.thick);
    }
    fit
}

/// Measure the strip that is up and place its box, in physical pixels —
/// everything `show` needs before the Area exists, and the whole of what
/// the placement tests check.
fn measure_and_place(
    ctx: &egui::Context,
    p: &PanelInputs,
    monitor: UiMonitor,
) -> (StripMetrics, f32, place::Side, flicksnap_shared::geometry::ScreenRect) {
    let ppp = monitor.dpi_scale.max(0.1);
    let lock = p.set.axis_lock();
    let col_inner = col_inner_for(ctx, p, p.set);
    let m = StripMetrics::measure(ctx, p.set, p.features, p.style, p.word_count);
    let fit = if PanelButtonSet::UNION.contains(&p.set) {
        let reserved = union_fit(ctx, p, ppp);
        // Keep the orientation stable across strip swaps when possible. On a
        // small monitor, a strip with fewer buttons might still fit on its own.
        if [Axis::Row, Axis::Column]
            .into_iter()
            .any(|axis| lock.is_none_or(|only| only == axis) && reserved.fits_on(axis, monitor.bounds))
        {
            reserved
        } else {
            footprint(&m, col_inner, ppp)
        }
    } else {
        footprint(&m, col_inner, ppp)
    };
    let (side, rect_px) = place::place(p.anchor, monitor.bounds, fit, Near::at_dpi(ppp), lock, |axis| {
        outer_px(m.content_size(axis, col_inner), ppp)
    });
    (m, col_inner, side, rect_px)
}

/// How a button says what it is. One per strip: the capture strip is
/// always `Icon`; every other set takes it from [`ButtonStyle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// Icon and accelerator letter in a square: needs a hover chip to say
    /// what it does.
    Key,
    /// Icon over the label.
    Below,
    /// The icon alone in a square, key-sized.
    Icon,
}

impl Presentation {
    fn of(set: PanelButtonSet, style: ButtonStyle) -> Self {
        if set.icons_only() {
            return Self::Icon;
        }
        match style {
            ButtonStyle::KeyHint => Self::Key,
            ButtonStyle::Below => Self::Below,
        }
    }

    /// Whether a button drawn this way needs a hover chip to say what it
    /// does. `below` is the exception: it is a whole strip of labels
    /// under icons, and a chip over every one of them was noise.
    fn wants_tip(self) -> bool {
        !matches!(self, Self::Below)
    }

    /// The padding this presentation carries on either side of its
    /// contents.
    fn pad_h(self) -> f32 {
        match self {
            Self::Key | Self::Icon => tokens::KEY_PAD_H,
            Self::Below => tokens::BELOW_PAD_H,
        }
    }
}

/// Everything about how one run of buttons is drawn, apart from the
/// buttons themselves: which presentation, where its contents sit, and
/// the group colours the hover veil works over.
#[derive(Clone, Copy)]
struct ButtonSkin {
    kind: Presentation,
    align: egui::Align2,
    pad_h: f32,
    base: egui::Color32,
    veil: f32,
}

impl ButtonSkin {
    fn new(kind: Presentation, tone: GroupTone, accent: egui::Color32) -> Self {
        Self {
            kind,
            align: egui::Align2::CENTER_CENTER,
            pad_h: kind.pad_h(),
            base: theme::group_fill(tone, accent),
            veil: theme::hover_veil(tone),
        }
    }
}

/// Draw one button at `size`, fold what the pointer did with it into
/// `out`, and hand back the box it took. A button is registered as the
/// hovered one only if its presentation wants a chip
/// ([`Presentation::wants_tip`]).
fn button(
    ui: &mut egui::Ui,
    p: &PanelInputs,
    measured: &MeasuredButton,
    size: Vec2,
    skin: ButtonSkin,
    out: &mut PanelOutcome,
    hovered: &mut Option<(egui::Id, Rect, &'static ButtonDef)>,
) -> Rect {
    let (table_idx, def, _) = *measured;
    // The set is part of the id, so hover state and hit tests can never
    // resolve against another strip; the table index keeps a switched-off
    // button from renumbering its neighbours.
    let id = egui::Id::unique(("panel", p.set as u8, table_idx));
    let mut widget = match skin.kind {
        Presentation::Key => widgets::key_hint_button(def, id, size, skin.base, skin.veil),
        Presentation::Below => widgets::below_button(def, id, size, skin.base, skin.veil),
        Presentation::Icon => widgets::icon_button(def, id, size, skin.base, skin.veil),
    };
    widget.align = skin.align;
    widget.pad_h = skin.pad_h;
    let r = widget.show(ui);
    out.over_button |= r.contains_pointer();
    if r.contains_pointer() && skin.kind.wants_tip() {
        *hovered = Some((id, r.rect, def));
    }
    if r.clicked() {
        out.clicked = Some(def.command);
    }
    r.rect
}

/// The strip: the emblem, the body and the button groups along `axis`,
/// flush inside each group, one `GAP` between them and a rule before a
/// ruled group.
fn single_strip(
    ui: &mut egui::Ui,
    m: &StripMetrics,
    p: &PanelInputs,
    axis: Axis,
    col_inner: f32,
    out: &mut PanelOutcome,
    hovered: &mut Option<(egui::Id, Rect, &'static ButtonDef)>,
) {
    let kind = m.kind;
    let strip = |ui: &mut egui::Ui| {
        let across = match axis {
            Axis::Row => m.thick,
            Axis::Column => col_inner,
        };
        let emblem_slot = match axis {
            Axis::Row => vec2(tokens::EMBLEM_SLOT, across),
            Axis::Column => vec2(across, tokens::EMBLEM_SLOT),
        };
        // The emblem's slot is padded around a smaller mark, so it needs
        // less of a gap than the flush button boxes. egui spends
        // `item_spacing` AFTER a widget, so this is set before the emblem
        // and restored before the readout.
        ui.spacing_mut().item_spacing = Vec2::splat(tokens::EMBLEM_GAP);
        widgets::emblem(ui, emblem_slot);
        ui.spacing_mut().item_spacing = Vec2::splat(tokens::GAP);
        let body_slot = match axis {
            Axis::Row => vec2(m.body_along, across),
            Axis::Column => vec2(across, m.thick),
        };
        match m.body {
            MeasuredBody::None => {}
            MeasuredBody::Readout(readout) => {
                widgets::readout(ui, readout, body_slot);
            }
            MeasuredBody::Hint {
                text,
                wrap,
            } => {
                widgets::hint(ui, text, wrap, body_slot);
            }
        }
        for g in &m.groups {
            if g.ruled {
                widgets::divider(ui, axis == Axis::Row, across);
            }
            let buttons = &g.buttons;
            let skin = ButtonSkin::new(kind, g.tone, p.accent);
            let group = |ui: &mut egui::Ui| {
                // Flush inside the group; the tray's gap is between
                // groups only.
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for measured in buttons {
                    let size = match axis {
                        Axis::Row => vec2(measured.2, across),
                        Axis::Column => vec2(across, m.thick),
                    };
                    button(ui, p, measured, size, skin, out, hovered);
                }
            };
            // The frame's content ui inherits the strip's layout, so the
            // buttons run along the same axis; a nested
            // `horizontal`/`vertical` here would pad the cross axis with
            // its own initial size.
            theme::group_frame(skin.base).show(ui, group);
        }
    };
    match axis {
        Axis::Row => {
            ui.horizontal(strip);
        }
        Axis::Column => {
            ui.vertical(strip);
        }
    }
}

/// Build the tray for one monitor and report what the pointer found.
/// Called inside a host's run closure, so text measurement is legal here.
///
/// `visible` is the Q toggle. A hidden tray is still laid out, still
/// hit-tested and still clicked — that is the overlay's long-standing
/// behaviour — it simply paints nothing: at opacity 0 every shape the
/// painter is handed becomes a `Shape::Noop`, while the widget rects and
/// the hover animation carry on as they are.
pub fn show(ctx: &egui::Context, p: &PanelInputs, monitor: UiMonitor, visible: bool) -> PanelOutcome {
    // Ahead of any measurement: a mark that cannot be rasterised is logged
    // here rather than drawn as egui's placeholder glyph.
    assets::preload(ctx);
    let ppp = monitor.dpi_scale.max(0.1);
    let (m, col_inner, side, rect_px) = measure_and_place(ctx, p, monitor);
    let axis = side.axis();
    let local = egui::pos2(
        (rect_px.left() - monitor.bounds.left()) as f32 / ppp,
        (rect_px.top() - monitor.bounds.top()) as f32 / ppp,
    );

    let mut out = PanelOutcome::default();
    let mut hovered: Option<(egui::Id, Rect, &'static ButtonDef)> = None;
    let area = egui::Area::new(egui::Id::unique(PANEL_ID))
        .order(egui::Order::Foreground)
        .fixed_pos(local)
        // Never `constrain`: egui's symmetric clamp would override the
        // tail-wins rule `place` just applied.
        .constrain(false)
        .fade_in(false)
        .interactable(true)
        // The dead chassis swallows clicks, as the C# strips do.
        .sense(egui::Sense::CLICK);
    let inner = area.show(ctx, |ui| {
        if !visible {
            ui.set_opacity(0.0);
        }
        theme::tray_frame().show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(tokens::GAP);
            single_strip(ui, &m, p, axis, col_inner, &mut out, &mut hovered);
        });
    });
    // The whole tray: emblem, readout, padding and gaps included.
    out.over_tray = inner.response.contains_pointer();
    // Only a presentation that wants a chip registers a hovered button,
    // so a `below` strip never raises one.
    show_tip(ctx, axis, inner.response.rect, hovered, visible);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::command::Command;
    use crate::ui::components::panel::model::FEATURE_COMBINATIONS;
    use egui::{Pos2, Rect};
    use flicksnap_shared::geometry::ScreenRect;

    const DPIS: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

    fn rect(x: i32, y: i32, w: i32, h: i32) -> ScreenRect {
        ScreenRect::from_xy_size(x, y, w, h)
    }

    fn monitor(bounds: ScreenRect, dpi: f32) -> UiMonitor {
        UiMonitor {
            bounds,
            dpi_scale: dpi,
            is_primary: true,
        }
    }

    const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x2F, 0x7C, 0xAE);

    fn inputs(set: PanelButtonSet, features: PanelFeatures, style: ButtonStyle, selection: ScreenRect, monitor: UiMonitor) -> PanelInputs {
        PanelInputs {
            set,
            features,
            style,
            accent: ACCENT,
            word_count: 1234,
            anchor: crate::selection::intersect_rects(monitor.bounds, selection).expect("the selection overlaps the monitor"),
        }
    }

    /// A styled, fonted context that runs the real panel. Real Inter
    /// faces are mandatory: `FontDefinitions::empty()` lays out zero-width
    /// glyphs and every size assertion would be wrong.
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

        fn raw_input(&self, pointer: Option<Pos2>, extra: &[egui::Event]) -> egui::RawInput {
            let ppp = self.monitor.dpi_scale.max(0.1);
            let b = self.monitor.bounds;
            let mut events = Vec::with_capacity(extra.len() + 1);
            events.push(match pointer {
                Some(p) => egui::Event::PointerMoved(p),
                None => egui::Event::PointerGone,
            });
            events.extend_from_slice(extra);
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(b.width() as f32, b.height() as f32) / ppp,
                )),
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

        /// A fixed three passes — enough for the Area's rects to settle,
        /// and for a press to land on the second the way the host feeds
        /// it — with every `FullOutput` dropped without applying its
        /// deltas (epaint debug-asserts a non-empty `TexturesDelta` on
        /// drop). Unlike `MonitorHost::run` this reports the pointer on
        /// every pass: the host gates that on movement so it can tell
        /// when it is owed another run, and a fixed-length loop is not.
        fn run(&mut self, p: &PanelInputs, pointer: Option<Pos2>) -> PanelOutcome {
            self.tick(p, pointer, false)
        }

        /// One tick with the press-and-release pair fed from the second
        /// pass on, exactly as the host does it, and `clicked` ORed over
        /// the passes.
        fn press(&mut self, p: &PanelInputs, pointer: Pos2) -> PanelOutcome {
            self.tick(p, Some(pointer), true)
        }

        fn tick(&mut self, p: &PanelInputs, pointer: Option<Pos2>, press: bool) -> PanelOutcome {
            let press_events: Vec<egui::Event> = match pointer.filter(|_| press) {
                Some(pos) => [true, false]
                    .into_iter()
                    .map(|pressed| egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    })
                    .collect(),
                None => Vec::new(),
            };
            let mut acc = PanelOutcome::default();
            for pass in 0..3 {
                let extra: &[egui::Event] = if press && pass == 1 { &press_events } else { &[] };
                let raw = self.raw_input(pointer, extra);
                let monitor = self.monitor;
                let mut out = PanelOutcome::default();
                let full = self.ctx.run_ui(raw, |ui| {
                    out = show(ui.ctx(), p, monitor, true);
                });
                full.drop_without_applying_deltas();
                acc = PanelOutcome {
                    clicked: acc.clicked.or(out.clicked),
                    over_button: out.over_button,
                    over_tray: out.over_tray,
                };
            }
            acc
        }

        fn area_rect(&self) -> Rect {
            self.ctx
                .memory(|m| m.area_rect(egui::Id::unique(PANEL_ID)))
                .expect("the tray has an area rect after a run")
        }

        fn interactive_rects(&self) -> Vec<Rect> {
            self.ctx.interactive_rects_last_pass()
        }

        /// The analytic placement, computed inside a pass so text
        /// measurement is legal. Call after at least one `run`.
        fn analytic(&mut self, p: &PanelInputs) -> (place::Side, ScreenRect) {
            let monitor = self.monitor;
            let mut got = None;
            let raw = self.raw_input(None, &[]);
            let full = self.ctx.run_ui(raw, |ui| {
                let (_, _, side, rect) = measure_and_place(ui.ctx(), p, monitor);
                got = Some((side, rect));
            });
            full.drop_without_applying_deltas();
            got.expect("the closure ran")
        }
    }

    const HD: (i32, i32) = (1920, 1080);

    fn hd(dpi: f32) -> UiMonitor {
        monitor(rect(0, 0, HD.0, HD.1), dpi)
    }

    /// A selection comfortably inset, so the row goes beneath it.
    fn row_selection() -> ScreenRect {
        rect(500, 300, 600, 400)
    }

    #[test]
    fn fullscreen_capture_uses_a_column_when_the_row_would_clip() {
        let no_upload = PanelFeatures {
            upload: false,
            ..PanelFeatures::ALL
        };
        for (w, h, features) in [(900, 1600, PanelFeatures::ALL), (600, 950, no_upload)] {
            let bounds = rect(0, 0, w, h);
            let mon = monitor(bounds, 2.0);
            let mut harness = Harness::new(mon);
            let p = inputs(PanelButtonSet::Normal, features, ButtonStyle::KeyHint, bounds, mon);
            harness.run(&p, None);

            let (side, _) = harness.analytic(&p);
            assert_eq!(side, place::Side::Inside(Axis::Column), "{w}x{h}");
            let screen = Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32) / mon.dpi_scale);
            let tray = harness.area_rect();
            assert!(screen.contains_rect(tray), "tray {tray:?} outside {screen:?}");
        }
    }

    #[test]
    fn button_count_matches_visible_defs() {
        for &style in &[ButtonStyle::KeyHint, ButtonStyle::Below] {
            for &set in PanelButtonSet::ALL {
                for features in FEATURE_COMBINATIONS {
                    let mon = hd(1.0);
                    let mut h = Harness::new(mon);
                    let p = inputs(set, features, style, row_selection(), mon);
                    h.run(&p, None);
                    let expected = set.visible_defs(features).count();
                    // One extra: the tray body itself senses clicks.
                    assert_eq!(h.interactive_rects().len(), expected + 1, "{set:?} {style:?} {features:?}");
                }
            }
        }
    }

    /// The OCR strip's readout is the word count over a "words" caption,
    /// and the caption is what sets the slot's width, so the count's
    /// digits never change the strip.
    #[test]
    fn ocr_readout_shows_words_and_the_caption_sets_the_width() {
        let job = widgets::readout_job(1234);
        assert_eq!(job.text, "1234\nwords");

        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let mut widths = Vec::new();
        for n in [0, 7, 1234, 99_999] {
            let p = PanelInputs {
                word_count: n,
                ..inputs(PanelButtonSet::Ocr, PanelFeatures::ALL, ButtonStyle::KeyHint, row_selection(), mon)
            };
            h.run(&p, None);
            widths.push(h.area_rect().width());
        }
        assert!(
            widths
                .iter()
                .all(|w| (w - widths[0]).abs() <= 0.01),
            "{widths:?}"
        );
    }

    /// The chip hangs off the strip's far edge, centred on the button,
    /// and flips to the near edge when the far one is off the monitor.
    #[test]
    fn tip_rect_hangs_off_the_far_edge_and_flips_when_out_of_room() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0));
        let chip = vec2(50.0, 20.0);
        let tray = Rect::from_min_size(pos2(600.0, 700.0), vec2(500.0, 48.0));
        let button = Rect::from_min_size(pos2(700.0, 704.0), vec2(40.0, 40.0));
        let below = tip_rect(Axis::Row, tray, button, chip, screen);
        assert_eq!(below.top(), tray.bottom() + tokens::TIP_GAP_BOTTOM);
        assert_eq!(below.center().x, button.center().x);

        let low_tray = Rect::from_min_size(pos2(600.0, 1030.0), vec2(500.0, 48.0));
        let low_button = Rect::from_min_size(pos2(700.0, 1034.0), vec2(40.0, 40.0));
        let above = tip_rect(Axis::Row, low_tray, low_button, chip, screen);
        assert_eq!(above.bottom(), low_tray.top() - tokens::TIP_GAP_BOTTOM);

        let col = Rect::from_min_size(pos2(1000.0, 100.0), vec2(48.0, 500.0));
        let col_button = Rect::from_min_size(pos2(1004.0, 200.0), vec2(40.0, 40.0));
        let right = tip_rect(Axis::Column, col, col_button, chip, screen);
        assert_eq!(right.left(), col.right() + tokens::TIP_GAP_RIGHT);
        assert_eq!(right.center().y, col_button.center().y);

        let edge_col = Rect::from_min_size(pos2(1870.0, 100.0), vec2(48.0, 500.0));
        let edge_button = Rect::from_min_size(pos2(1874.0, 200.0), vec2(40.0, 40.0));
        let left = tip_rect(Axis::Column, edge_col, edge_button, chip, screen);
        assert_eq!(left.right(), edge_col.left() - tokens::TIP_GAP_RIGHT);

        // Slid back onto the monitor along the strip.
        let corner_button = Rect::from_min_size(pos2(1900.0, 704.0), vec2(40.0, 40.0));
        let slid = tip_rect(Axis::Row, tray, corner_button, chip, screen);
        assert!(slid.right() <= screen.right(), "{slid:?}");
    }

    /// The tooltip is a `key`-style affair only, and it waits: a pointer
    /// resting on a button for a tick shows nothing, one resting for the
    /// delay shows the chip, and moving off the button removes it at
    /// once. Once a chip has been up, the next button gets its chip
    /// straight away — even after a short hop across no button — until
    /// the pointer has been off the buttons for the whole delay. The
    /// harness feeds no clock, so egui steps time by its predicted frame
    /// (1/60 s) per pass.
    #[test]
    fn key_hint_tooltip_appears_after_the_delay_and_names_the_button() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            row_selection(),
            mon,
        );
        h.run(&p, None);
        let first = h
            .interactive_rects()
            .into_iter()
            .filter(|r| r.width() <= tokens::KEY_TILE + 0.5)
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .expect("a button rect");
        let tip_layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique(TIP_ID));
        let tip_shown = |h: &Harness| {
            h.ctx
                .memory(|m| m.areas().visible_last_frame(&tip_layer))
        };

        h.run(&p, Some(first.center()));
        assert!(!tip_shown(&h), "the chip must wait out the delay");
        let passes = (tokens::TIP_DELAY_SECS * 60.0).ceil() as usize + 3;
        for _ in 0..passes / 3 {
            h.run(&p, Some(first.center()));
        }
        assert!(tip_shown(&h), "the chip is up after {passes} passes");
        let tip = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(TIP_ID)))
            .expect("the chip has a rect");
        let tray = h.area_rect();
        assert!(
            (tip.top() - (tray.bottom() + tokens::TIP_GAP_BOTTOM)).abs() < 0.01,
            "{tip:?} vs {tray:?}"
        );
        // Centred to the pixel the Area was pinned on.
        assert!((tip.center().x - first.center().x).abs() <= 0.5, "{tip:?} vs {first:?}");

        h.run(&p, None);
        assert!(!tip_shown(&h), "the chip goes with the pointer");

        // Within the grace period, the next button shows at once — one
        // tick off every button (a gap between groups) does not reset it.
        let second = h
            .interactive_rects()
            .into_iter()
            .filter(|r| r.width() <= tokens::KEY_TILE + 0.5 && r.left() > first.left() + 1.0)
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .expect("a second button rect");
        h.run(&p, Some(second.center()));
        assert!(tip_shown(&h), "the chip follows the pointer within the grace period");
        let tip = h
            .ctx
            .memory(|m| m.area_rect(egui::Id::unique(TIP_ID)))
            .expect("the chip has a rect");
        assert!((tip.center().x - second.center().x).abs() <= 0.5, "{tip:?} vs {second:?}");

        // Off every button for the whole delay: the wait is back.
        for _ in 0..passes / 3 {
            h.run(&p, None);
        }
        h.run(&p, Some(first.center()));
        assert!(!tip_shown(&h), "a long pause brings the wait back");

        // `below` has labels and never shows a chip.
        let below = inputs(PanelButtonSet::Ocr, PanelFeatures::ALL, ButtonStyle::Below, row_selection(), mon);
        let mut h = Harness::new(mon);
        h.run(&below, None);
        let first = h
            .interactive_rects()
            .into_iter()
            .filter(|r| r.height() <= tokens::BELOW_HEIGHT + 0.5 && r.width() < 200.0)
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .expect("a button rect");
        for _ in 0..passes / 3 {
            h.run(&below, Some(first.center()));
        }
        assert!(!tip_shown(&h));
    }

    #[test]
    fn key_hint_letters_match_accelerators() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            row_selection(),
            mon,
        );
        h.run(&p, None);
        for set in PanelButtonSet::ALL {
            for def in set.defs() {
                let button = widgets::key_hint_button(def, egui::Id::unique(def.label), Vec2::ZERO, ACCENT, tokens::HOVER_VEIL_PRIMARY);
                assert_eq!(
                    button
                        .text
                        .as_ref()
                        .expect("a key tile carries its letter")
                        .text(),
                    def.accel_key()
                        .to_ascii_uppercase()
                        .to_string(),
                    "{}",
                    def.label
                );
            }
        }
    }

    #[test]
    fn below_labels_underline_the_accelerator_byte() {
        for set in PanelButtonSet::ALL {
            for def in set.defs() {
                let job = widgets::underlined_label(def);
                assert_eq!(job.text, def.label);
                assert_eq!(job.sections.len(), 3, "{}", def.label);
                let accel = &job.sections[1];
                assert_eq!(
                    (accel.byte_range.start.0, accel.byte_range.end.0),
                    (def.underline_idx, def.underline_idx + 1),
                    "{}",
                    def.label
                );
                assert!(accel.format.underline.width > 0.0, "{}", def.label);
            }
        }
    }

    /// The single-strip sets keep the one-row chassis the `key` and
    /// `below` styles size: 40 + 2 x 4 and 48 + 2 x 4.
    #[test]
    fn frame_is_48_tall_in_key_style_and_56_in_below_at_100_percent() {
        for (style, height) in [(ButtonStyle::KeyHint, 48.0), (ButtonStyle::Below, 56.0)] {
            let mon = hd(1.0);
            let mut h = Harness::new(mon);
            let p = inputs(PanelButtonSet::Ocr, PanelFeatures::ALL, style, row_selection(), mon);
            h.run(&p, None);
            assert!((h.area_rect().height() - height).abs() <= 0.5, "{style:?}: {:?}", h.area_rect());
        }
    }

    /// The capture strip's buttons sorted along the axis it was laid out
    /// on: left to right in a row, top to bottom in a column.
    fn capture_buttons(h: &Harness, axis: Axis) -> Vec<Rect> {
        let mut rects: Vec<Rect> = h
            .interactive_rects()
            .into_iter()
            .filter(|r| r.width() <= tokens::KEY_TILE + 0.5 && r.height() <= tokens::KEY_TILE + 0.5)
            .collect();
        rects.sort_by(|a, b| match axis {
            Axis::Row => a.left().total_cmp(&b.left()),
            Axis::Column => a.top().total_cmp(&b.top()),
        });
        rects
    }

    /// A selection hugging the bottom of a tall monitor: every union set
    /// answers it with a column.
    fn column_case() -> (UiMonitor, ScreenRect) {
        (monitor(rect(0, 0, 1920, 1300), 1.0), rect(100, 300, 400, 998))
    }

    /// The air the rule takes between the two runs, gaps included.
    const RULED_GAP: f32 = 2.0 * tokens::GAP + tokens::DIVIDER_BLOCK;

    /// The capture strip is one row of identical square icon tiles,
    /// whichever `--panel-buttons` style the run was given: one tile tall
    /// plus the tray's padding.
    #[test]
    fn the_capture_strip_is_one_row_of_square_tiles_in_either_style() {
        let want = tokens::KEY_TILE + 2.0 * tokens::PAD;
        for &style in &[ButtonStyle::KeyHint, ButtonStyle::Below] {
            let mon = hd(1.0);
            let mut h = Harness::new(mon);
            let p = inputs(PanelButtonSet::Normal, PanelFeatures::ALL, style, row_selection(), mon);
            h.run(&p, None);
            let area = h.area_rect();
            assert!((area.height() - want).abs() <= 0.5, "{style:?}: {area:?}");
            let buttons = capture_buttons(&h, Axis::Row);
            assert_eq!(buttons.len(), 11, "{style:?}: {buttons:?}");
            for r in &buttons {
                assert!((r.width() - tokens::KEY_TILE).abs() <= 0.5, "{style:?}: {r:?}");
                assert!((r.height() - tokens::KEY_TILE).abs() <= 0.5, "{style:?}: {r:?}");
                assert!((r.top() - buttons[0].top()).abs() <= 0.5, "{style:?}: one row: {buttons:?}");
            }
        }
    }

    /// The design, as a row: the emblem, then the optional hand-offs and
    /// the core actions flush in one run, a rule, and the two ways out
    /// ending on the tray's right padding. Every button answers with its
    /// own command, in table order.
    #[test]
    fn capture_row_runs_hand_offs_then_core_then_a_rule_and_the_ways_out() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            row_selection(),
            mon,
        );
        h.run(&p, None);
        let area = h.area_rect();
        let buttons = capture_buttons(&h, Axis::Row);
        assert_eq!(h.analytic(&p).0.axis(), Axis::Row);
        assert_eq!(buttons.len(), 11);

        let head = area.left() + tokens::PAD + tokens::EMBLEM_SLOT + tokens::EMBLEM_GAP;
        assert!((buttons[0].left() - head).abs() <= 0.5, "{:?} in {area:?}", buttons[0]);
        assert!(
            (buttons[10].right() - (area.right() - tokens::PAD)).abs() <= 0.5,
            "{:?} in {area:?}",
            buttons[10]
        );
        for (i, pair) in buttons.windows(2).enumerate() {
            let gap = pair[1].left() - pair[0].right();
            let want = if i == 8 { RULED_GAP } else { 0.0 };
            assert!((gap - want).abs() <= 0.5, "between {i} and {}: {gap}, want {want}", i + 1);
        }

        let expected: Vec<Command> = PanelButtonSet::Normal
            .visible_defs(PanelFeatures::ALL)
            .map(|(_, d)| d.command)
            .collect();
        assert_eq!(
            expected,
            vec![
                Command::Upload,
                Command::Share,
                Command::ScrollCapture,
                Command::Ocr,
                Command::SearchImage,
                Command::Edit,
                Command::Video,
                Command::Copy,
                Command::Save,
                Command::Reset,
                Command::Exit,
            ]
        );
        for (r, cmd) in buttons.iter().zip(expected) {
            let out = h.press(&p, r.center());
            assert_eq!(out.clicked, Some(cmd), "{r:?}");
        }
    }

    /// Stood on its end the capture strip is the same run top to bottom:
    /// one tile per line at the column's width, flush, with the rule lying
    /// flat before the ways out.
    #[test]
    fn capture_column_is_the_same_run_top_to_bottom() {
        let (mon, sel) = column_case();
        let mut h = Harness::new(mon);
        let p = inputs(PanelButtonSet::Normal, PanelFeatures::ALL, ButtonStyle::KeyHint, sel, mon);
        h.run(&p, None);
        let area = h.area_rect();
        let buttons = capture_buttons(&h, Axis::Column);
        assert_eq!(h.analytic(&p).0.axis(), Axis::Column);
        assert_eq!(buttons.len(), 11);
        assert!((area.width() - (tokens::KEY_TILE + 2.0 * tokens::PAD)).abs() <= 0.5, "{area:?}");
        for r in &buttons {
            assert!((r.left() - (area.left() + tokens::PAD)).abs() <= 0.5, "{r:?} in {area:?}");
        }
        for (i, pair) in buttons.windows(2).enumerate() {
            let gap = pair[1].top() - pair[0].bottom();
            let want = if i == 8 { RULED_GAP } else { 0.0 };
            assert!((gap - want).abs() <= 0.5, "between {i} and {}: {gap}, want {want}", i + 1);
        }
        let out = h.press(&p, buttons[10].center());
        assert_eq!(out.clicked, Some(Command::Exit), "{out:?}");
    }

    /// Switching optional buttons off only shortens the run from the left:
    /// the core and the ways out keep their place against the right edge,
    /// the run stays flush, and the rule stays before the ways out.
    #[test]
    fn the_capture_strip_survives_every_switch_combination() {
        let mon = hd(1.0);
        for features in FEATURE_COMBINATIONS {
            let mut h = Harness::new(mon);
            let p = inputs(PanelButtonSet::Normal, features, ButtonStyle::KeyHint, row_selection(), mon);
            h.run(&p, None);
            let area = h.area_rect();
            let buttons = capture_buttons(&h, Axis::Row);
            let n = PanelButtonSet::Normal
                .visible_defs(features)
                .count();
            assert_eq!(buttons.len(), n, "{features:?}");
            let last = buttons
                .last()
                .expect("the ways out are always there");
            assert!(
                (last.right() - (area.right() - tokens::PAD)).abs() <= 0.5,
                "{features:?}: {last:?} in {area:?}"
            );
            for (i, pair) in buttons.windows(2).enumerate() {
                let gap = pair[1].left() - pair[0].right();
                let want = if i == n - 3 { RULED_GAP } else { 0.0 };
                assert!((gap - want).abs() <= 0.5, "{features:?}: between {i} and {}: {gap}", i + 1);
            }
        }
    }

    /// The icon tile carries no text at all, and its chip names the key
    /// the tile no longer shows.
    #[test]
    fn icon_tiles_have_no_text_and_their_chip_names_the_key() {
        for def in PanelButtonSet::Normal.defs() {
            let tile = widgets::icon_button(
                def,
                egui::Id::unique(def.label),
                Vec2::ZERO,
                tokens::TRAY_FILL,
                tokens::HOVER_VEIL_BARE,
            );
            assert!(tile.text.is_none(), "{}", def.label);
            let job = widgets::tip_job(def);
            assert!(job.text.starts_with(def.tip), "{}: {:?}", def.label, job.text);
            let key = def
                .accel_key()
                .to_ascii_uppercase()
                .to_string();
            assert!(job.text.ends_with(&key), "{}: {:?}", def.label, job.text);
        }
    }

    #[test]
    fn area_rect_equals_the_analytic_strip() {
        // A row anchor and a column anchor: a selection hugging the bottom
        // of a tall monitor forces the column for either set.
        let cases: [(UiMonitor, ScreenRect); 2] = [
            (hd(1.0), row_selection()),
            (monitor(rect(0, 0, 1920, 1300), 1.0), rect(100, 300, 400, 998)),
        ];
        for &style in &[ButtonStyle::KeyHint, ButtonStyle::Below] {
            for &set in PanelButtonSet::ALL {
                for (mon, sel) in cases {
                    for dpi in DPIS {
                        let mon = monitor(mon.bounds, dpi);
                        let mut h = Harness::new(mon);
                        let p = inputs(set, PanelFeatures::ALL, style, sel, mon);
                        h.run(&p, None);
                        let (_, expected) = h.analytic(&p);
                        let area = h.area_rect();
                        let tol = 1.0 / dpi + 1e-3;
                        assert!(
                            (area.width() - expected.width() as f32 / dpi).abs() <= tol
                                && (area.height() - expected.height() as f32 / dpi).abs() <= tol,
                            "{set:?} {style:?} at {dpi}: {area:?} vs {expected:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn side_follows_the_placement_rules() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            rect(500, 300, 100, 100),
            mon,
        );
        h.run(&p, None);
        assert_eq!(h.analytic(&p).0.axis(), Axis::Row);

        let low = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            rect(500, 1000, 100, 60),
            mon,
        );
        h.run(&low, None);
        assert_eq!(h.analytic(&low).0.axis(), Axis::Column);
    }

    #[test]
    fn swap_recentres_with_the_new_width() {
        for dpi in DPIS {
            let mon = hd(dpi);
            let centres: Vec<f32> = PanelButtonSet::ALL
                .iter()
                .map(|&set| {
                    let mut h = Harness::new(mon);
                    let p = inputs(set, PanelFeatures::ALL, ButtonStyle::KeyHint, row_selection(), mon);
                    h.run(&p, None);
                    h.area_rect().center().x
                })
                .collect();
            assert!((centres[0] - centres[1]).abs() <= 1.0 / dpi, "at {dpi}: {centres:?}");
        }
    }

    /// Pinned over the union sets only: the scroll-picker is not one of
    /// them — it is locked to a row, so it has no column width to agree
    /// about, and its instruction would widen every other set's column if
    /// it were folded in.
    /// The instruction is wrapped by the strip, not by hand: two lines,
    /// near enough the same length that the block reads as a paragraph
    /// rather than a long line with a word hanging off it.
    #[test]
    fn the_scroll_pick_instruction_wraps_into_two_balanced_lines() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::ScrollPick,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            row_selection(),
            mon,
        );
        h.run(&p, None);

        let raw = h.raw_input(None, &[]);
        let mut rows: Vec<f32> = Vec::new();
        let full = h.ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx();
            let (wrap, lines) = hint_wrap(ctx, crate::ui::components::panel::model::SCROLL_PICK_HINT);
            assert_eq!(lines, 2, "wrapped at {wrap}");
            let galley = ctx.fonts_mut(|f| f.layout_job(widgets::hint_job(crate::ui::components::panel::model::SCROLL_PICK_HINT, wrap)));
            rows = galley
                .rows
                .iter()
                .map(|r| r.rect().width())
                .collect();
        });
        full.drop_without_applying_deltas();

        assert_eq!(rows.len(), 2);
        let (short, long) = (rows[0].min(rows[1]), rows[0].max(rows[1]));
        assert!(short / long >= 0.75, "lines are lopsided: {rows:?}");
    }

    /// The scroll-pick strip is a row wherever it lands: the anchor here
    /// is the one every other set answers with a column.
    #[test]
    fn the_scroll_pick_strip_is_always_a_row() {
        let mon = monitor(rect(0, 0, 1920, 1300), 1.0);
        let sel = rect(100, 300, 400, 998);
        for &set in PanelButtonSet::ALL {
            let mut h = Harness::new(mon);
            let p = inputs(set, PanelFeatures::ALL, ButtonStyle::Below, sel, mon);
            h.run(&p, None);
            let want = if set == PanelButtonSet::ScrollPick {
                Axis::Row
            } else {
                Axis::Column
            };
            assert_eq!(h.analytic(&p).0.axis(), want, "{set:?}");
        }
    }

    /// A set's column width is its own — the capture strip's grid is two
    /// tiles wide where the OCR strip is one — but it may never move when
    /// a switch is flipped, or a user turning UPLOAD off would find the
    /// tray under a different part of the selection.
    #[test]
    fn column_width_is_fixed_per_set_under_every_switch_combination() {
        let mon = monitor(rect(0, 0, 1920, 1300), 1.0);
        let sel = rect(100, 300, 400, 998);
        for &set in PanelButtonSet::UNION {
            let mut widths = Vec::new();
            for features in FEATURE_COMBINATIONS {
                let mut h = Harness::new(mon);
                let p = inputs(set, features, ButtonStyle::Below, sel, mon);
                h.run(&p, None);
                assert_eq!(h.analytic(&p).0.axis(), Axis::Column, "{set:?} {features:?}");
                widths.push(h.area_rect().width());
            }
            let first = widths[0];
            for w in &widths {
                assert!((w - first).abs() <= 0.01, "{set:?}: {widths:?}");
            }
        }
    }

    /// The capture strip is two bare runs, the second ruled off and
    /// holding the ways out — and nothing in it carries a fill.
    #[test]
    fn capture_groups_are_bare_with_the_ways_out_ruled_off() {
        let groups = PanelButtonSet::Normal.groups();
        assert!(
            groups
                .iter()
                .all(|g| g.tone == GroupTone::Bare),
            "{groups:?}"
        );
        let ruled: Vec<bool> = groups.iter().map(|g| g.ruled).collect();
        assert_eq!(ruled, vec![false, true]);
        let last = groups.last().expect("a last group");
        let ways_out: Vec<Command> = PanelButtonSet::Normal.defs()[PanelButtonSet::Normal.defs().len() - last.len..]
            .iter()
            .map(|d| d.command)
            .collect();
        assert_eq!(ways_out, vec![Command::Reset, Command::Exit]);
    }

    #[test]
    fn dead_tray_swallows_and_reports_over_tray() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            row_selection(),
            mon,
        );
        h.run(&p, None);
        let area = h.area_rect();
        let emblem = egui::pos2(area.left() + tokens::PAD + tokens::EMBLEM_SLOT / 2.0, area.center().y);
        let out = h.press(&p, emblem);
        assert!(out.clicked.is_none() && out.over_tray && !out.over_button, "{out:?}");

        let outside = egui::pos2(area.right() + 1.0, area.center().y);
        let out = h.run(&p, Some(outside));
        assert!(!out.over_tray && !out.over_button, "{out:?}");
    }

    /// Decision 3: the overlay fires on the OS press, so the press tick
    /// must answer with the command in the same call that laid the strip
    /// out — no second event, no frame of latency.
    #[test]
    fn press_on_a_button_reports_its_command_synchronously() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            row_selection(),
            mon,
        );
        h.run(&p, None);
        // The first visible def is the strip's leftmost button.
        let first = capture_buttons(&h, Axis::Row)[0];
        let expected = PanelButtonSet::Normal
            .visible_defs(PanelFeatures::ALL)
            .next()
            .expect("a visible button")
            .1
            .command;
        let out = h.press(&p, first.center());
        assert_eq!(out.clicked, Some(expected), "{out:?}");
        assert!(out.over_button && out.over_tray, "{out:?}");
    }

    #[test]
    fn pointer_gone_clears_hover() {
        let mon = hd(1.0);
        let mut h = Harness::new(mon);
        let p = inputs(
            PanelButtonSet::Normal,
            PanelFeatures::ALL,
            ButtonStyle::KeyHint,
            row_selection(),
            mon,
        );
        h.run(&p, None);
        let first = h
            .interactive_rects()
            .into_iter()
            .find(|r| r.width() <= tokens::KEY_TILE + 0.5)
            .expect("a button rect");
        let over = h.run(&p, Some(first.center()));
        assert!(over.over_button && over.over_tray, "{over:?}");
        let gone = h.run(&p, None);
        assert!(!gone.over_button && !gone.over_tray, "{gone:?}");
    }
}
