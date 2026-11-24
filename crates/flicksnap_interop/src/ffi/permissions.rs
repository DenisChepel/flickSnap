//! Privacy permissions (README section 4.5).
//!
//! Screen Recording and Accessibility answer with `flicksnap_platform::permissions`, the one rule the
//! hotkey hook, the scrolling driver, the capturer and the UI share (Accessibility is
//! `AXIsProcessTrusted && CGPreflightPostEventAccess`). Microphone and Camera read AVFoundation's
//! stored authorization. Off macOS every kind is `NotApplicable`.

#[cfg(target_os = "macos")]
use crate::platform::macos::permissions as av;
use flicksnap_platform::permissions::{self as platform, Permission, State};

use crate::error::{Error, Result};
use crate::ffi::status::FLICKSNAP_OK;
use crate::{panic, util};

/// A permission `flicksnap_permission_state` / `flicksnap_permission_request` know.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapPermissionKind {
    /// Capturing the screen and other apps' windows.
    ScreenRecording = 0,
    /// Event taps and posted events (the hotkey hook, the scrolling driver).
    Accessibility = 1,
    Microphone = 2,
    Camera = 3,
}

crate::convert::ffi_enum!(FlickSnapPermissionKind: i32 {
    ScreenRecording, Accessibility, Microphone, Camera
});

/// What the OS says about a permission.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapPermissionState {
    Granted = 0,
    Denied = 1,
    NotDetermined = 2,
    /// The OS has no such permission (every kind off macOS).
    NotApplicable = 3,
}

crate::convert::ffi_enum!(FlickSnapPermissionState: i32 {
    Granted, Denied, NotDetermined, NotApplicable
});

/// The current state of a permission (a `FlickSnapPermissionKind`), written to `out_state` as a
/// `FlickSnapPermissionState`. Never prompts and never caches. Screen Recording and Accessibility are
/// Granted or Denied on macOS; Microphone and Camera may also be NotDetermined (Restricted reads as
/// Denied). Every kind is NotApplicable off macOS.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for an unknown kind or a null `out_state`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_permission_state(kind: i32, out_state: *mut i32) -> i32 {
    panic::guard("flicksnap_permission_state", || state(kind, out_state))
}

/// Makes sure a permission is granted, prompting only when it is not (an idempotent "ensure"),
/// and writes the state read afterwards with the same rule as `flicksnap_permission_state`. Already
/// Granted (or NotApplicable) returns at once without prompting. Otherwise Screen Recording calls
/// `CGRequestScreenCaptureAccess`; Accessibility calls `AXIsProcessTrustedWithOptions` with the
/// prompt option when AX is not trusted, or `CGRequestPostEventAccess` when only post-event access
/// is missing. macOS offers each prompt once per app, and a fresh Screen Recording grant reaches a
/// running process only after a restart, so Denied afterwards means "not usable yet", not "refused".
/// Microphone and Camera are not requested through this export (no caller asks for them).
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for an unknown kind, Microphone or Camera, or a null
/// `out_state`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_permission_request(kind: i32, out_state: *mut i32) -> i32 {
    panic::guard("flicksnap_permission_request", || request(kind, out_state))
}

unsafe fn state(kind: i32, out_state: *mut i32) -> Result {
    let kind = FlickSnapPermissionKind::from_raw(kind)?;
    let out_state = util::out(out_state, "out_state")?;
    let state = match shared(kind) {
        Some(permission) => from_platform(platform::state(permission)),
        None => av_state(kind),
    };
    *out_state = state as i32;
    Ok(FLICKSNAP_OK)
}

unsafe fn request(kind: i32, out_state: *mut i32) -> Result {
    let kind = FlickSnapPermissionKind::from_raw(kind)?;
    let out_state = util::out(out_state, "out_state")?;
    let Some(permission) = shared(kind) else {
        return Err(Error::invalid_arg(format!(
            "{kind:?} cannot be requested through flicksnap_permission_request"
        )));
    };
    *out_state = from_platform(platform::request(permission)) as i32;
    Ok(FLICKSNAP_OK)
}

/// The kinds flicksnap_platform answers for every crate.
fn shared(kind: FlickSnapPermissionKind) -> Option<Permission> {
    match kind {
        FlickSnapPermissionKind::ScreenRecording => Some(Permission::ScreenRecording),
        FlickSnapPermissionKind::Accessibility => Some(Permission::Accessibility),
        FlickSnapPermissionKind::Microphone | FlickSnapPermissionKind::Camera => None,
    }
}

fn from_platform(state: State) -> FlickSnapPermissionState {
    match state {
        State::Granted => FlickSnapPermissionState::Granted,
        State::Denied => FlickSnapPermissionState::Denied,
        State::NotApplicable => FlickSnapPermissionState::NotApplicable,
    }
}

/// Microphone or Camera: AVFoundation's stored authorization on macOS, NotApplicable elsewhere.
fn av_state(kind: FlickSnapPermissionKind) -> FlickSnapPermissionState {
    #[cfg(target_os = "macos")]
    {
        av::state(kind == FlickSnapPermissionKind::Microphone)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        FlickSnapPermissionState::NotApplicable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::status::FlickSnapInteropStatus as S;

    const UNWRITTEN: i32 = -77;

    #[test]
    fn unknown_kinds_and_null_outs_are_invalid_arg_and_write_nothing() {
        for export in [flicksnap_permission_state, flicksnap_permission_request] {
            let mut state = UNWRITTEN;
            assert_eq!(unsafe { export(4, &mut state) }, S::InvalidArg as i32);
            assert_eq!(unsafe { export(-1, &mut state) }, S::InvalidArg as i32);
            assert_eq!(state, UNWRITTEN);
            assert_eq!(unsafe { export(0, std::ptr::null_mut()) }, S::InvalidArg as i32);
        }
    }

    #[test]
    fn microphone_and_camera_are_not_requested() {
        for kind in [FlickSnapPermissionKind::Microphone, FlickSnapPermissionKind::Camera] {
            let mut state = UNWRITTEN;
            assert_eq!(
                unsafe { flicksnap_permission_request(kind as i32, &mut state) },
                S::InvalidArg as i32
            );
            assert_eq!(state, UNWRITTEN);
        }
    }

    #[test]
    fn every_kind_has_a_state_and_none_is_not_applicable_on_macos() {
        for kind in [
            FlickSnapPermissionKind::ScreenRecording,
            FlickSnapPermissionKind::Accessibility,
            FlickSnapPermissionKind::Microphone,
            FlickSnapPermissionKind::Camera,
        ] {
            let mut state = UNWRITTEN;
            assert_eq!(unsafe { flicksnap_permission_state(kind as i32, &mut state) }, FLICKSNAP_OK);
            let state = FlickSnapPermissionState::from_raw(state).expect("a known state");
            if cfg!(target_os = "macos") {
                assert_ne!(state, FlickSnapPermissionState::NotApplicable, "{kind:?}");
            } else {
                assert_eq!(state, FlickSnapPermissionState::NotApplicable, "{kind:?}");
            }
        }
    }
}
