//! The macapp exports through the library's public surface (README section 4.5): permissions on
//! any thread, and the main-thread exports refusing a libtest thread. The main-thread success
//! paths are in main_thread_macapp.rs.

use std::ffi::c_void;
use std::ptr;
use std::time::{Duration, Instant};

use flicksnap_interop::ffi::app::{flicksnap_app_set_activation_policy_macos, FlickSnapActivationPolicy};
use flicksnap_interop::ffi::permissions::{
    flicksnap_permission_request, flicksnap_permission_state, FlickSnapPermissionKind as K, FlickSnapPermissionState,
};
use flicksnap_interop::ffi::services::flicksnap_services_register_provider_macos;
use flicksnap_interop::ffi::status::FlickSnapInteropStatus as S;
use flicksnap_interop::ffi::types::FlickSnapString;
use flicksnap_platform::permissions::{self as platform, Permission, State};

const OK: i32 = S::Ok as i32;
const KINDS: [K; 4] = [K::ScreenRecording, K::Accessibility, K::Microphone, K::Camera];

fn state(kind: K) -> i32 {
    let mut state = -1;
    assert_eq!(unsafe { flicksnap_permission_state(kind as i32, &mut state) }, OK, "{kind:?}");
    state
}

fn expected(state: State) -> i32 {
    (match state {
        State::Granted => FlickSnapPermissionState::Granted,
        State::Denied => FlickSnapPermissionState::Denied,
        State::NotApplicable => FlickSnapPermissionState::NotApplicable,
    }) as i32
}

#[test]
fn state_agrees_with_flicksnap_platform_and_never_prompts() {
    for (kind, permission) in [
        (K::ScreenRecording, Permission::ScreenRecording),
        (K::Accessibility, Permission::Accessibility),
    ] {
        let started = Instant::now();
        let answer = state(kind);
        // A prompt would block or take far longer than a TCC read (about 8 ms).
        assert!(started.elapsed() < Duration::from_secs(2), "{kind:?} took {:?}", started.elapsed());
        assert_eq!(answer, expected(platform::state(permission)), "{kind:?}");
    }
}

#[test]
fn every_kind_answers_and_is_not_applicable_only_off_macos() {
    for kind in KINDS {
        let answer = state(kind);
        assert!((0..=3).contains(&answer), "{kind:?} wrote {answer}");
        let not_applicable = answer == FlickSnapPermissionState::NotApplicable as i32;
        assert_eq!(not_applicable, !cfg!(target_os = "macos"), "{kind:?} wrote {answer}");
    }
}

/// Only a kind that is already usable is requested, which by contract never reaches a prompt, so
/// this is safe on a developer machine and in CI. Elsewhere the case is reported and skipped.
#[test]
fn request_on_a_usable_kind_returns_it_without_prompting() {
    for kind in [K::ScreenRecording, K::Accessibility] {
        let before = state(kind);
        if before == FlickSnapPermissionState::Denied as i32 {
            println!("skipping {kind:?}: not granted here, and a request would prompt");
            continue;
        }
        let started = Instant::now();
        let mut after = -1;
        assert_eq!(unsafe { flicksnap_permission_request(kind as i32, &mut after) }, OK);
        assert_eq!(after, before, "{kind:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "{kind:?}");
    }
}

#[test]
fn microphone_camera_and_unknown_kinds_are_not_requestable() {
    for kind in [K::Microphone as i32, K::Camera as i32, 4, -1] {
        let mut after = -1;
        assert_eq!(
            unsafe { flicksnap_permission_request(kind, &mut after) },
            S::InvalidArg as i32,
            "{kind}"
        );
        assert_eq!(after, -1);
    }
}

unsafe extern "C" fn never_called(_user: *mut c_void, _paths: *const FlickSnapString, _count: usize) {
    unreachable!("this test registers no provider");
}

#[test]
fn main_thread_exports_answer_e_state_off_the_main_thread() {
    let expected = if cfg!(target_os = "macos") { S::State } else { S::Unsupported } as i32;
    assert_eq!(
        unsafe { flicksnap_app_set_activation_policy_macos(FlickSnapActivationPolicy::Accessory as i32, 0) },
        expected
    );
    let name = b"FlickSnapInteropTest";
    assert_eq!(
        unsafe { flicksnap_services_register_provider_macos(name.as_ptr(), name.len(), Some(never_called), ptr::null_mut()) },
        expected
    );
}
