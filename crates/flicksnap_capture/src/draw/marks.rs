//! The marks drawn on the overlay: the finished ones, newest last, and
//! the one under the pointer.

use std::sync::Arc;

use egui::{pos2, Color32, Pos2};
use flicksnap_shared::geometry::ScreenPointF;

use super::geometry::snap_for;
use super::{DrawTool, Shape};

/// One mark, in virtual-desktop physical px. It stays where it was drawn
/// on the screen: moving or resizing the selection afterwards does not
/// move it, it only changes how much of it the selection shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub ink: Ink,
    pub color: Color32,
    /// The DPI scale of the monitor the drag began on: the mark's canvas
    /// unit in px, so it draws as big as draw-on-screen's would there.
    pub scale: f32,
}

/// What a mark is drawn from.
#[derive(Debug, Clone, PartialEq)]
pub enum Ink {
    /// A shape dragged from `from` to `to` — `to` after the Shift snap.
    Shape { shape: Shape, from: Pos2, to: Pos2 },
    /// A freehand stroke: the points it runs through, from the press to
    /// where the pointer let go. These are its geometry as they are — a
    /// round-capped line through them — so nothing is ever rebuilt.
    Brush(Arc<Vec<Pos2>>),
}

/// The mark being dragged out, with the raw pointer it was last given so
/// a Shift press or release mid-drag can re-snap it without a move.
#[derive(Debug, Clone, PartialEq)]
struct Drawing {
    mark: Mark,
    pointer: Pos2,
}

/// Every mark on the overlay, and the one being drawn.
///
/// The finished marks sit behind an `Arc` so the per-monitor hosts can
/// hold this frame's list without copying it on every pointer move; only
/// finishing or undoing a mark makes a new one.
#[derive(Debug, Clone, Default)]
pub struct Marks {
    done: Arc<Vec<Mark>>,
    drawing: Option<Drawing>,
}

/// How far, in canvas units, a shape's drag has to reach on either axis
/// to make one. A press-and-release with no drag leaves nothing behind.
/// The brush has no such floor: a click leaves its dot.
const MIN_EXTENT: f32 = 1.0;

/// How far apart, in canvas units, a brush stroke's points are kept. A
/// move that lands closer than this to the last point kept only moves the
/// stroke's end, so a slow hand adds no points and the end is always the
/// pointer.
const BRUSH_STEP: f32 = 2.0;

/// The most points one brush stroke keeps: a guard, far past any real
/// mark — 20 000 points two units apart run 40 000 units, some twenty
/// screens across — so that a pointer held down and moved for minutes
/// cannot grow a stroke without bound. A stroke that reaches it stops
/// growing; the release keeps it as it is.
const MAX_BRUSH_POINTS: usize = 20_000;

/// Whether `mark` is one to show and keep: a shape whose drag reaches
/// `MIN_EXTENT` on either axis, or any brush stroke.
fn reaches(mark: &Mark) -> bool {
    match &mark.ink {
        Ink::Shape {
            from,
            to,
            ..
        } => {
            let reach = (*to - *from).abs() / mark.scale;
            reach.x >= MIN_EXTENT || reach.y >= MIN_EXTENT
        }
        Ink::Brush(_) => true,
    }
}

/// Where a shape's free end goes for the pointer at `pointer`: there, or
/// snapped as the shape snaps under Shift.
fn aim(shape: Shape, from: Pos2, pointer: Pos2, snap: bool) -> Pos2 {
    if snap {
        snap_for(shape, from, pointer)
    } else {
        pointer
    }
}

/// Take the pointer at `p` into a brush stroke's points. The last point
/// is the stroke's end, which follows the pointer; once it has got `step`
/// from the point before it, it stays where it is and `p` becomes the new
/// end. The press itself is never the end, so it stays put.
///
/// A stroke of `MAX_BRUSH_POINTS` points is left as it is — and left
/// shared: the cap is checked before the points are made writable, since
/// a host still holding last frame's stroke would have them copied whole
/// for a move that changes nothing.
fn stroke_to(points: &mut Arc<Vec<Pos2>>, p: Pos2, step: f32) {
    if points.len() >= MAX_BRUSH_POINTS {
        return;
    }
    let points = Arc::make_mut(points);
    match points.len() {
        0 | 1 => points.push(p),
        n if points[n - 2].distance(points[n - 1]) < step => points[n - 1] = p,
        _ => points.push(p),
    }
}

impl Marks {
    /// The finished marks, oldest first.
    pub fn done(&self) -> &Arc<Vec<Mark>> {
        &self.done
    }

    /// The mark under the pointer, once the drag drawing it has gone far
    /// enough to make one — the same test `finish` keeps it by, so what
    /// is previewed is exactly what will be kept. Before that, nothing:
    /// the moment after the press, or with the pointer back where the drag
    /// began, a shape's two ends meet and it has no direction to take.
    pub fn drawing(&self) -> Option<Mark> {
        self.drawing
            .as_ref()
            .map(|d| d.mark.clone())
            .filter(reaches)
    }

    pub fn is_drawing(&self) -> bool {
        self.drawing.is_some()
    }

    /// Whether UNDO has a mark to take back.
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    /// Start a `tool` mark at `at`.
    pub fn begin(&mut self, tool: DrawTool, at: ScreenPointF, color: Color32, scale: f32) {
        let at = pos2(at.x, at.y);
        let ink = match tool.shape() {
            Some(shape) => Ink::Shape {
                shape,
                from: at,
                to: at,
            },
            None => Ink::Brush(Arc::new(vec![at])),
        };
        self.drawing = Some(Drawing {
            mark: Mark {
                ink,
                color,
                scale,
            },
            pointer: at,
        });
    }

    /// The pointer moved to `pointer`: a shape's free end follows it,
    /// snapped as the shape snaps under Shift when `snap` is set; a brush
    /// stroke runs on to it.
    pub fn drag_to(&mut self, pointer: ScreenPointF, snap: bool) {
        let Some(d) = &mut self.drawing else {
            return;
        };
        d.pointer = pos2(pointer.x, pointer.y);
        match &mut d.mark.ink {
            Ink::Shape {
                shape,
                from,
                to,
            } => *to = aim(*shape, *from, d.pointer, snap),
            Ink::Brush(points) => stroke_to(points, d.pointer, BRUSH_STEP * d.mark.scale),
        }
    }

    /// Shift went down or up mid-drag: re-snap a shape from the pointer it
    /// was last dragged to, as draw-on-screen replays a move. The brush
    /// does not snap.
    pub fn resnap(&mut self, snap: bool) {
        if let Some(d) = &mut self.drawing {
            if let Ink::Shape {
                shape,
                from,
                to,
            } = &mut d.mark.ink
            {
                *to = aim(*shape, *from, d.pointer, snap);
            }
        }
    }

    /// End the drag: the mark joins the finished ones unless it is a shape
    /// whose drag never went anywhere.
    pub fn finish(&mut self) {
        if let Some(mark) = self
            .drawing
            .take()
            .map(|d| d.mark)
            .filter(reaches)
        {
            Arc::make_mut(&mut self.done).push(mark);
        }
    }

    /// Take back the newest finished mark. Returns whether there was one.
    pub fn undo(&mut self) -> bool {
        if self.done.is_empty() {
            return false;
        }
        Arc::make_mut(&mut self.done).pop();
        true
    }

    /// Drop every mark, the one being drawn included.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::geometry::{primitives, Prim};

    fn p(x: f32, y: f32) -> ScreenPointF {
        ScreenPointF::new(x, y)
    }

    fn line_end(mark: &Mark) -> Pos2 {
        match mark.ink {
            Ink::Shape {
                to,
                ..
            } => to,
            Ink::Brush(_) => panic!("a shape"),
        }
    }

    #[test]
    fn a_drag_makes_a_mark_and_undo_takes_it_back() {
        let mut m = Marks::default();
        m.begin(DrawTool::Line, p(10.0, 10.0), Color32::RED, 1.0);
        m.drag_to(p(60.0, 40.0), false);
        assert!(m.is_drawing() && !m.can_undo());
        m.finish();
        assert!(!m.is_drawing() && m.can_undo());
        assert_eq!(line_end(&m.done()[0]), pos2(60.0, 40.0));
        assert!(m.undo());
        assert!(!m.can_undo() && !m.undo());
    }

    #[test]
    fn a_click_without_a_drag_leaves_no_shape() {
        let mut m = Marks::default();
        m.begin(DrawTool::Rectangle, p(10.0, 10.0), Color32::RED, 2.0);
        m.drag_to(p(11.5, 11.5), false);
        m.finish();
        assert!(!m.can_undo(), "under one unit at 200 %");
    }

    /// A press previews nothing until the drag reaches a mark, and nothing
    /// again with the pointer brought back to where it began: an arrow
    /// whose ends meet has no direction, and its geometry no finite head.
    #[test]
    fn a_drag_with_its_ends_together_previews_nothing() {
        let finite = |m: &Marks| {
            m.drawing().is_none_or(|mark| {
                primitives(&mark)
                    .iter()
                    .all(|prim| match prim {
                        Prim::Convex(points) => points.iter().all(|p| p.is_finite()),
                        Prim::Polyline {
                            points,
                            width,
                        } => points.iter().all(|p| p.is_finite()) && width.is_finite(),
                        Prim::Segment {
                            a,
                            b,
                            width,
                        } => a.is_finite() && b.is_finite() && width.is_finite(),
                        Prim::Disc {
                            center,
                            radius,
                        } => center.is_finite() && radius.is_finite(),
                        Prim::RectStroke {
                            rect,
                            width,
                        } => rect.is_finite() && width.is_finite(),
                    })
            })
        };
        let mut m = Marks::default();
        m.begin(DrawTool::Arrow, p(10.0, 10.0), Color32::RED, 1.25);
        assert!(m.is_drawing() && m.drawing().is_none(), "just pressed");
        m.drag_to(p(80.0, 30.0), false);
        assert!(m.drawing().is_some() && finite(&m));
        m.drag_to(p(10.0, 10.0), false);
        assert!(m.drawing().is_none(), "back at the start");
        m.drag_to(p(11.0, 10.5), false);
        assert!(m.drawing().is_none(), "under one unit at 125 %");
        m.drag_to(p(11.25, 10.0), false);
        assert!(m.drawing().is_some() && finite(&m), "one unit: the shortest arrow");
        m.drag_to(p(10.0, 10.0), false);
        m.finish();
        assert!(!m.can_undo(), "released at the start: nothing kept");
    }

    /// Shift pressed mid-drag snaps the mark where the pointer already is;
    /// released, it lets go again.
    #[test]
    fn shift_mid_drag_resnaps_without_a_move() {
        let mut m = Marks::default();
        m.begin(DrawTool::Line, p(0.0, 0.0), Color32::RED, 1.0);
        m.drag_to(p(100.0, 4.0), false);
        m.resnap(true);
        assert_eq!(
            m.drawing()
                .map(|d| line_end(&d).y.abs() < 1e-4),
            Some(true)
        );
        m.resnap(false);
        assert_eq!(m.drawing().map(|d| line_end(&d)), Some(pos2(100.0, 4.0)));
    }

    #[test]
    fn clear_drops_the_finished_marks_and_the_one_being_drawn() {
        let mut m = Marks::default();
        m.begin(DrawTool::Arrow, p(0.0, 0.0), Color32::RED, 1.0);
        m.drag_to(p(50.0, 0.0), false);
        m.finish();
        m.begin(DrawTool::Arrow, p(0.0, 0.0), Color32::RED, 1.0);
        m.clear();
        assert!(!m.can_undo() && !m.is_drawing());
    }

    /// The brush keeps a point every `BRUSH_STEP` units and lets the moves
    /// between only carry its end, so the stroke always ends on the
    /// pointer. Shift does nothing to it. A whole gesture is one mark, a
    /// click alone included — its dot.
    #[test]
    fn the_brush_thins_its_points_and_ends_on_the_pointer() {
        let points = |m: &Marks| match m.drawing().map(|d| d.ink) {
            Some(Ink::Brush(points)) => points.to_vec(),
            other => panic!("a brush stroke: {other:?}"),
        };
        let mut m = Marks::default();
        m.begin(DrawTool::Brush, p(10.0, 10.0), Color32::RED, 1.5);
        assert_eq!(points(&m), [pos2(10.0, 10.0)], "a click previews its dot");
        m.drag_to(p(11.0, 10.0), false);
        m.drag_to(p(12.0, 10.0), false);
        assert_eq!(points(&m), [pos2(10.0, 10.0), pos2(12.0, 10.0)], "under three px: the end moves");
        m.drag_to(p(14.0, 10.0), true);
        m.drag_to(p(15.0, 10.0), false);
        assert_eq!(points(&m), [pos2(10.0, 10.0), pos2(14.0, 10.0), pos2(15.0, 10.0)]);
        m.finish();
        assert_eq!(m.done().len(), 1, "one gesture, one mark");

        m.begin(DrawTool::Brush, p(5.0, 5.0), Color32::RED, 1.0);
        m.finish();
        assert_eq!(m.done().len(), 2, "a click keeps its dot");
        assert!(m.undo() && m.undo() && !m.can_undo());
    }

    /// A stroke stops growing at `MAX_BRUSH_POINTS`, its last point where
    /// it was, however long the pointer goes on — and a move past the cap
    /// leaves the points shared with whoever holds them, uncopied.
    #[test]
    fn a_brush_stroke_stops_growing_at_its_cap() {
        let mut points = Arc::new(vec![pos2(0.0, 0.0)]);
        for i in 1..=MAX_BRUSH_POINTS + 500 {
            stroke_to(&mut points, pos2(3.0 * i as f32, 0.0), 2.0);
        }
        assert_eq!(points.len(), MAX_BRUSH_POINTS);
        let held = points.clone();
        stroke_to(&mut points, pos2(-50.0, -50.0), 2.0);
        assert!(Arc::ptr_eq(&points, &held), "no copy for a move that changes nothing");
    }
}
