//! Burning the drawn marks into the captured picture, so what COPY, SAVE,
//! EDIT, UPLOAD and SEARCH send out is what the overlay showed: the marks
//! cut to the selection.
//!
//! The marks are rasterised from the very primitives the overlay paints
//! (`draw::geometry::primitives`), anti-aliased, into a transparent layer
//! the size of the destination clip, then laid over the output pixels.
//! The capture the overlay is showing is never touched: an
//! action that is cancelled or fails goes back to it with the marks still
//! separate, still undoable.

use flicksnap_shared::geometry::{RectExt, ScreenRect};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use super::geometry::{primitives, Prim};
use super::Mark;

/// One rasterization of the exported marks, in desktop coordinates.
pub struct InkLayer {
    bounds: ScreenRect,
    pixels: Pixmap,
}

impl InkLayer {
    pub fn new(bounds: ScreenRect, marks: &[Mark]) -> anyhow::Result<Option<Self>> {
        if marks.is_empty() {
            return Ok(None);
        }
        let pixels =
            rasterize(marks, bounds).ok_or_else(|| anyhow!("could not create a {}x{} drawing layer", bounds.width(), bounds.height()))?;
        Ok(Some(Self {
            bounds,
            pixels,
        }))
    }
}

/// The marks drawn into a transparent layer covering `clip`, its pixel
/// (0, 0) the desktop's pixel at `clip`'s corner.
fn rasterize(marks: &[Mark], clip: ScreenRect) -> Option<Pixmap> {
    let mut layer = Pixmap::new(clip.width() as u32, clip.height() as u32)?;
    let to_layer = Transform::from_translate(-clip.left() as f32, -clip.top() as f32);
    for mark in marks {
        let mut paint = Paint::default();
        let [r, g, b, a] = mark.color.to_srgba_unmultiplied();
        paint.set_color_rgba8(r, g, b, a);
        paint.anti_alias = true;
        for prim in primitives(mark) {
            draw(&mut layer, &prim, &paint, to_layer);
        }
    }
    Some(layer)
}

/// One primitive, drawn as the overlay draws it. A primitive with no
/// area to draw (a zero-length line) has no path and draws nothing.
fn draw(layer: &mut Pixmap, prim: &Prim, paint: &Paint, to_layer: Transform) {
    let stroke = |width: f32, join: LineJoin| Stroke {
        width,
        line_cap: LineCap::Butt,
        line_join: join,
        ..Stroke::default()
    };
    match prim {
        Prim::Convex(points) => {
            if let Some(path) = polyline(points, true) {
                layer.fill_path(&path, paint, FillRule::Winding, to_layer, None);
            }
        }
        Prim::Segment {
            a,
            b,
            width,
        } => {
            if let Some(path) = polyline(&[*a, *b], false) {
                layer.stroke_path(&path, paint, &stroke(*width, LineJoin::Miter), to_layer, None);
            }
        }
        Prim::Disc {
            center,
            radius,
        } => {
            if let Some(path) = PathBuilder::from_circle(center.x, center.y, *radius) {
                layer.fill_path(&path, paint, FillRule::Winding, to_layer, None);
            }
        }
        Prim::RectStroke {
            rect,
            width,
        } => {
            let rect = tiny_skia::Rect::from_ltrb(rect.min.x, rect.min.y, rect.max.x, rect.max.y);
            if let Some(path) = rect.map(PathBuilder::from_rect) {
                layer.stroke_path(&path, paint, &stroke(*width, LineJoin::Miter), to_layer, None);
            }
        }
        Prim::Polyline {
            points,
            width,
        } => {
            if let Some(path) = polyline(points, false) {
                layer.stroke_path(&path, paint, &stroke(*width, LineJoin::Round), to_layer, None);
            }
        }
    }
}

/// A path through `points`, closed when `close` is set.
fn polyline(points: &[egui::Pos2], close: bool) -> Option<tiny_skia::Path> {
    let (first, rest) = points.split_first()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.x, first.y);
    for p in rest {
        pb.line_to(p.x, p.y);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

impl InkLayer {
    /// Lay premultiplied ink over the opaque RGBA output at `bounds`.
    pub fn blend(&self, rgba: &mut [u8], bounds: ScreenRect) {
        let Some(clip) = self.bounds.intersection(&bounds) else {
            return;
        };
        let x0 = (clip.left() - bounds.left()) as usize;
        let y0 = (clip.top() - bounds.top()) as usize;
        let src_x = (clip.left() - self.bounds.left()) as usize;
        let src_y = (clip.top() - self.bounds.top()) as usize;
        let (w, h) = (clip.width() as usize, clip.height() as usize);
        let stride = bounds.width() as usize * 4;
        let src_stride = self.pixels.width() as usize * 4;
        let src = self.pixels.data();
        for row in 0..h {
            let dst = &mut rgba[(y0 + row) * stride + x0 * 4..][..w * 4];
            let src = &src[(src_y + row) * src_stride + src_x * 4..][..w * 4];
            for (d, s) in dst
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                let a = s[3] as u32;
                if a == 0 {
                    continue;
                }
                let keep = 255 - a;
                let over = |src: u8, dst: u8| (src as u32 + (dst as u32 * keep + 127) / 255).min(255) as u8;
                d[0] = over(s[0], d[0]);
                d[1] = over(s[1], d[1]);
                d[2] = over(s[2], d[2]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use egui::{pos2, Color32};

    use super::*;
    use crate::draw::{Ink, Shape};

    fn bounds() -> ScreenRect {
        ScreenRect::from_xy_size(1000, 500, 200, 100)
    }

    fn render(marks: &[Mark], selection: ScreenRect) -> Vec<u8> {
        let mut rgba = GREY.repeat(200 * 100);
        if let Some(clip) = bounds().intersection(&selection) {
            if let Some(ink) = InkLayer::new(clip, marks).unwrap() {
                ink.blend(&mut rgba, bounds());
            }
        }
        rgba
    }

    fn px(rgba: &[u8], x: i32, y: i32) -> [u8; 4] {
        let i = ((y - 500) as usize * 200 + (x - 1000) as usize) * 4;
        rgba[i..i + 4]
            .try_into()
            .expect("four bytes")
    }

    fn line(from: (f32, f32), to: (f32, f32), color: Color32) -> Mark {
        Mark {
            ink: Ink::Shape {
                shape: Shape::Line,
                from: pos2(from.0, from.1),
                to: pos2(to.0, to.1),
            },
            color,
            scale: 1.0,
        }
    }

    const RED_RGBA: [u8; 4] = [0xDF, 0x28, 0x28, 0xFF];
    const GREY: [u8; 4] = [0x40, 0x40, 0x40, 0xFF];

    /// A mark is burned in its colour and position.
    #[test]
    fn a_mark_is_burned_where_it_was_drawn() {
        let red = crate::draw::PALETTE[0];
        let out = render(&[line((1020.0, 550.5), (1180.0, 550.5), red)], bounds());
        assert_eq!(px(&out, 1100, 550), RED_RGBA);
        assert_eq!(px(&out, 1100, 540), GREY, "well off the line");
    }

    #[test]
    fn the_line_has_the_same_edge_coverage_at_the_caps_and_the_middle() {
        let red = crate::draw::PALETTE[0];
        let out = render(&[line((1020.5, 550.0), (1179.5, 550.0), red)], bounds());
        for y in [548, 551] {
            let middle = px(&out, 1100, y)[0];
            assert!(middle > GREY[0] && middle < RED_RGBA[0]);
            for x in [1021, 1178] {
                let edge = px(&out, x, y)[0];
                assert!(edge.abs_diff(middle) <= 1, "({x}, {y}): {edge} vs {middle}");
            }
        }
    }

    /// A mark running out of the selection is burned only inside it.
    #[test]
    fn a_mark_is_cut_to_the_selection() {
        let red = crate::draw::PALETTE[0];
        let selection = ScreenRect::from_xy_size(1050, 520, 100, 60);
        let out = render(&[line((1010.0, 550.5), (1190.0, 550.5), red)], selection);
        assert_eq!(px(&out, 1100, 550), RED_RGBA, "inside");
        assert_eq!(px(&out, 1149, 550), RED_RGBA, "at the selection's last column");
        assert_eq!(px(&out, 1150, 550), GREY, "past it");
        assert_eq!(px(&out, 1030, 550), GREY, "before it");
    }

    /// Marks burn oldest first, a later one over an earlier.
    #[test]
    fn a_later_mark_burns_over_an_earlier_one() {
        let (red, blue) = (crate::draw::PALETTE[0], crate::draw::PALETTE[4]);
        let marks = [
            line((1020.0, 550.5), (1180.0, 550.5), red),
            line((1100.5, 510.0), (1100.5, 590.0), blue),
        ];
        let out = render(&marks, bounds());
        assert_eq!(px(&out, 1100, 550), [blue.r(), blue.g(), blue.b(), 0xFF]);
    }

    /// A brush stroke and a click's dot both burn; no marks, or a selection
    /// off the capture, burn nothing.
    #[test]
    fn brush_strokes_burn_and_nothing_burns_from_nothing() {
        let red = crate::draw::PALETTE[0];
        let brush = |points: Vec<egui::Pos2>| Mark {
            ink: Ink::Brush(Arc::new(points)),
            color: red,
            scale: 1.0,
        };
        let out = render(
            &[
                brush(vec![pos2(1020.5, 520.5), pos2(1060.5, 560.5), pos2(1100.5, 520.5)]),
                brush(vec![pos2(1150.5, 580.5)]),
            ],
            bounds(),
        );
        assert_eq!(px(&out, 1060, 560), RED_RGBA, "the stroke's bend");
        assert_eq!(px(&out, 1150, 580), RED_RGBA, "the dot");

        assert_eq!(render(&[], bounds()), GREY.repeat(200 * 100));
        let off = ScreenRect::from_xy_size(0, 0, 50, 50);
        assert_eq!(render(&[line((0.0, 0.0), (40.0, 40.0), red)], off), GREY.repeat(200 * 100));
    }
}
