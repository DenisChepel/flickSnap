//! The supervised global hotkey hook (README section 4.8): registered hotkeys, one-shot gesture
//! recording and the keystroke observe stream, from one hook that comes back by itself. The hook,
//! its macOS watch and its supervisor run on threads the handle owns (README D4), started by
//! `flicksnap_hotkeys_create` and joined by `flicksnap_hotkeys_stop` / `flicksnap_hotkeys_destroy`; events are
//! pulled with `flicksnap_hotkeys_next_event`, never pushed, so the hook never enters the host.
//!
//! Supervision (flicksnap_hotkeys `Supervised`): a hook that cannot start or dies is retried every 2 s
//! where that can help (macOS: a missing or stale Accessibility grant, a tap that could not be
//! made, a death), with a stale grant backing off to every 32 s; registrations keep their ids and
//! the observe switch holds across restarts. Every change of state arrives as a Status event.

use std::time::Duration;

use crate::error::{Error, Result};
use crate::ffi::status::{FLICKSNAP_OK, FLICKSNAP_S_CLOSED, FLICKSNAP_S_TIMEOUT};
use crate::ffi::types::{FlickSnapGestureRefusal, FlickSnapString};
use crate::hotkeys_host::{empty_event, HotkeysHost, Refused};
use crate::{panic, util};

/// The hotkey hook handle.
pub struct FlickSnapHotkeys {
    _private: [u8; 0],
}

/// What a `FlickSnapHotkeysEvent` reports.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapHotkeysEventKind {
    /// Registration `id` was pressed (`Repeat`: an auto-repeat of a press that fired).
    Hotkey = 1,
    /// While recording session `id`: the modifiers held changed.
    RecordingModifiers = 2,
    /// Recording session `id` finished with `key` + `modifiers` (swallowed).
    Recorded = 3,
    /// The observe stream: one key edge (`Down`, `Repeat`), with `raw_code` and the typed `ch`.
    Key = 4,
    /// The hook came up or went down: `hook_state`, `Recoverable`, and for a down state `reason`.
    Status = 5,
}

crate::convert::ffi_enum!(FlickSnapHotkeysEventKind: u32 {
    Hotkey, RecordingModifiers, Recorded, Key, Status
});

/// The hook's state in a Status event.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapHookState {
    /// The hook runs; every registration is live.
    Running = 0,
    /// macOS: the Accessibility permission is missing (or was revoked).
    PermissionMissing = 1,
    /// macOS: Accessibility reads as granted, yet the tap gets no key events (a stale grant that
    /// only removing and re-adding the app fixes).
    PermissionStale = 2,
    /// The hook could not start (the OS refused it; `reason` says how).
    Failed = 3,
    /// The hook ran and stopped (`reason` says why).
    Stopped = 4,
}

crate::convert::ffi_enum!(FlickSnapHookState: i32 {
    Running, PermissionMissing, PermissionStale, Failed, Stopped
});

/// `FlickSnapHotkeysEvent.flags`; bit flags (`[Flags]` in C#).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapHotkeysEventFlags {
    None = 0,
    /// Key: a key-down (else a key-up).
    Down = 1,
    /// Hotkey, Key: an auto-repeat.
    Repeat = 2,
    /// Status: the supervisor tries again by itself, so the hook may come back without a restart.
    Recoverable = 4,
}

/// One hook event (48 bytes on 64-bit).
#[repr(C)]
#[derive(Debug)]
pub struct FlickSnapHotkeysEvent {
    /// A `FlickSnapHotkeysEventKind`.
    pub kind: u32,
    /// Hotkey: the registration id; RecordingModifiers, Recorded: the recording session; else 0.
    pub id: u32,
    /// Recorded, Key: the key (the hotkey key table's value, 0 = none).
    pub key: i32,
    /// The modifier bits (Alt 1, Control 2, Shift 4, Meta 8).
    pub modifiers: u32,
    /// Key: the Windows virtual-key code or macOS CGKeyCode, passed through.
    pub raw_code: u32,
    /// Key: the Unicode scalar a key-down types, 0 for none.
    pub ch: u32,
    /// `FlickSnapHotkeysEventFlags`.
    pub flags: u32,
    /// A `FlickSnapHookState` (Status only).
    pub hook_state: i32,
    /// Status with a down state only: why, in the hook's words (mode 1, the receiver frees it with
    /// `flicksnap_string_free`); empty otherwise.
    pub reason: FlickSnapString,
}

/// The live host behind `hook`; `FLICKSNAP_E_INVALID_ARG` for null.
unsafe fn host<'a>(hook: *mut FlickSnapHotkeys) -> Result<&'a HotkeysHost> {
    hook.cast::<HotkeysHost>()
        .as_ref()
        .ok_or_else(|| Error::null("hook"))
}

/// Starts a supervised hook and writes its handle to `out_hook` (null on failure). The first
/// event is a Status with the state the first start left, queued before this returns; a hook that
/// is down is not a failure here (it says so in that Status, and comes back by itself where it
/// can).
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` (null `out_hook`); `FLICKSNAP_E_UNSUPPORTED` on an OS with
/// no hook backend (Linux); `FLICKSNAP_E_OS` when the supervisor thread cannot be started.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_create(out_hook: *mut *mut FlickSnapHotkeys) -> i32 {
    panic::guard("flicksnap_hotkeys_create", || {
        let out = util::out(out_hook, "out_hook")?;
        *out = std::ptr::null_mut();
        *out = Box::into_raw(HotkeysHost::create()?).cast::<FlickSnapHotkeys>();
        Ok(FLICKSNAP_OK)
    })
}

/// Registers `key` + `modifiers` and writes its id to `out_id` (stable across hook restarts, never
/// reused by the handle). A gesture is refused at once with `FLICKSNAP_E_INVALID_ARG` and its
/// `FlickSnapGestureRefusal` in `out_refusal` (optional; `None` on success): `InvalidKey`, `NoKey`,
/// `InvalidModifiers`, `ModifierOnly`, `UnsupportedKey`, `Duplicate`. While the hook is down a
/// registration is accepted and held; it goes live when the hook comes back.
///
/// Status: `FLICKSNAP_OK`, `FLICKSNAP_E_INVALID_ARG`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_register(
    hook: *mut FlickSnapHotkeys,
    key: i32,
    modifiers: u32,
    out_id: *mut u32,
    out_refusal: *mut i32,
) -> i32 {
    panic::guard("flicksnap_hotkeys_register", || {
        let host = host(hook)?;
        let out_id = util::out(out_id, "out_id")?;
        *out_id = 0;
        util::write_optional(out_refusal, FlickSnapGestureRefusal::None as i32);
        match host.register(key, modifiers) {
            Ok(id) => {
                *out_id = id;
                Ok(FLICKSNAP_OK)
            }
            Err(Refused::Gesture(refusal, text)) => {
                util::write_optional(out_refusal, refusal as i32);
                Err(Error::invalid_arg(text))
            }
            Err(Refused::Other(error)) => Err(error),
        }
    })
}

/// Unregisters `id`: OK in every state, and for an id that is not (or no longer) registered.
///
/// Status: `FLICKSNAP_OK`, `FLICKSNAP_E_INVALID_ARG` (null `hook`).
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_unregister(hook: *mut FlickSnapHotkeys, id: u32) -> i32 {
    panic::guard("flicksnap_hotkeys_unregister", || {
        host(hook)?.unregister(id);
        Ok(FLICKSNAP_OK)
    })
}

/// Turns the observe stream (Key events) on (non-zero) or off; holds across hook restarts.
///
/// Status: `FLICKSNAP_OK`, `FLICKSNAP_E_INVALID_ARG` (null `hook`).
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_set_observe(hook: *mut FlickSnapHotkeys, observe: u8) -> i32 {
    panic::guard("flicksnap_hotkeys_set_observe", || {
        host(hook)?.set_observe(util::flag(observe));
        Ok(FLICKSNAP_OK)
    })
}

/// Arms a one-shot gesture recording and writes its session token (nonzero, never reused by the
/// handle) to `out_session`. The next non-modifier key-down is swallowed and reported as a
/// Recorded event; while armed, registered gestures do not fire. Arming again replaces the
/// session.
///
/// Status: `FLICKSNAP_OK`, `FLICKSNAP_E_INVALID_ARG`, `FLICKSNAP_E_STATE` (the hook is down).
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_start_recording(hook: *mut FlickSnapHotkeys, out_session: *mut u32) -> i32 {
    panic::guard("flicksnap_hotkeys_start_recording", || {
        let host = host(hook)?;
        let out = util::out(out_session, "out_session")?;
        *out = 0;
        *out = host.start_recording()?;
        Ok(FLICKSNAP_OK)
    })
}

/// Cancels `session` if it is still the armed one; a no-op otherwise.
///
/// Status: `FLICKSNAP_OK`, `FLICKSNAP_E_INVALID_ARG` (null `hook`).
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_stop_recording(hook: *mut FlickSnapHotkeys, session: u32) -> i32 {
    panic::guard("flicksnap_hotkeys_stop_recording", || {
        host(hook)?.stop_recording(session);
        Ok(FLICKSNAP_OK)
    })
}

/// Waits up to `timeout_ms` for the next event and writes it to `out_event` (an empty event when
/// there is none). A Status event's `reason` is the caller's to free. Calls are serialized.
///
/// Status: `FLICKSNAP_OK` (an event), `FLICKSNAP_S_TIMEOUT`, `FLICKSNAP_S_CLOSED` (stopped, and every event
/// delivered), `FLICKSNAP_E_INVALID_ARG`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_next_event(
    hook: *mut FlickSnapHotkeys,
    timeout_ms: u32,
    out_event: *mut FlickSnapHotkeysEvent,
) -> i32 {
    panic::guard("flicksnap_hotkeys_next_event", || {
        let host = host(hook)?;
        let out = util::out(out_event, "out_event")?;
        *out = empty_event();
        match host.next_event(Duration::from_millis(u64::from(timeout_ms))) {
            Ok(event) => {
                *out = event;
                Ok(FLICKSNAP_OK)
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(FLICKSNAP_S_TIMEOUT),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ok(FLICKSNAP_S_CLOSED),
        }
    })
}

/// Stops the hook and its supervisor and joins their threads, so registered gestures reach other
/// apps again. Idempotent, and safe while another thread waits in `flicksnap_hotkeys_next_event`,
/// which then drains what is queued and answers `FLICKSNAP_S_CLOSED`. Every later call but destroy is
/// a harmless no-op (recording answers `FLICKSNAP_E_STATE`).
///
/// Status: `FLICKSNAP_OK`, `FLICKSNAP_E_INVALID_ARG` (null `hook`).
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_stop(hook: *mut FlickSnapHotkeys) -> i32 {
    panic::guard("flicksnap_hotkeys_stop", || {
        host(hook)?.stop();
        Ok(FLICKSNAP_OK)
    })
}

/// Stops (if needed) and frees the hook; null is `FLICKSNAP_OK`.
///
/// Status: `FLICKSNAP_OK`.
///
/// Thread: any thread, never concurrently with another call on the same handle.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_hotkeys_destroy(hook: *mut FlickSnapHotkeys) -> i32 {
    panic::guard("flicksnap_hotkeys_destroy", || {
        if !hook.is_null() {
            drop(Box::from_raw(hook.cast::<HotkeysHost>()));
        }
        Ok(FLICKSNAP_OK)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_are_pinned() {
        assert_eq!(size_of::<FlickSnapHotkeysEvent>(), 48);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, kind), 0);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, id), 4);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, key), 8);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, modifiers), 12);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, raw_code), 16);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, ch), 20);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, flags), 24);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, hook_state), 28);
        assert_eq!(offset_of!(FlickSnapHotkeysEvent, reason), 32);
    }
}
