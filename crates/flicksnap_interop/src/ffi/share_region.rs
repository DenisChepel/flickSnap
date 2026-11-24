//! The share-region rectangle rules (README section 4.10) over `flicksnap_shared::share_region`, the
//! one implementation the region helper (flicksnap_share_region) applies to every region it mirrors.
//! Rects are in capture space: physical pixels in virtual-desktop coordinates on Windows,
//! CoreGraphics points on macOS; x and y may be negative.

use crate::ffi::status::FLICKSNAP_OK;
use crate::ffi::types::FlickSnapRect;
use crate::{panic, util};
use flicksnap_shared::geometry::{RectExt, ScreenRect};
use flicksnap_shared::share_region::{self, Handle};

/// Which part of the region a drag moves: the corners, the edge midpoints, or the body (a move).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapShareRegionHandle {
    TopLeft = 0,
    TopRight = 1,
    BottomLeft = 2,
    BottomRight = 3,
    Left = 4,
    Top = 5,
    Right = 6,
    Bottom = 7,
    Body = 8,
}

crate::convert::ffi_enum!(FlickSnapShareRegionHandle: i32 {
    TopLeft, TopRight, BottomLeft, BottomRight, Left, Top, Right, Bottom, Body
});

impl From<FlickSnapShareRegionHandle> for Handle {
    fn from(handle: FlickSnapShareRegionHandle) -> Self {
        match handle {
            FlickSnapShareRegionHandle::TopLeft => Handle::TopLeft,
            FlickSnapShareRegionHandle::TopRight => Handle::TopRight,
            FlickSnapShareRegionHandle::BottomLeft => Handle::BottomLeft,
            FlickSnapShareRegionHandle::BottomRight => Handle::BottomRight,
            FlickSnapShareRegionHandle::Left => Handle::Left,
            FlickSnapShareRegionHandle::Top => Handle::Top,
            FlickSnapShareRegionHandle::Right => Handle::Right,
            FlickSnapShareRegionHandle::Bottom => Handle::Bottom,
            FlickSnapShareRegionHandle::Body => Handle::Body,
        }
    }
}

/// The region the helper would mirror for `rect`: each side floored at the minimum region size
/// (`ShareRegionGeometry.MinRegion`) and then rounded down to even, the origin untouched.
/// Idempotent.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for a null `rect` or `out`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_share_region_normalize(rect: *const FlickSnapRect, out: *mut FlickSnapRect) -> i32 {
    panic::guard("flicksnap_share_region_normalize", || {
        let rect = util::input(rect, "rect")?;
        let out = util::out(out, "out")?;
        *out = to_ffi(share_region::normalize(from_ffi(rect)));
        Ok(FLICKSNAP_OK)
    })
}

/// The region after dragging `handle` (a `FlickSnapShareRegionHandle`) by (`dx`, `dy`) from `anchor`,
/// the region as the drag began. The body moves the region unchanged in size; any other handle
/// moves only its own edges, swaps an edge that crosses its opposite (and which one is moving), and
/// normalizes the result by pushing back the moving edge, so the anchored edge stays pinned.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for a null `anchor` or `out`, or an unknown `handle`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_share_region_apply_drag(
    anchor: *const FlickSnapRect,
    handle: i32,
    dx: i32,
    dy: i32,
    out: *mut FlickSnapRect,
) -> i32 {
    panic::guard("flicksnap_share_region_apply_drag", || {
        let anchor = util::input(anchor, "anchor")?;
        let handle = FlickSnapShareRegionHandle::from_raw(handle)?;
        let out = util::out(out, "out")?;
        *out = to_ffi(share_region::apply_drag(from_ffi(anchor), handle.into(), dx, dy));
        Ok(FLICKSNAP_OK)
    })
}

fn from_ffi(r: &FlickSnapRect) -> ScreenRect {
    ScreenRect::from_xy_size(r.x, r.y, r.width, r.height)
}

fn to_ffi(r: ScreenRect) -> FlickSnapRect {
    FlickSnapRect {
        x: r.origin.x,
        y: r.origin.y,
        width: r.size.width,
        height: r.size.height,
    }
}
