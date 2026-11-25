use flicksnap_desktop::{WindowDescription, WindowFilter};
use flicksnap_shared::geometry::{RectExt, ScreenRect};

use super::mac_corners::CgBounds;
use super::{mac_corners, MonitorInfo, WindowTarget};

pub fn target(monitors: &[MonitorInfo], rounded: bool) -> Option<WindowTarget> {
    let pid = objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()?
        .processIdentifier();
    let filter = WindowFilter {
        layers: flicksnap_desktop::NORMAL_WINDOW_LAYERS,
        exclude_own_process: true,
        min_size: 0.0,
    };
    flicksnap_desktop::window_list(&filter)
        .into_iter()
        .filter(|window| window.pid == pid)
        .find_map(|window| window_target(window, monitors, rounded))
}

fn window_target(window: WindowDescription, monitors: &[MonitorInfo], rounded: bool) -> Option<WindowTarget> {
    let cg_rect = window.bounds;
    let center_x = cg_rect.origin.x + cg_rect.size.width / 2.0;
    let center_y = cg_rect.origin.y + cg_rect.size.height / 2.0;

    let (phys_x, phys_y, phys_w, phys_h, scale) = if let Some(m) = find_monitor_for_cg_point(center_x, center_y, monitors) {
        let ox = m.logical_origin.x;
        let oy = m.logical_origin.y;
        let s = m.scale_factor as f64;
        (
            m.bounds.min_x() + ((cg_rect.origin.x - ox) * s).round() as i32,
            m.bounds.min_y() + ((cg_rect.origin.y - oy) * s).round() as i32,
            (cg_rect.size.width * s).round() as i32,
            (cg_rect.size.height * s).round() as i32,
            m.scale_factor,
        )
    } else {
        let scale = display_scale_at_logical_point(center_x, center_y);
        let s = scale as f64;
        (
            (cg_rect.origin.x * s).round() as i32,
            (cg_rect.origin.y * s).round() as i32,
            (cg_rect.size.width * s).round() as i32,
            (cg_rect.size.height * s).round() as i32,
            scale,
        )
    };

    if phys_w <= 0 || phys_h <= 0 {
        return None;
    }
    let cg_bounds = CgBounds {
        x: cg_rect.origin.x,
        y: cg_rect.origin.y,
        w: cg_rect.size.width,
        h: cg_rect.size.height,
    };
    Some(WindowTarget {
        rect: ScreenRect::from_xy_size(phys_x, phys_y, phys_w, phys_h),
        corner_radius: if rounded {
            mac_corners::probe_corner_radius(window.id, cg_bounds, monitors)
                .unwrap_or_else(|| mac_corners::fallback_radius_points() * scale)
        } else {
            0.0
        },
    })
}

fn find_monitor_for_cg_point(x: f64, y: f64, monitors: &[MonitorInfo]) -> Option<&MonitorInfo> {
    monitors.iter().find(|m| {
        let ox = m.logical_origin.x;
        let oy = m.logical_origin.y;
        let lw = m.bounds.width() as f64 / m.scale_factor as f64;
        let lh = m.bounds.height() as f64 / m.scale_factor as f64;
        x >= ox && x < ox + lw && y >= oy && y < oy + lh
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the scale factor of the display whose logical bounds contain the
/// given CG point.
fn display_scale_at_logical_point(x: f64, y: f64) -> f32 {
    for d in flicksnap_desktop::active_displays().unwrap_or_default() {
        let b = d.bounds;
        if x >= b.origin.x && x < b.origin.x + b.size.width && y >= b.origin.y && y < b.origin.y + b.size.height {
            // The mode's pixel width is the true physical pixel count;
            // CGDisplayPixelsWide reports points on Retina displays.
            let logical_w = b.size.width as f32;
            if logical_w > 0.0 {
                if let Some(mode) = d.mode {
                    return mode.pixel_width as f32 / logical_w;
                }
            }
        }
    }
    // Fallback: assume Retina 2×.
    2.0
}
