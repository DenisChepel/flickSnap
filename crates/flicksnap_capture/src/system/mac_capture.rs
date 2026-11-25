use std::sync::Mutex;

use anyhow::Result;
use objc2_core_foundation::CFRetained;
#[allow(deprecated)]
use objc2_core_graphics::{CGDisplayCreateImage, CGImage, CGWindowImageOption, CGWindowListCreateImage, CGWindowListOption};

use crate::system::MonitorInfo;
use flicksnap_desktop::{grab, CGPoint, CGRect, CGSize, ImageBytes};
use flicksnap_shared::geometry::ScreenRect;

pub struct DesktopBitmap {
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Grab each display's part of the selection in its own backing scale.
#[allow(deprecated)]
pub fn capture_region(bounds: ScreenRect, monitors: &[MonitorInfo]) -> Result<DesktopBitmap> {
    let width = bounds.width() as usize;
    let height = bounds.height() as usize;
    let mut bgra = vec![0; width * height * 4];
    let mut captured = false;

    for monitor in monitors {
        let Some(part) = bounds.intersection(&monitor.bounds) else {
            continue;
        };
        let origin = monitor.screen_to_logical(part.origin);
        let size = monitor.physical_to_logical_size(part.width() as u32, part.height() as u32);
        let rect = CGRect::new(CGPoint::new(origin.x, origin.y), CGSize::new(size.width, size.height));
        let image = CGWindowListCreateImage(rect, CGWindowListOption::OptionOnScreenOnly, 0, CGWindowImageOption::BestResolution)
            .ok_or_else(|| anyhow!("could not capture the selected region"))?;
        let pixels = grab::from_cg_image(&image, false)?;

        let w = part.width() as usize;
        let h = part.height() as usize;
        let stride = pixels.stride();
        let dst_x = (part.min_x() - bounds.min_x()) as usize;
        let dst_y = (part.min_y() - bounds.min_y()) as usize;
        let src_w = pixels.width as usize;
        let src_h = pixels.height as usize;
        if src_w == 0 || src_h == 0 {
            bail!("screen capture returned an empty bitmap");
        }
        for row in 0..h {
            let src_y = row * src_h / h;
            let src = &pixels.bgra[src_y * stride..][..src_w * 4];
            let dst = &mut bgra[((dst_y + row) * width + dst_x) * 4..][..w * 4];
            if src_w == w {
                dst.copy_from_slice(src);
            } else {
                let (dst_pixels, _) = dst.as_chunks_mut::<4>();
                for (col, pixel) in dst_pixels.iter_mut().enumerate() {
                    let x = col * src_w / w * 4;
                    pixel.copy_from_slice(&src[x..x + 4]);
                }
            }
        }
        captured = true;
    }
    if !captured {
        bail!("the selection is outside the displays");
    }
    Ok(DesktopBitmap {
        bgra,
        width: width as u32,
        height: height as u32,
    })
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
/// positioning. Re-enumerates display IDs (cheap CG call) and composites
/// each display into a single BGRA buffer. Assumes Screen Recording is
/// already granted — `main()` gates on that before anything runs.
pub fn capture_bitmap(monitors: &[MonitorInfo]) -> Result<DesktopBitmap> {
    let vd = super::virtual_desktop_bounds(monitors);
    let vd_w = vd.width() as usize;
    let vd_h = vd.height() as usize;

    if vd_w == 0 || vd_h == 0 {
        bail!("virtual desktop has invalid dimensions: {}x{}", vd_w, vd_h);
    }

    let mut bgra = vec![0u8; vd_w * vd_h * 4];

    let display_ids = flicksnap_desktop::active_display_ids().map_err(|e| anyhow!("CGGetActiveDisplayList failed: {:?}", e))?;

    // Zip monitors with display_ids; truncate to the shorter list in
    // case a display connected/disconnected between enumeration and capture.
    let count = monitors.len().min(display_ids.len());
    let mut captured = 0;
    let mut display_images: Vec<(ScreenRect, DisplayImage)> = Vec::with_capacity(count);
    for i in 0..count {
        let monitor = &monitors[i];
        // Deprecated in favor of ScreenCaptureKit, which the capture has not
        // moved to.
        #[allow(deprecated)]
        let image = match CGDisplayCreateImage(display_ids[i]) {
            Some(img) => img,
            None => {
                warn!("CGDisplayCreateImage returned null for display {} — skipping", display_ids[i]);
                continue;
            }
        };
        // The layout rule every CoreGraphics reader shares (grab::cg_layout). Screen captures are
        // BGRA and their rows are copied straight in; any other 32-bit layout is converted first
        // by the shared grab, and a format it cannot convert skips the display.
        let layout = match grab::cg_layout(&image) {
            Ok(layout) => layout,
            Err(e) => {
                warn!("Display {} image: {e}, skipping", display_ids[i]);
                continue;
            }
        };
        let Some(pixels) = ImageBytes::of(&image) else {
            warn!("Display {} image has no readable pixels, skipping", display_ids[i]);
            continue;
        };
        let converted;
        let (src, bpr, img_w, img_h) = if layout == grab::Layout::Bgra {
            (pixels.bytes(), pixels.bytes_per_row, pixels.width, pixels.height)
        } else {
            converted = match grab::from_cg_image(&image, false) {
                Ok(converted) => converted,
                Err(e) => {
                    warn!("Display {} image: {e}, skipping", display_ids[i]);
                    continue;
                }
            };
            (
                &converted.bgra[..],
                converted.stride(),
                converted.width as usize,
                converted.height as usize,
            )
        };

        let dest_x = (monitor.bounds.min_x() - vd.min_x()) as usize;
        let dest_y = (monitor.bounds.min_y() - vd.min_y()) as usize;

        let copy_w = img_w.min(monitor.bounds.width() as usize);
        let copy_h = img_h.min(monitor.bounds.height() as usize);

        for row in 0..copy_h {
            let src_start = row * bpr;
            let src_end = src_start + copy_w * 4;
            let dst_start = ((dest_y + row) * vd_w + dest_x) * 4;
            let dst_end = dst_start + copy_w * 4;

            if src_end <= src.len() && dst_end <= bgra.len() {
                bgra[dst_start..dst_end].copy_from_slice(&src[src_start..src_end]);
            }
        }
        captured += 1;

        // Park the image for `render::window`'s backing layer instead of
        // letting it drop. Only an exact 1:1 match is parked: when the
        // captured image is a different size than the monitor rect the
        // composite path above takes a top-left sub-rect of it, and a layer
        // handed the whole image would resize it instead — different pixels.
        if img_w == monitor.bounds.width() as usize && img_h == monitor.bounds.height() as usize {
            display_images.push((monitor.bounds, DisplayImage(image)));
        }
    }

    if let Ok(mut stash) = DISPLAY_IMAGES.lock() {
        *stash = display_images;
    }

    // A per-display null is tolerable (the region stays black), but zero captures
    // means the overlay would show a fully black desktop — most likely a TCC verdict
    // that differs from the preflight `main()` ran. Fail so the shell reports it.
    if captured == 0 && count > 0 {
        bail!("CGDisplayCreateImage produced no image for any of {count} display(s) — Screen Recording may be denied for this process");
    }

    Ok(DesktopBitmap {
        bgra,
        width: vd_w as u32,
        height: vd_h as u32,
    })
}
