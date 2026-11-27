//! A pixel loupe over the frozen screenshot.

use std::sync::{Arc, LazyLock};

use egui::{pos2, vec2, Color32, Mesh, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::system::CapturedDesktop;
use crate::ui::components::{InputCtx, Local};
use crate::ui::shared::{sample_bgra, UiMonitor};
use flicksnap_shared::geometry::{to_screen_point, ScreenPoint};

const RADIUS: i32 = 5;
const CELLS: usize = (2 * RADIUS + 1) as usize;
const HALF: f32 = CELLS as f32 / 2.0;
const DIAMETER: f32 = 120.0;
const GAP: Vec2 = vec2(11.0, 5.0);

static CELL_SHAPES: LazyLock<Vec<(usize, Vec<Pos2>)>> = LazyLock::new(cell_shapes);

#[derive(Clone, PartialEq)]
pub struct LoupeInputs {
    pointer: Pos2,
    colors: Arc<[Color32; CELLS * CELLS]>,
}

pub fn inputs(index: usize, monitor: &UiMonitor, c: &InputCtx<'_>, desktop: Option<&CapturedDesktop>) -> Option<LoupeInputs> {
    let i = c.input;
    if !i.loupe_visible || !i.overlays_visible || i.ocr.active() || i.scroll_pick_mode || c.cursor_index != Some(index) {
        return None;
    }
    let desktop = desktop?;
    let point = to_screen_point(i.virtual_cursor);
    let center = pixel_color(desktop, point)?;
    let colors = std::array::from_fn(|n| {
        let point = ScreenPoint::new(point.x + (n % CELLS) as i32 - RADIUS, point.y + (n / CELLS) as i32 - RADIUS);
        if monitor.bounds.contains(point) {
            pixel_color(desktop, point).unwrap_or(center)
        } else {
            center
        }
    });
    Some(LoupeInputs {
        pointer: Local::of(monitor).pos(i.virtual_cursor.x, i.virtual_cursor.y),
        colors: Arc::new(colors),
    })
}

fn pixel_color(desktop: &CapturedDesktop, point: ScreenPoint) -> Option<Color32> {
    let [b, g, r, _] = sample_bgra(desktop, point)?;
    Some(Color32::from_rgb(r, g, b))
}

fn placement(screen: Rect, pointer: Pos2) -> Rect {
    let screen = screen.shrink(4.0);
    // Keep the same diameter across the monitor, including when the lens
    // flips. On a small viewport, size it so one side always fits.
    let size = DIAMETER
        .min(screen.width() / 2.0 - GAP.x)
        .min(screen.height() / 2.0 - GAP.y)
        .max(0.0);
    let place = |pointer: f32, max: f32, gap: f32| {
        let after = pointer + gap;
        if after + size <= max {
            after
        } else {
            pointer - gap - size
        }
    };
    Rect::from_min_size(
        pos2(place(pointer.x, screen.right(), GAP.x), place(pointer.y, screen.bottom(), GAP.y)),
        vec2(size, size),
    )
}

pub fn show(painter: &Painter, inputs: &LoupeInputs, screen: Rect) {
    let bounds = placement(screen, inputs.pointer);
    let center = bounds.center();
    let radius = bounds.width() / 2.0;
    let cell = bounds.width() / CELLS as f32;

    // One colored mesh. The circular clip is baked into the cell geometry
    // so cursor movement doesn't update the texture atlas.
    let mut mesh = Mesh::default();
    for (index, polygon) in CELL_SHAPES.iter() {
        let base = mesh.vertices.len() as u32;
        for &p in polygon {
            mesh.colored_vertex(center + p.to_vec2() * cell, inputs.colors[*index]);
        }
        for i in 1..polygon.len() as u32 - 1 {
            mesh.add_triangle(base, base + i, base + i + 1);
        }
    }
    painter.add(mesh);

    let grid = Stroke::new(0.35, Color32::BLACK);
    for i in 1..CELLS {
        let offset = (i as f32 - HALF) * cell;
        let extent = (radius * radius - offset * offset)
            .max(0.0)
            .sqrt();
        painter.line_segment([center + vec2(offset, -extent), center + vec2(offset, extent)], grid);
        painter.line_segment([center + vec2(-extent, offset), center + vec2(extent, offset)], grid);
    }

    painter.circle_stroke(center, radius, Stroke::new(1.0, Color32::WHITE));
    let target = Rect::from_center_size(center, vec2(cell, cell));
    painter.rect_stroke(target, 1.0, Stroke::new(3.0, Color32::BLACK), StrokeKind::Middle);
    painter.rect_stroke(target, 1.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Middle);
}

fn cell_shapes() -> Vec<(usize, Vec<Pos2>)> {
    let disk: Vec<_> = (0..64)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 64.0;
            pos2(a.cos() * HALF, a.sin() * HALF)
        })
        .collect();

    (0..CELLS * CELLS)
        .filter_map(|i| {
            let left = (i % CELLS) as f32 - HALF;
            let top = (i / CELLS) as f32 - HALF;
            let points = clip(&disk, |p| p.x - left);
            let points = clip(&points, |p| left + 1.0 - p.x);
            let points = clip(&points, |p| p.y - top);
            let points = clip(&points, |p| top + 1.0 - p.y);
            (points.len() >= 3).then_some((i, points))
        })
        .collect()
}

/// Clip a convex polygon to the half-plane with nonnegative distance.
fn clip(points: &[Pos2], distance: impl Fn(Pos2) -> f32) -> Vec<Pos2> {
    let Some(&last) = points.last() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    let mut previous = last;
    let mut previous_distance = distance(last);
    for &point in points {
        let d = distance(point);
        if (d >= 0.0) != (previous_distance >= 0.0) {
            result.push(previous + (point - previous) * (previous_distance / (previous_distance - d)));
        }
        if d >= 0.0 {
            result.push(point);
        }
        previous = point;
        previous_distance = d;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::InteractionState;
    use flicksnap_shared::geometry::{RectExt, ScreenPointF, ScreenRect};

    #[test]
    fn samples_unmodified_colors_at_negative_desktop_coordinates() {
        let monitor = UiMonitor {
            bounds: ScreenRect::from_xy_size(-100, 20, 2, 1),
            dpi_scale: 2.0,
            is_primary: false,
        };
        let desktop = CapturedDesktop {
            bgra: vec![11, 23, 100, 0, 21, 37, 180, 0],
            bounds: monitor.bounds,
            width: 2,
            height: 1,
            cursor: None,
        };
        let input = InteractionState {
            virtual_cursor: ScreenPointF::new(-99.0, 20.0),
            loupe_visible: true,
            ..InteractionState::new()
        };
        let context = InputCtx {
            input: &input,
            monitors: &[monitor],
            cursor_index: Some(0),
            accent: Color32::WHITE,
            hovered_monitor_name: None,
        };
        let loupe = inputs(0, &monitor, &context, Some(&desktop)).unwrap();
        let target = RADIUS as usize * (CELLS + 1);
        assert_eq!(loupe.pointer, pos2(0.5, 0.0));
        assert_eq!(loupe.colors[target], Color32::from_rgb(180, 37, 21));
        assert_eq!(loupe.colors[target - 1], Color32::from_rgb(100, 23, 11));
        assert_eq!(loupe.colors[target + 1], loupe.colors[target]);
        assert!(inputs(1, &monitor, &context, Some(&desktop)).is_none());
    }

    #[test]
    fn mirrors_the_cursor_offset_in_all_four_quadrants() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        for (pointer, expected_offset) in [
            (pos2(200.0, 150.0), vec2(71.0, 65.0)),
            (screen.left_top(), vec2(71.0, 65.0)),
            (screen.right_top(), vec2(-71.0, 65.0)),
            (screen.left_bottom(), vec2(71.0, -65.0)),
            (screen.right_bottom(), vec2(-71.0, -65.0)),
        ] {
            let lens = placement(screen, pointer);
            assert_eq!(lens.width(), DIAMETER);
            assert!(screen.contains_rect(lens));
            assert_eq!(lens.center() - pointer, expected_offset);
        }
    }

    #[test]
    fn keeps_its_size_and_distance_while_flipping_in_a_small_viewport() {
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(210.0, 200.0));
        let initial = placement(screen, screen.left_top());
        let distance = initial.center().distance(screen.left_top());
        for x in [0.0, 104.0, 105.0, 106.0, 210.0] {
            for y in [0.0, 99.0, 100.0, 101.0, 200.0] {
                let pointer = pos2(x, y);
                let lens = placement(screen, pointer);
                assert!(screen.contains_rect(lens));
                assert_eq!(lens.size(), initial.size());
                assert_eq!(lens.center().distance(pointer), distance);
            }
        }
    }
}
