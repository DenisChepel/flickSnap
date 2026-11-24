//! The microphone input (WP-AUDIO; README section 4.6). Windows answers `FLICKSNAP_E_UNSUPPORTED` this
//! phase (NAudio's WASAPI capture serves it, R15).
//!
//! Callback contract: a `Data` event returns 1 to continue and 0 to end delivery (no `Stopped`
//! follows an end the callee asked for); `Stopped` (the device went away, or the queue refused a
//! buffer) fires at most once and ends delivery; `flicksnap_audio_input_request_stop` ends delivery
//! without invoking the callback. Data and Stopped never overlap. The OS gets a token, not a
//! pointer, so a late OS callback after destroy finds nothing. Neither event may call an export.

use std::ffi::c_void;

use crate::error::Error;
use crate::ffi::status::FLICKSNAP_OK;
use crate::ffi::types::FlickSnapString;
use crate::panic;
use crate::util;

#[cfg(target_os = "macos")]
use crate::platform::macos::audio_input as os;

/// The microphone input handle.
pub struct FlickSnapAudioInput {
    _private: [u8; 0],
}

/// What a `FlickSnapAudioInputEvent` carries.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapAudioInputEventKind {
    /// A captured buffer: `data_len` bytes of interleaved float32 at `data`.
    Data = 0,
    /// Delivery ended by itself; `stop` says why.
    Stopped = 1,
}

crate::convert::ffi_enum!(FlickSnapAudioInputEventKind: u32 { Data, Stopped });

/// Why the input stopped by itself.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapAudioInputStop {
    /// The device was unplugged or disabled (its is-alive flag cleared, or it stopped answering).
    DeviceLost = 1,
    /// The queue refused a buffer back; `os_status` is the OSStatus.
    RequeueFailed = 2,
}

crate::convert::ffi_enum!(FlickSnapAudioInputStop: i32 { DeviceLost, RequeueFailed });

/// One input event (24 bytes), borrowed for the duration of the callback.
#[repr(C)]
#[derive(Debug)]
pub struct FlickSnapAudioInputEvent {
    /// A `FlickSnapAudioInputEventKind`.
    pub kind: u32,
    pub data_len: u32,
    /// Float32 interleaved samples (Data only), valid until the callback returns.
    pub data: *const u8,
    /// A `FlickSnapAudioInputStop` (Stopped only).
    pub stop: i32,
    pub os_status: i32,
}

/// What `flicksnap_audio_input_open` reports (32 bytes). `name` is mode 3, valid until the handle's
/// `_destroy`.
#[repr(C)]
#[derive(Debug)]
pub struct FlickSnapAudioInputInfo {
    /// The device's own input channel count (at least 1); every Data buffer is interleaved in it.
    pub channels: u32,
    /// The rate every Data buffer is in (the requested rate: the queue converts).
    pub sample_rate_hz: u32,
    /// 1 when the system default input was opened because no UID was given or it did not resolve.
    pub used_default: u8,
    pub _reserved: [u8; 7],
    /// The device's name (its UID when the name cannot be read).
    pub name: FlickSnapString,
}

/// The input callback: return 1 to continue, 0 to end (no Stopped follows a callee-requested
/// end). Runs on the OS audio threads, never two at once; must not block for long and must not
/// call any export (README C3).
pub type FlickSnapAudioInputCallback = Option<unsafe extern "C" fn(user: *mut c_void, event: *const FlickSnapAudioInputEvent) -> u8>;

/// Opens the input device with the UTF-8 `device_uid` (empty, or a UID that no longer resolves,
/// is the system default input) for interleaved float32 at `sample_rate_hz` in buffers of about
/// `buffer_ms`, with three buffers queued; delivery starts with `flicksnap_audio_input_start`. Reads the
/// stored microphone authorization first and never prompts: Denied or Restricted is
/// `FLICKSNAP_E_PERMISSION` (NotDetermined proceeds). Nothing is left open on failure.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` (null callback or out pointer, bad UTF-8, zero rate,
/// a buffer too large); `FLICKSNAP_E_PERMISSION`; `FLICKSNAP_E_CAPABILITY` (no default input, or the device
/// has no UID); `FLICKSNAP_E_OS` (an AudioToolbox call failed, OSStatus in the last-error code);
/// `FLICKSNAP_E_UNSUPPORTED` on Windows and off macOS.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_audio_input_open(
    device_uid: *const u8,
    uid_len: usize,
    sample_rate_hz: u32,
    buffer_ms: u32,
    callback: FlickSnapAudioInputCallback,
    user: *mut c_void,
    out_input: *mut *mut FlickSnapAudioInput,
    out_info: *mut FlickSnapAudioInputInfo,
) -> i32 {
    panic::guard("flicksnap_audio_input_open", || {
        let uid = util::utf8(device_uid, uid_len, "device_uid")?;
        let out_input = util::out(out_input, "out_input")?;
        let out_info = util::out(out_info, "out_info")?;
        let callback = callback.ok_or_else(|| Error::null("callback"))?;
        if sample_rate_hz == 0 {
            return Err(Error::invalid_arg("sample_rate_hz was zero"));
        }
        #[cfg(target_os = "macos")]
        {
            let input = os::Input::open(uid, sample_rate_hz, buffer_ms, callback, user)?;
            *out_info = input.info();
            *out_input = os::Input::into_handle(input);
            Ok(FLICKSNAP_OK)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (uid, buffer_ms, callback, user, out_input, out_info);
            Err(Error::unsupported(
                "flicksnap_audio_input_open is implemented on macOS only (NAudio serves Windows)",
            ))
        }
    })
}

/// Starts delivery (idempotent) and the device-alive listener; `out_watching` (optional) is 1 when
/// the listener is on, 0 when the HAL refused it (an unplug then freezes delivery instead of
/// raising Stopped).
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` (null handle); `FLICKSNAP_E_STATE` (delivery already ended);
/// `FLICKSNAP_E_OS`; `FLICKSNAP_E_PANIC` (an OS callback trampoline panicked since the last call);
/// `FLICKSNAP_E_UNSUPPORTED` off macOS.
///
/// Thread: any thread, never concurrently with another call on the same handle.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_audio_input_start(input: *mut FlickSnapAudioInput, out_watching: *mut u8) -> i32 {
    panic::guard("flicksnap_audio_input_start", || {
        #[cfg(target_os = "macos")]
        {
            let watching = os::Input::live(input)?.start()?;
            util::write_optional(out_watching, util::to_flag(watching));
            Ok(FLICKSNAP_OK)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (input, out_watching);
            Err(Error::unsupported("flicksnap_audio_input_start is implemented on macOS only"))
        }
    })
}

/// Ends delivery once and never invokes the callback; `out_first` (optional) is 1 for the call
/// that ended it, 0 when delivery had already ended (an earlier request, a callee-requested end, or
/// a Stopped event). A callback already running finishes; none starts after this returns.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` (null handle); `FLICKSNAP_E_PANIC`;
/// `FLICKSNAP_E_UNSUPPORTED` off macOS.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_audio_input_request_stop(input: *mut FlickSnapAudioInput, out_first: *mut u8) -> i32 {
    panic::guard("flicksnap_audio_input_request_stop", || {
        #[cfg(target_os = "macos")]
        {
            let first = os::Input::live(input)?.request_stop();
            util::write_optional(out_first, util::to_flag(first));
            Ok(FLICKSNAP_OK)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (input, out_first);
            Err(Error::unsupported(
                "flicksnap_audio_input_request_stop is implemented on macOS only",
            ))
        }
    })
}

/// Ends delivery, removes the listener, waits out a callback in flight, stops and disposes the
/// queue and frees the handle; null is `FLICKSNAP_OK`. When it returns the callback never runs again
/// and the caller may free whatever `user` points at. Teardown always runs; a teardown OSStatus and
/// a trampoline panic not yet reported are logged, not returned. Never call it from inside the
/// callback (that is `FLICKSNAP_E_STATE` and destroys nothing).
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_STATE`; `FLICKSNAP_E_UNSUPPORTED` off macOS (for a non-null handle).
///
/// Thread: any thread, never concurrently with another call on the same handle.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_audio_input_destroy(input: *mut FlickSnapAudioInput) -> i32 {
    panic::guard("flicksnap_audio_input_destroy", || {
        if input.is_null() {
            return Ok(FLICKSNAP_OK);
        }
        #[cfg(target_os = "macos")]
        {
            os::Input::destroy(input)?;
            Ok(FLICKSNAP_OK)
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(Error::unsupported("flicksnap_audio_input_destroy is implemented on macOS only"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_are_pinned() {
        assert_eq!(size_of::<FlickSnapAudioInputEvent>(), 24);
        assert_eq!(offset_of!(FlickSnapAudioInputEvent, kind), 0);
        assert_eq!(offset_of!(FlickSnapAudioInputEvent, data_len), 4);
        assert_eq!(offset_of!(FlickSnapAudioInputEvent, data), 8);
        assert_eq!(offset_of!(FlickSnapAudioInputEvent, stop), 16);
        assert_eq!(offset_of!(FlickSnapAudioInputEvent, os_status), 20);
        assert_eq!(size_of::<FlickSnapAudioInputInfo>(), 32);
        assert_eq!(offset_of!(FlickSnapAudioInputInfo, channels), 0);
        assert_eq!(offset_of!(FlickSnapAudioInputInfo, sample_rate_hz), 4);
        assert_eq!(offset_of!(FlickSnapAudioInputInfo, used_default), 8);
        assert_eq!(offset_of!(FlickSnapAudioInputInfo, _reserved), 9);
        assert_eq!(offset_of!(FlickSnapAudioInputInfo, name), 16);
    }
}
