//! The overlay's Windows grab, through the shared GDI grab (`flicksnap_desktop::grab`, the one
//! FlickSnap.Ui and the scrolling driver use): the whole virtual desktop, frozen when the capture
//! starts.

use anyhow::Result;
use flicksnap_desktop::grab::{self, Options};
use flicksnap_shared::geometry::ScreenRect;

/// Raw bitmap product of `capture_desktop`. Public to its own module only;
/// the public `CapturedDesktop` (defined in `system/mod.rs`) wraps this and
/// adds per-monitor metadata.
pub struct DesktopBitmap {
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bounds: ScreenRect,
}

/// The virtual desktop inside `bounds` as raw BGRA, top-down (pixel (0,0) is the screen's
/// top-left, the texture convention the shaders assume, so the upload needs no Y flip).
/// `CAPTUREBLT` composites layered windows in. Alpha is left as GDI wrote it: the GPU uploads
/// this buffer straight into a `Bgra8UnormSrgb` texture and never reads it, so a CPU pass over
/// ~33 MB at 4K would buy nothing.
pub fn capture_desktop(bounds: &ScreenRect) -> Result<DesktopBitmap> {
    let bounds = *bounds;
    if bounds.width() <= 0 || bounds.height() <= 0 {
        bail!("virtual desktop has invalid dimensions: {}x{}", bounds.width(), bounds.height());
    }
    let image = grab::grab(
        bounds,
        Options {
            layered_windows: true,
            opaque: false,
        },
    )?;
    Ok(DesktopBitmap {
        bgra: image.bgra,
        width: image.width,
        height: image.height,
        bounds,
    })
}
