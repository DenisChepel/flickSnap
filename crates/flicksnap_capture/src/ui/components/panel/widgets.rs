//! The tray's pieces as egui widgets: the button styles, the OCR word
//! count, the instruction, the rule and the emblem.
//!
//! Only the buttons are interactive. The readout and the emblem allocate
//! space and paint into it, so the tray body under them is dead in the
//! same way the retired composer's chassis was. A button has no fill of
//! its own at rest: it sits on its group's fill (`theme::group_frame`)
//! and paints a rounded veil in the group's colour lightened only while
//! hovered.

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, AtomExt as _, Color32, Direction, Frame, Id, Image, ImageSource, Margin, Response, RichText, Sense, TextStyle, Ui, Vec2,
    WidgetAtom, WidgetText,
};

use super::assets;
use super::model::ButtonDef;
use super::theme::{self, tokens};

/// An icon and, optionally, a piece of text in a rounded segment. Every
/// panel style is a parameter set of this one widget: `key` lays them out
/// side by side in a square, `below` stacks them, `icon` drops the text.
pub struct StackedButton {
    pub id: Id,
    pub icon: ImageSource<'static>,
    pub text: Option<WidgetText>,
    pub min_size: Vec2,
    pub direction: Direction,
    pub gap: f32,
    pub pad_h: f32,
    /// The fill of the group this button sits in: what the hover veil
    /// lightens, and what the button paints (invisibly) at rest.
    pub base: Color32,
    /// How far the hover lightens `base` (`theme::hover_veil`).
    pub veil: f32,
    /// Where the contents sit in whatever room `min_size` leaves over.
    /// Centred is the default and what every strip along an axis wants;
    /// `LEFT_CENTER` is for a stack of buttons of one width, where
    /// centring would put each icon at its own x.
    pub align: Align2,
}

impl StackedButton {
    pub fn show(self, ui: &mut Ui) -> Response {
        // The loader rasterises the mark at the exact pixel size this
        // quad covers on this host, so it is 1:1 at every DPI. Never a
        // spinner: `assets::preload` has already loaded (or logged) it,
        // and the loader is synchronous anyway.
        let icon = Image::new(self.icon.clone())
            .fit_to_exact_size(tokens::ICON_SIZE)
            .show_loading_spinner(false)
            .atom_size(tokens::ICON_SIZE);
        let layout = match self.text {
            Some(text) => WidgetAtom::new((icon, text)),
            None => WidgetAtom::new(icon),
        };
        let layout = layout
            .id(self.id)
            .direction(self.direction)
            .gap(self.gap)
            .sense(Sense::CLICK)
            .min_size(self.min_size)
            // The contents sit where `align` puts them in whatever room
            // `min_size` leaves over — centred, as the retired kit's tile
            // and button boxes were, unless a caller asks otherwise.
            .align2(self.align)
            .frame(
                Frame::new()
                    .inner_margin(Margin::symmetric(self.pad_h as i8, 0))
                    .corner_radius(tokens::RADIUS),
            );
        // Allocate first so the fill can be chosen from this pass's hover
        // state, then paint: one rect at the blended colour, never a
        // second translucent rect over the group.
        let mut allocated = layout.allocate(ui);
        let t = ui
            .ctx()
            .animate_bool_with_time(self.id.with("hover"), allocated.response.hovered(), tokens::HOVER_FADE_SECS);
        allocated.container.frame.fill = theme::hover_fill(self.base, self.veil, t);
        allocated.paint(ui).response
    }
}

/// `key` style: a `KEY_TILE` square, the icon beside the accelerator letter
/// (white at 80 %: the earlier 45 % tag was too dim to read on either
/// group fill).
pub fn key_hint_button(def: &ButtonDef, id: Id, min_size: Vec2, base: Color32, veil: f32) -> StackedButton {
    let letter = RichText::new(
        def.accel_key()
            .to_ascii_uppercase()
            .to_string(),
    )
    .text_style(TextStyle::Small)
    .color(tokens::FG_80);
    StackedButton {
        id,
        icon: def.icon.source(),
        text: Some(letter.into()),
        min_size,
        direction: Direction::LeftToRight,
        gap: tokens::KEY_GAP,
        pad_h: tokens::KEY_PAD_H,
        base,
        veil,
        align: Align2::CENTER_CENTER,
    }
}

/// `below` style: `BELOW_HEIGHT` tall, the icon over the Title-case label with the
/// accelerator glyph underlined.
pub fn below_button(def: &ButtonDef, id: Id, min_size: Vec2, base: Color32, veil: f32) -> StackedButton {
    StackedButton {
        id,
        icon: def.icon.source(),
        text: Some(WidgetText::from(underlined_label(def))),
        min_size,
        direction: Direction::TopDown,
        gap: tokens::BELOW_GAP,
        pad_h: tokens::BELOW_PAD_H,
        base,
        veil,
        align: Align2::CENTER_CENTER,
    }
}

/// `icon` style — the capture strip: a `key`-sized square holding the
/// icon alone, centred. What it does and its accelerator are in the hover
/// chip ([`tip_job`]).
pub fn icon_button(def: &ButtonDef, id: Id, min_size: Vec2, base: Color32, veil: f32) -> StackedButton {
    StackedButton {
        id,
        icon: def.icon.source(),
        text: None,
        min_size,
        direction: Direction::LeftToRight,
        gap: 0.0,
        pad_h: tokens::KEY_PAD_H,
        base,
        veil,
        align: Align2::CENTER_CENTER,
    }
}

/// The tooltip chip's text: what the button does (`ButtonDef::tip`),
/// 11 pt, white, then the accelerator key dimmed after a middle dot — the
/// icon-only buttons carry no letter of their own, so the chip is where
/// the key is learned.
pub fn tip_job(def: &ButtonDef) -> LayoutJob {
    tip_text_job(def.tip, Some(def.accel_key()))
}

/// A tooltip chip's text: `text`, 11 pt, white, and — for a button that
/// has one — its key dimmed after a middle dot.
pub fn tip_text_job(text: &str, key: Option<char>) -> LayoutJob {
    let font = egui::FontId::new(tokens::TIP_FONT, egui::FontFamily::Proportional);
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat {
            font_id: font.clone(),
            color: tokens::FG,
            ..Default::default()
        },
    );
    if let Some(key) = key {
        job.append(
            &format!("  ·  {}", key.to_ascii_uppercase()),
            0.0,
            TextFormat {
                font_id: font,
                color: tokens::FG_70,
                ..Default::default()
            },
        );
    }
    job
}

/// The tooltip chip at `rect`: the dark fill and the label centred in it.
/// A plain painter call rather than a widget: the chip is never
/// interactive and must not take the hover from the button under it.
pub fn tip(painter: &egui::Painter, rect: egui::Rect, job: LayoutJob) {
    painter.rect_filled(rect, tokens::TIP_RADIUS, tokens::TIP_FILL);
    let galley = painter.layout_job(job);
    // Centred on the box, not the ink: a tip with a descender must not
    // sit higher than one without, and the chip is padded for the box.
    painter.galley(rect.center() - galley.size() / 2.0, galley, tokens::FG);
}

/// The label in three sections, the middle one underlined: `RichText`
/// underlines a whole run, not a substring. Labels are ASCII (a model
/// test pins it), so the byte index is also the glyph index.
pub fn underlined_label(def: &ButtonDef) -> LayoutJob {
    let font = egui::FontId::new(tokens::LABEL_FONT, crate::ui::fonts::SEMIBOLD.clone());
    let plain = TextFormat {
        font_id: font,
        color: tokens::FG,
        ..Default::default()
    };
    let under = TextFormat {
        underline: egui::Stroke {
            width: 1.0,
            color: tokens::FG,
        },
        ..plain.clone()
    };
    let (a, b) = (def.underline_idx, def.underline_idx + 1);
    let mut job = LayoutJob::default();
    job.append(&def.label[..a], 0.0, plain.clone());
    job.append(&def.label[a..b], 0.0, under);
    job.append(&def.label[b..], 0.0, plain);
    job
}

/// The readout stacked on 1 em line boxes so the lines sit tight. Bold
/// number at 85 % and the regular "words" caption at 70 %.
pub fn readout_job(word_count: usize) -> LayoutJob {
    let f = |family: egui::FontFamily, color: Color32| TextFormat {
        font_id: egui::FontId::new(tokens::READOUT_FONT, family),
        color,
        line_height: Some(tokens::READOUT_LINE),
        ..Default::default()
    };
    let mut job = LayoutJob {
        halign: egui::Align::Center,
        ..Default::default()
    };
    job.append(
        &format!("{word_count}\n"),
        0.0,
        f(crate::ui::fonts::SEMIBOLD.clone(), tokens::FG_85),
    );
    job.append("words", 0.0, f(egui::FontFamily::Proportional, tokens::FG_70));
    job
}

/// The instruction's layout job at one wrap width. Regular weight at
/// 85 %, like the readout's numbers: it is the strip's one piece of
/// prose, and it sits straight on the chassis with no fill of its own.
pub fn hint_job(text: &str, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat {
            font_id: egui::FontId::new(tokens::HINT_FONT, egui::FontFamily::Proportional),
            color: tokens::FG_85,
            line_height: Some(tokens::HINT_LINE),
            ..Default::default()
        },
    );
    job.wrap.max_width = wrap_width;
    job
}

/// The instruction in a dead slot, its block centred in the slot. Box
/// centring, not ink centring: a wrapped paragraph must sit on its line
/// boxes, or a line without descenders would ride differently from one
/// with them.
pub fn hint(ui: &mut Ui, text: &str, wrap_width: f32, slot: Vec2) -> egui::Rect {
    let galley = ui
        .painter()
        .layout_job(hint_job(text, wrap_width));
    let (rect, _) = ui.allocate_exact_size(slot, Sense::hover());
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, tokens::FG_85);
    rect
}

/// The readout in a dead slot, centred on its ink: the galley's box
/// carries Inter's ascent and line gap above the digits, so centring the box
/// leaves the glyphs riding high, more so at every DPI step.
/// `mesh_bounds` is the glyphs' own extent, relative to the galley's
/// origin (`halign: Center` puts that origin on the block's centre line).
pub fn readout(ui: &mut Ui, word_count: usize, slot: Vec2) -> egui::Rect {
    let galley = ui
        .painter()
        .layout_job(readout_job(word_count));
    let (rect, _) = ui.allocate_exact_size(slot, Sense::hover());
    ui.painter()
        .galley(ink_centred(rect, &galley), galley, tokens::FG_85);
    rect
}

/// Where to put a galley so its ink, not its box, is centred in `rect`.
pub fn ink_centred(rect: egui::Rect, galley: &egui::Galley) -> egui::Pos2 {
    let ink = galley.mesh_bounds;
    if ink.is_positive() {
        rect.center() - ink.center().to_vec2()
    } else {
        rect.center() - galley.size() / 2.0
    }
}

/// A hairline between two runs of buttons, inset from either end, with
/// air on both sides of it: across a row it stands upright `thick` tall,
/// along a column it lies flat `thick` wide. Dead space — it allocates
/// and paints, and nothing else, the way the readout and the emblem do.
pub fn divider(ui: &mut Ui, upright: bool, thick: f32) -> egui::Rect {
    let size = if upright {
        Vec2::new(tokens::DIVIDER_BLOCK, thick)
    } else {
        Vec2::new(thick, tokens::DIVIDER_BLOCK)
    };
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if upright {
        ui.painter().vline(
            rect.center().x,
            (rect.top() + tokens::DIVIDER_INSET)..=(rect.bottom() - tokens::DIVIDER_INSET),
            tokens::DIVIDER,
        );
    } else {
        ui.painter().hline(
            (rect.left() + tokens::DIVIDER_INSET)..=(rect.right() - tokens::DIVIDER_INSET),
            rect.center().y,
            tokens::DIVIDER,
        );
    }
    rect
}

/// The emblem in a dead slot: no fill, no hover, the mark centred at its
/// own size (the brand blue is baked into the SVG).
pub fn emblem(ui: &mut Ui, slot: Vec2) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(slot, Sense::hover());
    let mark = Vec2::splat(tokens::EMBLEM_MARK);
    // `paint_at` rounds its rect to whole pixels and asks the loader for
    // exactly that many, so the mark is 1:1 at every DPI.
    Image::new(assets::FLICKSNAP_LOGO.source())
        .fit_to_exact_size(mark)
        .show_loading_spinner(false)
        .paint_at(ui, egui::Rect::from_center_size(rect.center(), mark));
    rect
}
