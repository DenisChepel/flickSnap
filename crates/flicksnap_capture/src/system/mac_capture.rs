use std::sync::Mutex;

use anyhow::Result;
use objc2_core_foundation::CFRetained;
#[allow(deprecated)]
use objc2_core_graphics::{CGDisplayCreateImage, CGImage};

use crate::system::MonitorInfo;
use flicksnap_desktop::{grab, ImageBytes};
use flicksnap_shared::geometry::ScreenRect;

pub struct DesktopBitmap {
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// A `CGImage` parked for the main thread. `CGImage` is a bare CF pointer, so
/// Rust considers it neither `Send` nor `Sync` and it cannot ride along inside
/// `CapturedDesktop`, which is `Arc`'d from the screenshot thread to the main
/// thread and to every render worker. The objects themselves are immutable and
/// refcounted atomically, which is precisely the case `unsafe impl Send` is for.
struct DisplayImage(CFRetained<CGImage>);
unsafe impl Send for DisplayImage {}

/// Per-display images from the most recent [`capture_bitmap`], keyed by the
/// monitor rect they cover. `capture_bitmap` composites each display into the
/// virtual-desktop buffer and would otherwise drop these; the overlay window
/// for that display wants exactly this image as its backing-layer contents, and
/// reconstructing it from the composite costs a row-by-row crop plus a
/// `CGBitmapContextCreateImage` — both on the main thread, per monitor, while
/// the user is waiting for the first frame.
static DISPLAY_IMAGES: Mutex<Vec<(ScreenRect, DisplayImage)>> = Mutex::new(Vec::new());

/// Claim the untouched capture of the display covering `bounds`, if there was
/// one. Taking rather than borrowing means the backing store (tens of MB for a
/// 4K display) is released as soon as the layer that wants it has retained it.
///
/// Ordering is guaranteed by the screenshot latch: nothing has a
/// `CapturedDesktop` to ask about until `capture_bitmap` has returned.
pub fn take_display_image(bounds: ScreenRect) -> Option<CFRetained<CGImage>> {
    let mut images = DISPLAY_IMAGES.lock().ok()?;
    let idx = images
        .iter()
        .position(|(b, _)| *b == bounds)?;
    Some(images.swap_remove(idx).1 .0)
}

/// Whether the process may capture the screen. Reported to `main()` so the
/// capturer can refuse to start; it never prompts and never opens System
/// Settings, because Screen Recording is the shell's to explain and request
/// (Settings → System → Permissions).
pub fn has_screen_recording_permission() -> bool {
    flicksnap_platform::permissions::screen_recording()
}

/// Capture the desktop bitmap using pre-enumerated monitors for
/// positioning and captures each display by its ID. Assumes Screen Recording is
/// already granted — `main()` gates on that before anything runs.
pub fn capture_bitmap(monitors: &[MonitorInfo]) -> Result<DesktopBitmap> {
    let vd = super::virtual_desktop_bounds(monitors);
    let vd_w = vd.width() as usize;
    let vd_h = vd.height() as usize;

    if vd_w == 0 || vd_h == 0 {
        bail!("virtual desktop has invalid dimensions: {}x{}", vd_w, vd_h);
    }

    let mut bgra = vec![0u8; vd_w * vd_h * 4];

    let mut display_images = Vec::with_capacity(monitors.len());
    for monitor in monitors {
        let id = monitor.display_id;
        // Deprecated in favor of ScreenCaptureKit.
        #[allow(deprecated)]
        let image = CGDisplayCreateImage(id).ok_or_else(|| anyhow!("no image for display {id}"))?;
        let layout = grab::cg_layout(&image).map_err(|e| anyhow!("display {id}: {e}"))?;

        let converted;
        let pixels;
        let (src, bpr, img_w, img_h) = if layout == grab::Layout::Bgra {
            pixels = ImageBytes::of(&image).ok_or_else(|| anyhow!("display {id} has no readable pixels"))?;
            (pixels.bytes(), pixels.bytes_per_row, pixels.width, pixels.height)
        } else {
            converted = grab::from_cg_image(&image, false).map_err(|e| anyhow!("display {id}: {e}"))?;
            (
                &converted.bgra[..],
                converted.stride(),
                converted.width as usize,
                converted.height as usize,
            )
        };

        let dest_x = (monitor.bounds.min_x() - vd.min_x()) as usize;
        let dest_y = (monitor.bounds.min_y() - vd.min_y()) as usize;
        let width = monitor.bounds.width() as usize;
        let height = monitor.bounds.height() as usize;
        if img_w < width || img_h < height {
            bail!("display {id} changed size during capture ({img_w}x{img_h}, expected {width}x{height})");
        }
        for row in 0..height {
            let src_start = row * bpr;
            let dst_start = ((dest_y + row) * vd_w + dest_x) * 4;
            let line = src
                .get(src_start..src_start + width * 4)
                .ok_or_else(|| anyhow!("incomplete capture of display {id}"))?;
            bgra[dst_start..dst_start + width * 4].copy_from_slice(line);
        }

        // A layer can use the original image directly only at a 1:1 size.
        if img_w == width && img_h == height {
            display_images.push((monitor.bounds, DisplayImage(image)));
        }
    }

    if let Ok(mut stash) = DISPLAY_IMAGES.lock() {
        *stash = display_images;
    }

    Ok(DesktopBitmap {
        bgra,
        width: vd_w as u32,
        height: vd_h as u32,
    })
}
