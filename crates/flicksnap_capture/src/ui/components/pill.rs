//! The chip style the pill overlays share.
//!
//! The OCR notice and the OCR text bubbles read as one family — same fill,
//! border, radius, paddings and text colour — so the values and the
//! painters live here rather than in whichever overlay happened to define
//! them first. That is what keeps them from drifting apart: nothing pulls a
//! colour out of another overlay's module.
//!
//! Everything is in points. The old renderer multiplied each constant by
//! the monitor's DPI and floored it; egui does the pixel rounding itself,
//! so the literals below are the old pre-DPI numbers used as-is.

use egui::epaint::RectShape;
use egui::{Color32, FontFamily, FontId, Rect, Stroke, StrokeKind, Vec2};

/// Chip body: white at 80 %.
pub const FILL: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 204);
/// The hairline around it, light grey at the same alpha.
pub const BORDER: Color32 = Color32::from_rgba_unmultiplied_const(191, 191, 191, 204);
/// The hard-edged drop shadow under it.
pub const SHADOW: Color32 = Color32::from_black_alpha(102);
/// The one knob for text weight: the old glyph shader linearised 0x1A
/// before writing to a non-sRGB target, so the old text came out heavier
/// than nominal; egui draws 0x1A as 0x1A. Darken here if the visual pass
/// wants the old weight back.
pub const TEXT: Color32 = Color32::from_rgba_unmultiplied_const(0x1A, 0x1A, 0x1A, 0xE0);

pub const FONT_PT: f32 = 11.0;
pub const PAD_H: f32 = 6.0;
pub const PAD_V: f32 = 4.0;
pub const RADIUS: f32 = 6.0;
/// The old `dpi.ceil()` hairline, one point wide.
pub const BORDER_W: f32 = 1.0;
pub const SHADOW_OFFSET: f32 = 3.0;
pub const SHADOW_SPREAD: f32 = 2.0;

pub fn font() -> FontId {
    FontId::new(FONT_PT, FontFamily::Proportional)
}

/// The chip's shadow: its rect moved down-right and grown, with no blur —
/// the old one had none either.
pub fn shadow(rect: Rect, alpha: f32) -> RectShape {
    RectShape::filled(
        rect.translate(Vec2::splat(SHADOW_OFFSET))
            .expand(SHADOW_SPREAD),
        RADIUS + SHADOW_SPREAD,
        SHADOW.gamma_multiply(alpha),
    )
}

/// Fill plus, when `border` is set, the inside hairline.
pub fn body(rect: Rect, radius: f32, alpha: f32, border: bool) -> RectShape {
    let stroke = if border {
        Stroke::new(BORDER_W, BORDER.gamma_multiply(alpha))
    } else {
        Stroke::NONE
    };
    RectShape::new(rect, radius, FILL.gamma_multiply(alpha), stroke, StrokeKind::Inside)
}
