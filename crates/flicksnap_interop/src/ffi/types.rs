//! The shared ABI types (README B1): the string, rectangle and point every area uses, and the
//! enums two areas share.

/// A UTF-8 string handed out by this library: `ptr` is null exactly when `len` is zero, the
/// bytes are not NUL-terminated, and the allocation is Rust's (a boxed slice, so capacity equals
/// `len`). Who frees it depends on the mode the export documents (README B6): mode 1, standalone
/// owned, is freed by `flicksnap_string_free` and by nothing else; mode 2 belongs to a containing
/// allocation freed by its own `_free`; mode 3 is borrowed from a handle and valid until that
/// handle's `_destroy`.
#[repr(C)]
#[derive(Debug)]
pub struct FlickSnapString {
    pub ptr: *mut u8,
    pub len: usize,
}

/// A rectangle in the coordinate space the export documents (physical virtual-desktop pixels on
/// Windows, CoreGraphics global points with a top-left origin on macOS, unless stated).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlickSnapRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A point in the same space as [`FlickSnapRect`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlickSnapPoint {
    pub x: i32,
    pub y: i32,
}

/// Why a gesture cannot be a global hotkey (`flicksnap_gesture_check`, `flicksnap_hotkeys_register`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapGestureRefusal {
    /// The gesture is acceptable.
    None = 0,
    /// The gesture has no key.
    NoKey = 1,
    /// The key value is not in the key table.
    InvalidKey = 2,
    /// The modifiers carry an unknown bit.
    InvalidModifiers = 3,
    /// The key is a modifier on its own.
    ModifierOnly = 4,
    /// The key has no code on this OS, so it could never be seen.
    UnsupportedKey = 5,
    /// An equal gesture is already registered.
    Duplicate = 6,
    /// The OS keeps the gesture for itself.
    Reserved = 7,
}

crate::convert::ffi_enum!(FlickSnapGestureRefusal: i32 {
    None, NoKey, InvalidKey, InvalidModifiers, ModifierOnly, UnsupportedKey, Duplicate, Reserved
});

/// What `flicksnap_window_prepare_overlay` sets up on a window; bit flags (`[Flags]` in C#). A bit
/// that does not apply on an OS is a truthful no-op there.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlickSnapOverlayFlags {
    None = 0,
    /// macOS: the overlay level and collection behaviour, so the window may cover the menu bar,
    /// fullscreen apps and every Space.
    CoverMenuBar = 1,
    /// Windows: a fully opaque layered window (`SetLayeredWindowAttributes(255, LWA_ALPHA)`).
    OpaqueLayered = 2,
    /// Mouse events pass through the window (`WS_EX_TRANSPARENT` / `setIgnoresMouseEvents:`).
    ClickThrough = 4,
    /// The window takes the mouse, over its transparent pixels too (`WS_EX_TRANSPARENT` cleared /
    /// `setIgnoresMouseEvents:NO`). With neither this nor `ClickThrough` the mouse state is left
    /// alone; both together are invalid.
    TakesMouse = 8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layouts_are_pinned() {
        assert_eq!(size_of::<FlickSnapString>(), 16);
        assert_eq!(offset_of!(FlickSnapString, len), 8);
        assert_eq!(size_of::<FlickSnapRect>(), 16);
        assert_eq!(
            [
                offset_of!(FlickSnapRect, x),
                offset_of!(FlickSnapRect, y),
                offset_of!(FlickSnapRect, width),
                offset_of!(FlickSnapRect, height)
            ],
            [0, 4, 8, 12]
        );
        assert_eq!(size_of::<FlickSnapPoint>(), 8);
        assert_eq!(offset_of!(FlickSnapPoint, y), 4);
    }
}
