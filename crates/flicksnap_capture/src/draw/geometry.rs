//! What a mark looks like, as primitives every renderer can draw the same
//! way.
//!
//! Draw-on-screen's shapes ported expression for expression: the line is
//! `GraphicLine` (a round-capped pen), the rectangle `GraphicRectangle`
//! (square corners, its pen inset so the ink stays inside the dragged
//! box, the box rounded to whole units) and the arrow the shared
//! `ArrowGeometry` (a shaft tapering from a thin round tail, a swept-back
//! head whose corners a round-joined outline softens). The Shift snap is
//! `AngleSnap`. The brush is the screenshot strip's own, kept minimal for
//! quick marks: a line of the same width through the stroke's points,
//! round at both ends.
//!
//! Draw-on-screen measures all of this in canvas units — the monitor's
//! DIPs — so every length here is a canvas length times the mark's
//! `scale`, the DPI scale of the monitor it was drawn on. The result is in
//! virtual-desktop physical px, ready for any renderer.
//!
//! Every primitive is one a renderer can draw exactly and seam-free on its
//! own: convex fills, butt-ended segments, discs, a stroked rectangle. A
//! concave fill (the head) comes as two convex pieces that overlap past
//! their shared edge, so an anti-aliased renderer never leaves a hairline
//! between them; round caps and joins come as discs on the ends.

use std::sync::Arc;

use egui::{pos2, vec2, Pos2, Rect, Vec2};

use super::{Ink, Mark, Shape};

/// One piece of a mark, in physical px.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    /// A filled convex polygon.
    Convex(Vec<Pos2>),
    /// A straight line `width` wide with butt ends.
    Segment { a: Pos2, b: Pos2, width: f32 },
    /// A filled circle.
    Disc { center: Pos2, radius: f32 },
    /// A rectangle's outline, the pen centred on `rect`'s edges, square
    /// corners.
    RectStroke { rect: Rect, width: f32 },
    /// An open line `width` wide through `points`, joined where it bends,
    /// with butt ends. Shared with the brush stroke it is drawn from, not
    /// copied.
    Polyline { points: Arc<Vec<Pos2>>, width: f32 },
}

/// The line width every shape draws at, in canvas units: draw-on-screen's
/// thinnest size (`DrawPalette.Sizes[0].Stroke`), which line, arrow and
/// rectangle all take as is (`DrawPalette.LineWidthFor`).
pub const STROKE: f32 = 3.0;

/// The pieces of `mark`, in physical px.
pub fn primitives(mark: &Mark) -> Vec<Prim> {
    let width = STROKE * mark.scale;
    match &mark.ink {
        &Ink::Shape {
            shape,
            from,
            to,
        } => match shape {
            Shape::Line => vec![line(from, to, width)],
            Shape::Arrow => arrow(from, to, width, mark.scale),
            Shape::Rectangle => vec![rectangle(from, to, width, mark.scale)],
        },
        Ink::Brush(points) => brush(points, width),
    }
}

/// The brush: a round-capped line through the stroke's points, as thick
/// as every other mark's — the stroke's points are its geometry, so this
/// is the same cheap call whether the stroke is three points long or
/// three thousand. A click is a dot the line's width across.
fn brush(points: &Arc<Vec<Pos2>>, width: f32) -> Vec<Prim> {
    let cap = |center: Pos2| Prim::Disc {
        center,
        radius: width / 2.0,
    };
    match (points.first(), points.last()) {
        (Some(&first), Some(&last)) if points.len() > 1 => vec![
            Prim::Polyline {
                points: points.clone(),
                width,
            },
            cap(first),
            cap(last),
        ],
        (Some(&dot), _) => vec![cap(dot)],
        _ => Vec::new(),
    }
}

/// `GraphicLine`: fill a single outline so antialiasing the caps does
/// not blend a second time over the sides of the stroke.
fn line(a: Pos2, b: Pos2, width: f32) -> Prim {
    const CAP_STEPS: usize = 12;
    let radius = width / 2.0;
    if a == b {
        return Prim::Disc {
            center: a,
            radius,
        };
    }

    let tangent = (b - a).normalized();
    let normal = vec2(-tangent.y, tangent.x);
    let cap = (0..=CAP_STEPS).map(|i| {
        let angle = std::f32::consts::PI * i as f32 / CAP_STEPS as f32;
        let (sin, cos) = angle.sin_cos();
        (normal * cos - tangent * sin) * radius
    });
    Prim::Convex(
        cap.clone()
            .map(|offset| a + offset)
            .chain(cap.map(|offset| b - offset))
            .collect(),
    )
}

/// `GraphicRectangle`: the dragged box rounded to whole canvas units
/// (`CreateRectSafeRounded`), its pen inset by half a width so the outer
/// edge of the ink is the box (`DrawRectangle`), never thinner than one
/// unit inside.
fn rectangle(a: Pos2, b: Pos2, width: f32, scale: f32) -> Prim {
    let round = |v: f32| (v / scale).round() * scale;
    let (left, right) = (a.x.min(b.x), a.x.max(b.x));
    let (top, bottom) = (a.y.min(b.y), a.y.max(b.y));
    let left = round(left);
    let top = round(top);
    let w = round(right - left);
    let h = round(bottom - top);
    let inner = vec2((w - width).max(scale), (h - width).max(scale));
    Prim::RectStroke {
        rect: Rect::from_min_size(pos2(left + width / 2.0, top + width / 2.0), inner),
        width,
    }
}

/// `ArrowGeometry` — head length as a multiple of the stroke, floored so a
/// thin arrow still gets a readable head and capped against the path so a
/// short one is not all head.
const HEAD_LENGTH_FACTOR: f32 = 5.0;
const MIN_HEAD_LENGTH: f32 = 14.0;
const MAX_HEAD_FRACTION: f32 = 0.45;
/// Half-width of the head against its length (about a 32° half angle).
const HEAD_ASPECT: f32 = 0.62;
/// How far the rear notch is pushed toward the tip, against head length.
const NOTCH_FRACTION: f32 = 0.22;
/// How far the shaft runs past the notch into the head, burying the seam.
const SHAFT_OVERLAP_FRACTION: f32 = 0.4;
/// The shaft widens from `TAIL_WIDTH_FACTOR` to `SHAFT_WIDTH_FACTOR` times
/// the stroke, never thinner than `MIN_TAIL_WIDTH` canvas units.
const SHAFT_WIDTH_FACTOR: f32 = 1.5;
const TAIL_WIDTH_FACTOR: f32 = 0.6;
const MIN_TAIL_WIDTH: f32 = 2.0;
/// Samples along a straight shaft: its width profile is not linear.
const STRAIGHT_SPINE_SAMPLES: usize = 12;

/// Thickness of the round-joined outline that softens the head's corners.
fn corner_rounding(width: f32, scale: f32) -> f32 {
    scale.max(width * 0.7)
}

/// `ArrowGeometry.Build` for a straight, solid, tapered arrow from `start`
/// to `end`.
fn arrow(start: Pos2, end: Pos2, width: f32, scale: f32) -> Vec<Prim> {
    let corner = corner_rounding(width, scale);
    let path = end - start;
    let path_length = path.length();
    let direction = if path_length > 0.0 { path / path_length } else { vec2(1.0, 0.0) };
    let head_length = (path_length * MAX_HEAD_FRACTION).min((width * HEAD_LENGTH_FACTOR).max(MIN_HEAD_LENGTH * scale));

    // The head, placed so its rounded point lands on `end`: the corner
    // outline grows it by half its thickness, so the tip is pulled back.
    let normal = vec2(-direction.y, direction.x);
    let half_width = head_length * HEAD_ASPECT;
    let tip_inset = (corner / 2.0) / half_width.atan2(head_length).sin();
    let tip = end - direction * tip_inset;
    let base = tip - direction * head_length;
    let left = base + normal * half_width;
    let right = base - normal * half_width;
    let notch = base + direction * (head_length * NOTCH_FRACTION);

    let mut out = Vec::new();
    let shaft_length = path_length - tip_inset - head_length * (1.0 - NOTCH_FRACTION - SHAFT_OVERLAP_FRACTION);
    if shaft_length > 0.0 {
        let shaft_end = start + direction * shaft_length;
        out.extend(taper(start, shaft_end, direction, width, scale));
    }
    out.extend(head(tip, left, notch, right));
    out.extend(round_outline(&[tip, left, notch, right], corner));
    out
}

/// The head's dart (tip, left wing, notch, right wing) as two convex
/// pieces: the left half, and the right half reaching back past the
/// tip–notch diagonal to the left half's centroid, so the two overlap
/// across the edge they share.
fn head(tip: Pos2, left: Pos2, notch: Pos2, right: Pos2) -> [Prim; 2] {
    let centroid = pos2((tip.x + left.x + notch.x) / 3.0, (tip.y + left.y + notch.y) / 3.0);
    [
        Prim::Convex(vec![tip, left, notch]),
        Prim::Convex(vec![tip, centroid, notch, right]),
    ]
}

/// A closed outline `width` wide with round joins: each edge a segment,
/// each corner a disc.
fn round_outline(points: &[Pos2], width: f32) -> Vec<Prim> {
    let n = points.len();
    let edges = (0..n).map(|i| Prim::Segment {
        a: points[i],
        b: points[(i + 1) % n],
        width,
    });
    let corners = points.iter().map(|&center| Prim::Disc {
        center,
        radius: width / 2.0,
    });
    edges.chain(corners).collect()
}

/// `ArrowGeometry.BuildTaper` on a straight spine: a stroke whose width
/// grows by arc length, easing out, from the tail's to the shaft's full
/// weight, with a semicircular tail cap. The head covers the wide end, so
/// it is left square.
///
/// The outline draw-on-screen fills is not convex where the cap meets the
/// sides — the width climbs fastest right at the tail — so it comes here
/// as its two convex parts: the body, its tail end a straight edge across
/// the start, and the cap as the disc it is a half of. The body's tail
/// edge is the disc's diameter and the disc's front half lies inside the
/// body, so the two cover each other's edges.
fn taper(start: Pos2, end: Pos2, direction: Vec2, width: f32, scale: f32) -> [Prim; 2] {
    let n = STRAIGHT_SPINE_SAMPLES + 1;
    let spine: Vec<Pos2> = (0..n)
        .map(|i| start.lerp(end, i as f32 / STRAIGHT_SPINE_SAMPLES as f32))
        .collect();
    let mut along = vec![0.0f32; n];
    for i in 1..n {
        along[i] = along[i - 1] + spine[i - 1].distance(spine[i]);
    }
    let total = along[n - 1];

    let head_half = (MIN_TAIL_WIDTH * scale).max(width * SHAFT_WIDTH_FACTOR) / 2.0;
    let tail_half = head_half.min((MIN_TAIL_WIDTH * scale).max(width * TAIL_WIDTH_FACTOR) / 2.0);
    let half_at = |i: usize| {
        if total <= 0.0 {
            return head_half;
        }
        let f = 1.0 - along[i] / total;
        tail_half + (head_half - tail_half) * (1.0 - f * f)
    };
    let normal = vec2(-direction.y, direction.x);

    let body = (0..n)
        .map(|i| spine[i] + normal * half_at(i))
        .chain(
            (0..n)
                .rev()
                .map(|i| spine[i] - normal * half_at(i)),
        )
        .collect();
    [
        Prim::Convex(body),
        Prim::Disc {
            center: start,
            radius: tail_half,
        },
    ]
}

/// The Shift snap step for lines and arrows, in degrees.
const SNAP_STEP_DEGREES: f32 = 15.0;

/// `AngleSnap.Snap`: `point` about `anchor` turned to the nearest 15°, or
/// with `diagonal_only` to the nearest diagonal (the square constraint),
/// keeping its distance along the snapped direction.
fn snap(anchor: Pos2, point: Pos2, diagonal_only: bool) -> Pos2 {
    let d = point - anchor;
    let degrees = d.y.atan2(d.x).to_degrees();
    let closest = if diagonal_only {
        let angle = (degrees + 360.0 + 45.0) % 360.0;
        (angle / 90.0).round() * 90.0 - 45.0
    } else {
        let angle = (degrees + 360.0) % 360.0;
        (angle / SNAP_STEP_DEGREES).round() * SNAP_STEP_DEGREES
    };
    let theta = closest.to_radians();
    let unit = vec2(theta.cos(), theta.sin());
    anchor + unit * d.dot(unit)
}

/// The snap a shape takes under Shift: any 15° for a line or an arrow, a
/// diagonal for a box — draw-on-screen's `SnapMode.All` and
/// `SnapMode.Diagonal`.
pub fn snap_for(shape: Shape, anchor: Pos2, point: Pos2) -> Pos2 {
    match shape {
        Shape::Rectangle => snap(anchor, point, true),
        Shape::Line | Shape::Arrow => snap(anchor, point, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Color32;

    fn mark(shape: Shape, from: Pos2, to: Pos2, scale: f32) -> Mark {
        Mark {
            ink: Ink::Shape {
                shape,
                from,
                to,
            },
            color: Color32::RED,
            scale,
        }
    }

    /// Whether `points` wind one way all round: a convex polygon.
    fn is_convex(points: &[Pos2]) -> bool {
        let n = points.len();
        let mut sign = 0.0f32;
        for i in 0..n {
            let (a, b, c) = (points[i], points[(i + 1) % n], points[(i + 2) % n]);
            let cross = (b - a).x * (c - b).y - (b - a).y * (c - b).x;
            if cross.abs() < 1e-4 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        true
    }

    #[test]
    fn every_fill_is_convex_for_any_arrow() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for (dx, dy) in [
                (300.0, 0.0),
                (0.0, -200.0),
                (-120.0, 80.0),
                (20.0, 10.0),
                (5.0, 0.0),
                (900.0, 700.0),
            ] {
                let m = mark(Shape::Arrow, pos2(500.0, 500.0), pos2(500.0 + dx, 500.0 + dy), scale);
                for p in primitives(&m) {
                    if let Prim::Convex(points) = p {
                        assert!(is_convex(&points), "{dx},{dy} at {scale}: {points:?}");
                    }
                }
            }
        }
    }

    /// The head's tip is pulled back by draw-on-screen's inset — half the
    /// corner outline over the sine of the head's half angle — so with the
    /// round outline on it the ink stops just short of the point dragged
    /// to, exactly where draw-on-screen's does.
    #[test]
    fn the_arrow_tip_sits_where_draw_on_screen_puts_it() {
        let end = pos2(400.0, 100.0);
        let m = mark(Shape::Arrow, pos2(100.0, 100.0), end, 1.0);
        let reach = primitives(&m)
            .iter()
            .filter_map(|p| match p {
                Prim::Disc {
                    center,
                    radius,
                } => Some(center.x + radius),
                _ => None,
            })
            .fold(f32::MIN, f32::max);
        let head = 15.0f32;
        let corner = 3.0 * 0.7;
        let inset = (corner / 2.0) / (head * HEAD_ASPECT).atan2(head).sin();
        assert!((reach - (end.x - inset + corner / 2.0)).abs() < 1e-3, "{reach}");
    }

    /// A thin arrow's head is five strokes long (15 units, over the
    /// 14-unit floor), its wings 0.62 of that to either side.
    #[test]
    fn a_thin_arrow_gets_a_five_stroke_head_at_every_scale() {
        for scale in [1.0, 1.5, 2.0] {
            let m = mark(Shape::Arrow, pos2(0.0, 0.0), pos2(1000.0, 0.0), scale);
            let widest = primitives(&m)
                .iter()
                .filter_map(|p| match p {
                    Prim::Convex(points) => Some(
                        points
                            .iter()
                            .map(|q| q.y.abs())
                            .fold(0.0, f32::max),
                    ),
                    _ => None,
                })
                .fold(0.0, f32::max);
            assert!((widest - 15.0 * scale * HEAD_ASPECT).abs() < 1e-3, "at {scale}: {widest}");
        }
    }

    /// The shaft tapers: thin at the tail, full weight where it meets the
    /// head, the tail rounded off by a disc as wide as the tail.
    #[test]
    fn the_shaft_tapers_from_tail_to_head() {
        let [Prim::Convex(body), Prim::Disc {
            center,
            radius,
        }] = taper(pos2(0.0, 0.0), pos2(300.0, 0.0), vec2(1.0, 0.0), 3.0, 1.0)
        else {
            panic!("a body and a tail cap");
        };
        let at = |x: f32| {
            body.iter()
                .filter(|p| (p.x - x).abs() < 1e-3)
                .map(|p| p.y.abs())
                .fold(0.0, f32::max)
        };
        assert!((at(0.0) - 1.0).abs() < 1e-4, "tail half is max(2, 1.8) / 2");
        assert!((at(300.0) - 2.25).abs() < 1e-4, "head half is 4.5 / 2");
        assert!(at(150.0) > 1.0 && at(150.0) < 2.25);
        assert_eq!((center, radius), (pos2(0.0, 0.0), 1.0));
    }

    #[test]
    fn the_line_is_a_round_capped_stroke_three_units_wide() {
        let (a, b) = (pos2(10.0, 10.0), pos2(110.0, 60.0));
        let prims = primitives(&mark(Shape::Line, a, b, 1.5));
        let [Prim::Convex(outline)] = prims.as_slice() else {
            panic!("the line is a single fill");
        };
        assert!(is_convex(outline));
        let tangent = (b - a).normalized();
        let normal = vec2(-tangent.y, tangent.x);
        let mut bounds = Rect::NOTHING;
        for &p in outline {
            bounds.extend_with(pos2((p - a).dot(tangent), (p - a).dot(normal)));
        }
        assert!((bounds.width() - (a.distance(b) + 4.5)).abs() < 1e-3);
        assert!((bounds.height() - 4.5).abs() < 1e-3);
    }

    /// The rectangle's ink stays inside the box dragged, whichever way it
    /// was dragged, the box rounded to whole units.
    #[test]
    fn the_rectangle_inks_inside_the_box_dragged() {
        for (from, to) in [
            (pos2(100.0, 50.0), pos2(300.0, 150.0)),
            (pos2(300.0, 150.0), pos2(100.0, 50.0)),
            (pos2(100.4, 50.4), pos2(300.4, 150.4)),
        ] {
            let Prim::RectStroke {
                rect,
                width,
            } = rectangle(from, to, 3.0, 1.0)
            else {
                panic!("a rectangle is one stroke");
            };
            assert_eq!(width, 3.0);
            assert_eq!(
                rect.expand(width / 2.0),
                Rect::from_min_max(pos2(100.0, 50.0), pos2(300.0, 150.0)),
                "{from:?} {to:?}"
            );
        }
    }

    /// A brush stroke is one line through its points, shared rather than
    /// copied, with a round cap at each end; a click is a dot as wide as
    /// the line.
    #[test]
    fn the_brush_is_a_round_capped_line_and_a_click_a_dot() {
        let points = Arc::new(vec![pos2(10.0, 10.0), pos2(20.0, 14.0), pos2(30.0, 10.0)]);
        let m = Mark {
            ink: Ink::Brush(points.clone()),
            color: Color32::RED,
            scale: 2.0,
        };
        let prims = primitives(&m);
        let Prim::Polyline {
            points: line,
            width,
        } = &prims[0]
        else {
            panic!("a line first");
        };
        assert!(Arc::ptr_eq(line, &points) && *width == 6.0);
        assert_eq!(
            prims[1..],
            [
                Prim::Disc {
                    center: pos2(10.0, 10.0),
                    radius: 3.0
                },
                Prim::Disc {
                    center: pos2(30.0, 10.0),
                    radius: 3.0
                },
            ]
        );

        let dot = Mark {
            ink: Ink::Brush(Arc::new(vec![pos2(5.0, 5.0)])),
            ..m
        };
        assert_eq!(
            primitives(&dot),
            [Prim::Disc {
                center: pos2(5.0, 5.0),
                radius: 3.0
            }]
        );
    }

    #[test]
    fn shift_snaps_lines_to_fifteen_degrees_and_boxes_to_the_diagonal() {
        let o = pos2(0.0, 0.0);
        let p = snap_for(Shape::Line, o, pos2(100.0, 5.0));
        assert!((p.y).abs() < 1e-4 && (p.x - 100.0).abs() < 1e-4, "{p:?}");
        let p = snap_for(Shape::Arrow, o, pos2(100.0, 30.0));
        assert!((p.y / p.x - 15f32.to_radians().tan()).abs() < 1e-4, "{p:?}");
        let p = snap_for(Shape::Rectangle, o, pos2(100.0, 80.0));
        assert!((p.x - p.y).abs() < 1e-3, "a square: {p:?}");
        let p = snap_for(Shape::Rectangle, o, pos2(-100.0, 80.0));
        assert!((p.x + p.y).abs() < 1e-3, "a square up and left: {p:?}");
    }
}
