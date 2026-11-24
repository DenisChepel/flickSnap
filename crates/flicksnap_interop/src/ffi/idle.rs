//! The idle time (README section 4.2): how long the whole machine has been without input, which
//! decides whether FlickSnap may restart itself under the user to apply an update.

use crate::error::Result;
use crate::{panic, util};

#[cfg(target_os = "macos")]
use crate::platform::macos::idle as imp;
#[cfg(windows)]
use crate::platform::windows::idle as imp;

/// Seconds since the last keyboard or mouse input anywhere on the machine, not just in FlickSnap:
/// `GetLastInputInfo` against `GetTickCount` on Windows (the same wrapping 32-bit millisecond
/// counter, so the 49.7-day wrap comes out right), `CGEventSourceSecondsSinceLastEventType` for
/// the combined session and any input event on macOS. `out_seconds` is written only on
/// `FLICKSNAP_OK`.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_S_NONE` when CoreGraphics answers no interval (a negative one);
/// `FLICKSNAP_E_INVALID_ARG` for a null `out_seconds`; `FLICKSNAP_E_OS` when `GetLastInputInfo` fails;
/// `FLICKSNAP_E_UNSUPPORTED` on a platform without a system idle time.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_idle_seconds(out_seconds: *mut f64) -> i32 {
    panic::guard("flicksnap_idle_seconds", || idle_seconds(out_seconds))
}

unsafe fn idle_seconds(out_seconds: *mut f64) -> Result {
    let out_seconds = util::out(out_seconds, "out_seconds")?;
    #[cfg(any(windows, target_os = "macos"))]
    {
        match imp::idle_seconds()? {
            Some(seconds) => {
                *out_seconds = seconds;
                Ok(crate::ffi::status::FLICKSNAP_OK)
            }
            None => Ok(crate::ffi::status::FLICKSNAP_S_NONE),
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = out_seconds;
        Err(crate::error::Error::unsupported("there is no system idle time on this platform"))
    }
}

/// The idle time in milliseconds between two readings of the same wrapping 32-bit millisecond
/// counter (`GetTickCount` and `LASTINPUTINFO.dwTime`): unsigned subtraction is right across the
/// wrap.
#[allow(dead_code)] // the Windows half and the tests use it
pub(crate) fn elapsed_ms(now: u32, last_input: u32) -> u32 {
    now.wrapping_sub(last_input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::status::{FlickSnapInteropStatus, FLICKSNAP_OK, FLICKSNAP_S_NONE};

    #[test]
    fn the_tick_difference_survives_the_wrap() {
        assert_eq!(elapsed_ms(5_000, 2_000), 3_000);
        assert_eq!(elapsed_ms(10, u32::MAX - 9), 20);
        assert_eq!(elapsed_ms(7, 7), 0);
    }

    #[test]
    fn a_null_out_is_invalid_arg() {
        assert_eq!(
            unsafe { flicksnap_idle_seconds(std::ptr::null_mut()) },
            FlickSnapInteropStatus::InvalidArg as i32
        );
    }

    #[test]
    fn the_idle_time_is_plausible() {
        let mut seconds = -1.0f64;
        let status = unsafe { flicksnap_idle_seconds(&mut seconds) };
        if !cfg!(any(windows, target_os = "macos")) {
            assert_eq!(status, FlickSnapInteropStatus::Unsupported as i32);
            return;
        }
        if status == FLICKSNAP_S_NONE {
            eprintln!("skipping: this session reports no idle time");
            return;
        }
        assert_eq!(status, FLICKSNAP_OK);
        // a year: anything longer is a unit slip or a clock that wrapped the wrong way
        assert!((0.0..31_536_000.0).contains(&seconds), "implausible idle time {seconds}");
    }
}
