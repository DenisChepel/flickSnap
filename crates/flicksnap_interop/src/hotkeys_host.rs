//! The supervised hotkey hook behind the `flicksnap_hotkeys_*` exports: a `flicksnap_hotkeys::Supervised`
//! (retries, the stale-grant back-off, the recovery policy per OS, registrations with stable ids)
//! and the mapping of its events onto the C ABI. The only threads a handle of this crate owns live
//! inside it (`flicksnap-hotkeys-hook`, `flicksnap-hotkeys-watch` on macOS, `flicksnap-hotkeys-supervise`):
//! started by `flicksnap_hotkeys_create`, joined by `flicksnap_hotkeys_stop` / `flicksnap_hotkeys_destroy`,
//! never calling the host and never logging (README D4).

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use flicksnap_hotkeys::supervise::{Down, Status, Supervised, SupervisedEvent};

use crate::error::{Error, Result};
use crate::ffi::gesture::{gesture_from_raw, refusal_of};
use crate::ffi::hotkeys::{FlickSnapHookState, FlickSnapHotkeysEvent, FlickSnapHotkeysEventFlags, FlickSnapHotkeysEventKind};
use crate::ffi::types::{FlickSnapGestureRefusal, FlickSnapString};

/// The real type behind a `FlickSnapHotkeys*`.
pub(crate) struct HotkeysHost {
    supervised: Supervised,
}

/// Why `register` said no.
pub(crate) enum Refused {
    /// The gesture itself: `FLICKSNAP_E_INVALID_ARG` with this refusal and text.
    Gesture(FlickSnapGestureRefusal, String),
    /// Anything else (never expected from a supervised hook).
    Other(Error),
}

impl HotkeysHost {
    /// Starts the supervised hook; its first event is a Status with the state the first start left.
    pub(crate) fn create() -> Result<Box<HotkeysHost>> {
        if !cfg!(any(windows, target_os = "macos")) {
            return Err(Error::unsupported("global hotkeys are not supported on this OS"));
        }
        let supervised = Supervised::start().map_err(|e| {
            Error::os(
                i64::from(e.raw_os_error().unwrap_or(0)),
                format!("could not start the hotkey supervisor thread: {e}"),
            )
        })?;
        Ok(Box::new(HotkeysHost {
            supervised,
        }))
    }

    /// Registers `key` + `modifiers` under a stable id, held while the hook is down.
    pub(crate) fn register(&self, key: i32, modifiers: u32) -> std::result::Result<u32, Refused> {
        let gesture = gesture_from_raw(key, modifiers).map_err(|(refusal, text)| Refused::Gesture(refusal, text))?;
        self.supervised
            .register(gesture)
            .map_err(|error| match refusal_of(&error) {
                Some(refusal) => Refused::Gesture(refusal, error.to_string()),
                None => Refused::Other(Error::state(error.to_string())),
            })
    }

    pub(crate) fn unregister(&self, id: u32) {
        self.supervised.unregister(id);
    }

    pub(crate) fn set_observe(&self, observe: bool) {
        self.supervised.set_observe(observe);
    }

    /// A recording token; `FLICKSNAP_E_STATE` while the hook is down.
    pub(crate) fn start_recording(&self) -> Result<u32> {
        self.supervised
            .start_recording()
            .ok_or_else(|| Error::state("the hotkey hook is down, so nothing can be recorded"))
    }

    pub(crate) fn stop_recording(&self, session: u32) {
        self.supervised.stop_recording(session);
    }

    pub(crate) fn next_event(&self, timeout: Duration) -> std::result::Result<FlickSnapHotkeysEvent, RecvTimeoutError> {
        self.supervised
            .next_event(timeout)
            .map(to_ffi)
    }

    pub(crate) fn stop(&self) {
        self.supervised.stop();
    }
}

/// An event with nothing in it: what `next_event` writes when there is no event.
pub(crate) fn empty_event() -> FlickSnapHotkeysEvent {
    FlickSnapHotkeysEvent {
        kind: 0,
        id: 0,
        key: 0,
        modifiers: 0,
        raw_code: 0,
        ch: 0,
        flags: FlickSnapHotkeysEventFlags::None as u32,
        hook_state: FlickSnapHookState::Running as i32,
        reason: FlickSnapString::empty(),
    }
}

/// The hook state a Status reports.
pub(crate) fn hook_state(down: Option<&Down>) -> FlickSnapHookState {
    match down {
        None => FlickSnapHookState::Running,
        Some(Down::StaleGrant) => FlickSnapHookState::PermissionStale,
        Some(Down::Error(flicksnap_hotkeys::Error::PermissionDenied)) => FlickSnapHookState::PermissionMissing,
        Some(Down::Error(flicksnap_hotkeys::Error::Stopped)) => FlickSnapHookState::Stopped,
        Some(Down::Error(_)) => FlickSnapHookState::Failed,
    }
}

fn flag(set: bool, flag: FlickSnapHotkeysEventFlags) -> u32 {
    if set {
        flag as u32
    } else {
        0
    }
}

/// One supervised event as the C side sees it. A Status for a down hook carries its reason as a
/// mode 1 string the receiver frees; every other event's reason is empty.
pub(crate) fn to_ffi(event: SupervisedEvent) -> FlickSnapHotkeysEvent {
    let mut out = empty_event();
    match event {
        SupervisedEvent::Hotkey {
            id,
            repeat,
        } => {
            out.kind = FlickSnapHotkeysEventKind::Hotkey as u32;
            out.id = id;
            out.flags = flag(repeat, FlickSnapHotkeysEventFlags::Repeat);
        }
        SupervisedEvent::RecordingModifiers {
            session,
            modifiers,
        } => {
            out.kind = FlickSnapHotkeysEventKind::RecordingModifiers as u32;
            out.id = session;
            out.modifiers = u32::from(modifiers.bits());
        }
        SupervisedEvent::Recorded {
            session,
            gesture,
        } => {
            out.kind = FlickSnapHotkeysEventKind::Recorded as u32;
            out.id = session;
            out.key = gesture.key as i32;
            out.modifiers = u32::from(gesture.modifiers.bits());
        }
        SupervisedEvent::Key(key) => {
            out.kind = FlickSnapHotkeysEventKind::Key as u32;
            out.key = key.key as i32;
            out.modifiers = u32::from(key.modifiers.bits());
            out.raw_code = key.raw_code;
            out.ch = key.ch.map_or(0, u32::from);
            out.flags = flag(key.down, FlickSnapHotkeysEventFlags::Down) | flag(key.repeat, FlickSnapHotkeysEventFlags::Repeat);
        }
        SupervisedEvent::Status(Status {
            down,
            recoverable,
            detail,
        }) => {
            out.kind = FlickSnapHotkeysEventKind::Status as u32;
            out.hook_state = hook_state(down.as_ref()) as i32;
            out.flags = flag(recoverable, FlickSnapHotkeysEventFlags::Recoverable);
            if down.is_some() {
                out.reason = FlickSnapString::from_string(detail);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicksnap_hotkeys::{Gesture, Key, KeyEvent, Modifiers};

    fn take_reason(event: &mut FlickSnapHotkeysEvent) -> String {
        let text = unsafe { event.reason.as_str() }.to_string();
        unsafe { event.reason.release() };
        text
    }

    #[test]
    fn every_event_kind_maps() {
        let gesture = Gesture::new(Key::PrintScreen, Modifiers::CONTROL | Modifiers::SHIFT);
        let e = to_ffi(SupervisedEvent::Hotkey {
            id: 9,
            repeat: true,
        });
        assert_eq!((e.kind, e.id, e.flags), (1, 9, 2));
        let e = to_ffi(SupervisedEvent::RecordingModifiers {
            session: 4,
            modifiers: Modifiers::ALT | Modifiers::META,
        });
        assert_eq!((e.kind, e.id, e.modifiers), (2, 4, 9));
        let e = to_ffi(SupervisedEvent::Recorded {
            session: 5,
            gesture,
        });
        assert_eq!((e.kind, e.id, e.key, e.modifiers), (3, 5, Key::PrintScreen as i32, 6));
        let key = KeyEvent {
            down: true,
            repeat: true,
            key: Key::E,
            modifiers: Modifiers::SHIFT,
            raw_code: 0x45,
            ch: Some('\u{20AC}'),
        };
        let e = to_ffi(SupervisedEvent::Key(key));
        assert_eq!(
            (e.kind, e.key, e.modifiers, e.raw_code, e.ch, e.flags),
            (4, Key::E as i32, 4, 0x45, 0x20AC, 3)
        );
        assert!(e.reason.ptr.is_null());
        let up = to_ffi(SupervisedEvent::Key(KeyEvent {
            down: false,
            repeat: false,
            key: Key::None,
            ch: None,
            ..key
        }));
        assert_eq!((up.kind, up.key, up.ch, up.flags), (4, 0, 0, 0));

        let mut running = to_ffi(SupervisedEvent::Status(Status {
            down: None,
            recoverable: false,
            detail: String::new(),
        }));
        assert_eq!(
            (running.kind, running.hook_state, running.flags),
            (5, FlickSnapHookState::Running as i32, 0)
        );
        assert_eq!(take_reason(&mut running), "");

        let mut stale = to_ffi(SupervisedEvent::Status(Status {
            down: Some(Down::StaleGrant),
            recoverable: true,
            detail: "stale".into(),
        }));
        assert_eq!((stale.hook_state, stale.flags), (FlickSnapHookState::PermissionStale as i32, 4));
        assert_eq!(take_reason(&mut stale), "stale");
    }

    #[test]
    fn every_down_state_maps() {
        use flicksnap_hotkeys::Error as E;
        assert_eq!(hook_state(None), FlickSnapHookState::Running);
        assert_eq!(
            hook_state(Some(&Down::Error(E::PermissionDenied))),
            FlickSnapHookState::PermissionMissing
        );
        assert_eq!(hook_state(Some(&Down::StaleGrant)), FlickSnapHookState::PermissionStale);
        assert_eq!(
            hook_state(Some(&Down::Error(E::HookFailed("x".into())))),
            FlickSnapHookState::Failed
        );
        assert_eq!(hook_state(Some(&Down::Error(E::Unsupported))), FlickSnapHookState::Failed);
        assert_eq!(hook_state(Some(&Down::Error(E::Stopped))), FlickSnapHookState::Stopped);
    }

    #[test]
    fn raw_gestures_are_read_or_refused() {
        let refusal = |key, modifiers| {
            gesture_from_raw(key, modifiers)
                .unwrap_err()
                .0
        };
        assert_eq!(
            gesture_from_raw(Key::F19 as i32, u32::from((Modifiers::CONTROL | Modifiers::ALT).bits())).unwrap(),
            Gesture::new(Key::F19, Modifiers::CONTROL | Modifiers::ALT)
        );
        assert_eq!(refusal(-1, 0), FlickSnapGestureRefusal::InvalidKey);
        assert_eq!(refusal(Key::None as i32, 0), FlickSnapGestureRefusal::NoKey);
        assert_eq!(refusal(Key::A as i32, 0x100), FlickSnapGestureRefusal::InvalidModifiers);
    }

    #[test]
    fn only_gesture_errors_are_refusals() {
        use flicksnap_hotkeys::Error as E;
        assert_eq!(refusal_of(&E::NoKey), Some(FlickSnapGestureRefusal::NoKey));
        assert_eq!(refusal_of(&E::ModifierKey), Some(FlickSnapGestureRefusal::ModifierOnly));
        assert_eq!(
            refusal_of(&E::UnsupportedKey(Key::A)),
            Some(FlickSnapGestureRefusal::UnsupportedKey)
        );
        assert_eq!(refusal_of(&E::Duplicate), Some(FlickSnapGestureRefusal::Duplicate));
        for hook in [E::Unsupported, E::PermissionDenied, E::HookFailed("x".into()), E::Stopped] {
            assert_eq!(refusal_of(&hook), None, "{hook:?}");
        }
    }
}
