//! Screen pixels and the screen grab (README section 4.2), a thin layer over
//! `flicksnap_desktop::grab`, the grab the capture overlay and the scrolling driver use
//! too. Coordinates are the `flicksnap_screens_*` space. On macOS both exports answer
//! `FLICKSNAP_E_PERMISSION` first when Screen Recording is missing (`flicksnap_platform::permissions`, the
//! same check every other reader makes): an unpermitted window-list image succeeds and shows the
//! desktop picture instead of the windows, so it would silently read the wrong pixels.

use flicksnap_desktop::grab::{self, GrabError, Options};
use flicksnap_shared::geometry::ScreenPoint;

use crate::error::{Error, Result};
use crate::ffi::screens::to_rect;
use crate::ffi::status::{FLICKSNAP_OK, FLICKSNAP_S_NONE};
use crate::ffi::types::FlickSnapRect;
use crate::{panic, util};

/// A grabbed screen image (32 bytes). The pixels are mode 2: owned by this struct and freed by
/// `flicksnap_screen_image_free`.
#[repr(C)]
#[derive(Debug)]
pub struct FlickSnapScreenImage {
    /// BGRA8, top-down, alpha forced to 255.
    pub pixels: *mut u8,
    pub len: usize,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub _reserved: u32,
}

/// The colour of one screen pixel as `0x00RRGGBB` (the pixel under the point; on a Retina display
/// the top-left of the point's pixels). `out_rgb` is written only on `FLICKSNAP_OK`.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_S_NONE` when the point is on no screen; `FLICKSNAP_E_INVALID_ARG` for a
/// null `out_rgb`; `FLICKSNAP_E_PERMISSION` (macOS Screen Recording); `FLICKSNAP_E_OS`;
/// `FLICKSNAP_E_UNSUPPORTED` on a platform without a screen grab.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_screen_read_pixel(x: i32, y: i32, out_rgb: *mut u32) -> i32 {
    panic::guard("flicksnap_screen_read_pixel", || read_pixel(x, y, out_rgb))
}

unsafe fn read_pixel(x: i32, y: i32, out_rgb: *mut u32) -> Result {
    let out_rgb = util::out(out_rgb, "out_rgb")?;
    require_screen_recording()?;
    match grab::pixel(ScreenPoint::new(x, y)).map_err(grab_error)? {
        Some(rgb) => {
            *out_rgb = rgb;
            Ok(FLICKSNAP_OK)
        }
        None => Ok(FLICKSNAP_S_NONE),
    }
}

/// Grabs `rect` of the screen (layered windows included, alpha forced to 255) into an owned image;
/// free it with `flicksnap_screen_image_free`. On macOS the image has the display's backing pixels (a
/// 16x16 point rect is 32x32 on a Retina display); on Windows it is the rect's physical pixels.
/// `out_image` is written only on `FLICKSNAP_OK`.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for a null `rect` or `out_image` or a rect without
/// area; `FLICKSNAP_E_PERMISSION` (macOS Screen Recording); `FLICKSNAP_E_CAPABILITY` (CoreGraphics handed
/// back a pixel format this does not convert); `FLICKSNAP_E_OS`; `FLICKSNAP_E_UNSUPPORTED` on a platform
/// without a screen grab.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_screen_grab(rect: *const FlickSnapRect, out_image: *mut FlickSnapScreenImage) -> i32 {
    panic::guard("flicksnap_screen_grab", || grab_rect(rect, out_image))
}

unsafe fn grab_rect(rect: *const FlickSnapRect, out_image: *mut FlickSnapScreenImage) -> Result {
    let rect = to_rect(util::input(rect, "rect")?);
    let out_image = util::out(out_image, "out_image")?;
    if rect.size.width <= 0 || rect.size.height <= 0 {
        return Err(Error::invalid_arg(format!("rect is {}x{}", rect.size.width, rect.size.height)));
    }
    require_screen_recording()?;
    let image = grab::grab(
        rect,
        Options {
            layered_windows: true,
            opaque: true,
        },
    )
    .map_err(grab_error)?;

    let (width, height, stride) = (image.width, image.height, image.stride() as u32);
    let pixels = Box::into_raw(image.bgra.into_boxed_slice());
    *out_image = FlickSnapScreenImage {
        pixels: pixels.cast::<u8>(),
        len: pixels.len(),
        width,
        height,
        stride,
        _reserved: 0,
    };
    Ok(FLICKSNAP_OK)
}

/// Frees a grabbed image's pixels and zeroes the struct; a second free (or a zeroed struct) is a
/// no-op.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for a null `image`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_screen_image_free(image: *mut FlickSnapScreenImage) -> i32 {
    panic::guard("flicksnap_screen_image_free", || free(image))
}

unsafe fn free(image: *mut FlickSnapScreenImage) -> Result {
    let image = util::out(image, "image")?;
    if !image.pixels.is_null() {
        // SAFETY: (pixels, len) is the boxed slice flicksnap_screen_grab leaked, freed once: the
        // struct is zeroed right after.
        drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(image.pixels, image.len)));
    }
    *image = FlickSnapScreenImage::empty();
    Ok(FLICKSNAP_OK)
}

impl FlickSnapScreenImage {
    fn empty() -> Self {
        FlickSnapScreenImage {
            pixels: std::ptr::null_mut(),
            len: 0,
            width: 0,
            height: 0,
            stride: 0,
            _reserved: 0,
        }
    }
}

fn require_screen_recording() -> Result<()> {
    if flicksnap_platform::permissions::screen_recording() {
        Ok(())
    } else {
        Err(Error::permission("Screen Recording is not granted, so the screen cannot be read"))
    }
}

fn grab_error(e: GrabError) -> Error {
    match e {
        GrabError::Empty {
            ..
        }
        | GrabError::TooLarge {
            ..
        } => Error::invalid_arg(e.to_string()),
        GrabError::Os(os) => Error::os(os.code, os.message),
        GrabError::Format(_) => Error::capability(e.to_string()),
        GrabError::Unsupported => Error::unsupported(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::status::FlickSnapInteropStatus;
    use std::mem::{offset_of, size_of};
    use std::ptr;

    #[test]
    fn layouts_are_pinned() {
        assert_eq!(size_of::<FlickSnapScreenImage>(), 32);
        assert_eq!(offset_of!(FlickSnapScreenImage, pixels), 0);
        assert_eq!(offset_of!(FlickSnapScreenImage, len), 8);
        assert_eq!(offset_of!(FlickSnapScreenImage, width), 16);
        assert_eq!(offset_of!(FlickSnapScreenImage, height), 20);
        assert_eq!(offset_of!(FlickSnapScreenImage, stride), 24);
        assert_eq!(offset_of!(FlickSnapScreenImage, _reserved), 28);
    }

    #[test]
    fn null_arguments_and_empty_rects_are_invalid_arg() {
        let invalid = FlickSnapInteropStatus::InvalidArg as i32;
        let mut image = FlickSnapScreenImage::empty();
        let empty = FlickSnapRect {
            x: 0,
            y: 0,
            width: 0,
            height: 5,
        };
        assert_eq!(unsafe { flicksnap_screen_read_pixel(0, 0, ptr::null_mut()) }, invalid);
        assert_eq!(unsafe { flicksnap_screen_grab(ptr::null(), &mut image) }, invalid);
        assert_eq!(unsafe { flicksnap_screen_grab(&empty, ptr::null_mut()) }, invalid);
        assert_eq!(unsafe { flicksnap_screen_grab(&empty, &mut image) }, invalid);
        assert!(image.pixels.is_null());
        assert_eq!(unsafe { flicksnap_screen_image_free(ptr::null_mut()) }, invalid);
    }

    #[test]
    fn freeing_twice_is_a_no_op() {
        let pixels = Box::into_raw(vec![1u8; 16].into_boxed_slice());
        let mut image = FlickSnapScreenImage {
            pixels: pixels.cast::<u8>(),
            len: 16,
            width: 2,
            height: 2,
            stride: 8,
            _reserved: 0,
        };
        assert_eq!(unsafe { flicksnap_screen_image_free(&mut image) }, 0);
        assert!(image.pixels.is_null());
        assert_eq!((image.len, image.width, image.height, image.stride), (0, 0, 0, 0));
        assert_eq!(unsafe { flicksnap_screen_image_free(&mut image) }, 0);
    }

    #[test]
    fn grab_errors_map_to_their_statuses() {
        use flicksnap_desktop::screens::OsError;
        let status = |e: GrabError| grab_error(e).status;
        assert_eq!(
            status(GrabError::Empty {
                width: 0,
                height: 0
            }),
            FlickSnapInteropStatus::InvalidArg
        );
        assert_eq!(
            status(GrabError::TooLarge {
                width: 1,
                height: 1
            }),
            FlickSnapInteropStatus::InvalidArg
        );
        assert_eq!(status(GrabError::Format("x".into())), FlickSnapInteropStatus::Capability);
        assert_eq!(status(GrabError::Unsupported), FlickSnapInteropStatus::Unsupported);
        let os = grab_error(GrabError::Os(OsError {
            code: -5,
            message: "GetDC failed".into(),
        }));
        assert_eq!((os.status, os.code), (FlickSnapInteropStatus::Os, -5));
    }
}
