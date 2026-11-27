//! The marks drawn on the overlay, painted whole on every monitor, each
//! cut only to its own monitor. A mark is drawn anywhere on the screen,
//! inside the selection or out, and shows where it was drawn; the cut to
//! the selection is the exported image's, made when it is rendered.

use std::sync::Arc;

use egui::{Color32, Painter, Shape, Stroke, StrokeKind};

use crate::draw::geometry::{primitives, Prim};
use crate::draw::Mark;
use crate::ui::components::{InputCtx, Local};
use crate::ui::shared::UiMonitor;
use flicksnap_shared::geometry::ScreenRect;

/// One host's share of the marks: all of them, and this monitor they are
/// cut to.
#[derive(Clone, PartialEq)]
pub struct MarksInputs {
    pub done: Arc<Vec<Mark>>,
    pub drawing: Option<Mark>,
    /// This monitor's bounds, in virtual-desktop physical px.
    pub clip: ScreenRect,
}

/// The marks, on every monitor, while there are any — whether or not the
/// selection reaches it. They are the capture's content rather than its
/// chrome, so Q — which hides the overlay's chrome to show the capture as
/// it will be — leaves them on screen.
pub fn inputs(_index: usize, monitor: &UiMonitor, c: &InputCtx<'_>) -> Option<MarksInputs> {
    let i = c.input;
    if !i.captured || (i.marks.done().is_empty() && !i.marks.is_drawing()) {
        return None;
    }
    Some(MarksInputs {
        done: i.marks.done().clone(),
        drawing: i.marks.drawing(),
        clip: monitor.bounds,
    })
}

/// Paint every mark, oldest first and the one being drawn on top, cut to
/// this monitor.
pub fn show(painter: &Painter, m: &MarksInputs, monitor: &UiMonitor) {
    let local = Local::of(monitor);
    let painter = painter.with_clip_rect(local.rect(m.clip.to_f32()));
    for mark in m.done.iter().chain(m.drawing.as_ref()) {
        for prim in primitives(mark) {
            painter.add(shape(&prim, mark.color, &local));
        }
    }
}

/// One primitive as an egui shape in this host's points.
fn shape(prim: &Prim, color: Color32, local: &Local) -> Shape {
    let at = |p: egui::Pos2| local.pos(p.x, p.y);
    let len = |v: f32| v / local.ppp;
    match prim {
        Prim::Convex(points) => Shape::convex_polygon(points.iter().map(|&p| at(p)).collect(), color, Stroke::NONE),
        Prim::Segment {
            a,
            b,
            width,
        } => Shape::line_segment([at(*a), at(*b)], Stroke::new(len(*width), color)),
        Prim::Disc {
            center,
            radius,
        } => Shape::circle_filled(at(*center), len(*radius), color),
        Prim::RectStroke {
            rect,
            width,
        } => Shape::rect_stroke(
            egui::Rect::from_min_max(at(rect.min), at(rect.max)),
            0.0,
            Stroke::new(len(*width), color),
            StrokeKind::Middle,
        ),
        Prim::Polyline {
            points,
            width,
        } => Shape::line(points.iter().map(|&p| at(p)).collect(), Stroke::new(len(*width), color)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawTool;
    use crate::interaction::InteractionState;
    use crate::ui::components::InputCtx;
    use flicksnap_shared::geometry::{RectExt, ScreenPointF};

    fn monitor(x: i32) -> UiMonitor {
        UiMonitor {
            bounds: ScreenRect::from_xy_size(x, 0, 1920, 1080),
            dpi_scale: 1.0,
            is_primary: x == 0,
        }
    }

    fn ctx<'a>(input: &'a InteractionState, monitors: &'a [UiMonitor]) -> InputCtx<'a> {
        InputCtx {
            input,
            monitors,
            cursor_index: Some(0),
            accent: Color32::BLUE,
            hovered_monitor_name: None,
        }
    }

    fn captured_with_a_mark(selection: ScreenRect) -> InteractionState {
        let mut input = InteractionState::new();
        input.captured = true;
        input.selection = Some(selection);
        input
            .marks
            .begin(DrawTool::Line, ScreenPointF::new(1800.0, 500.0), Color32::RED, 1.0);
        input
            .marks
            .drag_to(ScreenPointF::new(2100.0, 500.0), false);
        input.marks.finish();
        input
    }

    /// Every monitor draws the marks, each cut to itself — the one the
    /// selection never touches as well, so a mark drawn there shows.
    #[test]
    fn every_monitor_draws_the_marks_cut_to_itself() {
        let monitors = [monitor(0), monitor(1920), monitor(3840)];
        let input = captured_with_a_mark(ScreenRect::from_xy_size(100, 100, 300, 300));
        let c = ctx(&input, &monitors);
        for (i, mon) in monitors.iter().enumerate() {
            let m = inputs(i, mon, &c).expect("every monitor");
            assert_eq!(m.clip, mon.bounds);
            assert_eq!(m.done.len(), 1);
        }
    }

    #[test]
    fn nothing_to_draw_is_no_input() {
        let monitors = [monitor(0)];
        let mut input = InteractionState::new();
        input.captured = true;
        input.selection = Some(ScreenRect::from_xy_size(100, 100, 300, 300));
        assert!(inputs(0, &monitors[0], &ctx(&input, &monitors)).is_none());

        let mut input = captured_with_a_mark(ScreenRect::from_xy_size(100, 100, 300, 300));
        input.marks.clear();
        assert!(inputs(0, &monitors[0], &ctx(&input, &monitors)).is_none());
    }

    /// A mark is cut to its monitor, not to the selection: one drawn
    /// wholly outside the selection still paints, clipped only at the
    /// monitor's edge.
    #[test]
    fn a_mark_outside_the_selection_paints_cut_to_the_monitor() {
        let mon = monitor(0);
        let input = captured_with_a_mark(ScreenRect::from_xy_size(100, 100, 300, 300));
        let monitors = [mon];
        let m = inputs(0, &mon, &ctx(&input, &monitors)).expect("marks");
        let egui_ctx = egui::Context::default();
        let output = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui
                .ctx()
                .layer_painter(egui::LayerId::background());
            show(&painter, &m, &mon);
        });
        let clip = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1920.0, 1080.0));
        assert!(!output.shapes.is_empty());
        assert!(
            output
                .shapes
                .iter()
                .all(|c| c.clip_rect == clip),
            "{:?}",
            output.shapes
        );
        output.drop_without_applying_deltas();
    }
}
