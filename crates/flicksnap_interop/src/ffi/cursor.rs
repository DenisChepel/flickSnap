//! The cursor position (WP-WINDOW; README section 4.3).

use crate::error::Result;
use crate::ffi::types::FlickSnapPoint;
use crate::{panic, util};

/// The cursor position right now, in the space `flicksnap_screens_*` reports screens in: physical
/// virtual-desktop pixels on Windows (`GetCursorPos`), CoreGraphics global points with a top-left
/// origin on macOS (the location of a fresh `CGEvent`, floored). Needs no window and no primary
/// screen. `out_point` is written only on `FLICKSNAP_OK`.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_S_NONE` when the OS will not say (`GetCursorPos` failed, as on a
/// secure desktop, or the window server made no event); `FLICKSNAP_E_INVALID_ARG` for a null
/// `out_point`; `FLICKSNAP_E_UNSUPPORTED` on a platform without a cursor backend.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_cursor_position(out_point: *mut FlickSnapPoint) -> i32 {
    panic::guard("flicksnap_cursor_position", || cursor_position(out_point))
}

unsafe fn cursor_position(out_point: *mut FlickSnapPoint) -> Result {
    let out_point = util::out(out_point, "out_point")?;
    #[cfg(windows)]
    let point = flicksnap_desktop::cursor_position().map(|p| (p.x, p.y));
    #[cfg(target_os = "macos")]
    let point = flicksnap_desktop::cursor_location().map(|p| (p.x.floor() as i32, p.y.floor() as i32));
    #[cfg(any(windows, target_os = "macos"))]
    {
        match point {
            Some((x, y)) => {
                *out_point = FlickSnapPoint {
                    x,
                    y,
                };
                Ok(crate::ffi::status::FLICKSNAP_OK)
            }
            None => Ok(crate::ffi::status::FLICKSNAP_S_NONE),
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = out_point;
        Err(crate::error::Error::unsupported("there is no cursor backend on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::status::{FlickSnapInteropStatus, FLICKSNAP_OK, FLICKSNAP_S_NONE};

    #[test]
    fn a_null_out_is_invalid_arg() {
        assert_eq!(
            unsafe { flicksnap_cursor_position(std::ptr::null_mut()) },
            FlickSnapInteropStatus::InvalidArg as i32
        );
    }

    #[test]
    fn the_position_is_written_only_on_ok() {
        let mut point = FlickSnapPoint {
            x: i32::MIN,
            y: i32::MIN,
        };
        let status = unsafe { flicksnap_cursor_position(&mut point) };
        if !cfg!(any(windows, target_os = "macos")) {
            assert_eq!(status, FlickSnapInteropStatus::Unsupported as i32);
            return;
        }
        if status == FLICKSNAP_S_NONE {
            assert_eq!(
                point,
                FlickSnapPoint {
                    x: i32::MIN,
                    y: i32::MIN
                }
            );
            return;
        }
        assert_eq!(status, FLICKSNAP_OK);
        assert_ne!(
            point,
            FlickSnapPoint {
                x: i32::MIN,
                y: i32::MIN
            }
        );
    }
}
