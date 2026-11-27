use std::borrow::Cow;

use crate::gxi::{self, TexFormat, TextureDesc};
use crate::system::{CapturedCursor, CapturedDesktop, CursorImage};
use flicksnap_shared::geometry::{ScreenPoint, ScreenRect};

/// Uniforms for the desktop background pass (desktop.wgsl): opening
/// fade, selection interior + OCR treatment, and the
/// frozen cursor composite. The selection border and handles have their
/// own pass ([`crate::gpu::overlay::OverlayUniforms`]).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WindowUniforms {
    /// x = grayscale fade factor in [0, 1]; y/z/w spare.
    pub params: [f32; 4],
    /// Selection in window-local physical px (l, t, r, b); empty
    /// (right<=left) when there is no selection. The interior shows the
    /// desktop untouched (or OCR-treated), everything else fades.
    pub selection_rect: [f32; 4],
    /// Cursor rect in window-local physical pixels: [left, top, right, bottom].
    /// Empty (right<=left) when cursor is hidden or off this monitor.
    pub cursor_rect: [f32; 4],
    /// Cursor compositing type: 0=hidden, 1=alpha_blended, 2=masked.
    pub cursor_params: [f32; 4],
    /// x = source-region dim amount 0..1 (ramps with the lift animation),
    /// z = OCR selection desaturation 0..1; y/w spare.
    pub ocr_params: [f32; 4],
}

pub const WINDOW_UNIFORMS_SIZE: u64 = std::mem::size_of::<WindowUniforms>() as u64;

pub struct CursorTextures {
    pub color: gxi::Texture,
    pub mask: gxi::Texture,
    pub cursor_type: u32,
    pub position: ScreenPoint,
    pub hotspot_x: i32,
    pub hotspot_y: i32,
    pub width: u32,
    pub height: u32,
    pub visible: bool,
}

pub struct DesktopSnapshot {
    pub texture: gxi::Texture,
    pub cursor: Option<CursorTextures>,
}

pub fn upload_snapshot(
    device: &gxi::Device,
    queue: &gxi::Queue,
    captured: &CapturedDesktop,
    bounds: ScreenRect,
) -> anyhow::Result<DesktopSnapshot> {
    let width = bounds.width() as u32;
    let height = bounds.height() as u32;
    let max = device.max_texture_dimension_2d();
    if width > max || height > max {
        bail!("display snapshot {width}x{height} exceeds GPU texture limit {max}");
    }
    if width == 0 || height == 0 {
        bail!("display snapshot has zero dimension");
    }

    let pixels = monitor_pixels(captured, bounds).ok_or_else(|| anyhow!("display {bounds:?} is outside the captured desktop"))?;
    let texture = device.try_create_texture_with_data(
        queue,
        &TextureDesc {
            label: "desktop snapshot",
            width,
            height,
            format: TexFormat::Bgra8Unorm,
        },
        &pixels,
    )?;

    let cursor = captured.cursor.as_ref().and_then(|c| {
        if !c.visible {
            return None;
        }
        upload_cursor_textures(device, queue, c)
    });

    Ok(DesktopSnapshot {
        texture,
        cursor,
    })
}

fn monitor_pixels(captured: &CapturedDesktop, bounds: ScreenRect) -> Option<Cow<'_, [u8]>> {
    if captured.bounds == bounds {
        return Some(Cow::Borrowed(&captured.bgra));
    }
    let (pixels, _, _, covered) = crate::image_extract::extract_selection_bgra(bounds, captured)?;
    (covered == bounds).then_some(Cow::Owned(pixels))
}

fn create_small_texture(device: &gxi::Device, queue: &gxi::Queue, bgra: &[u8], width: u32, height: u32, label: &str) -> gxi::Texture {
    device.create_texture_with_data(
        queue,
        &TextureDesc {
            label,
            width,
            height,
            format: TexFormat::Bgra8Unorm,
        },
        bgra,
    )
}

pub fn create_placeholder_cursor_texture(device: &gxi::Device, queue: &gxi::Queue) -> gxi::Texture {
    create_small_texture(device, queue, &[0, 0, 0, 0], 1, 1, "cursor placeholder")
}

fn upload_cursor_textures(device: &gxi::Device, queue: &gxi::Queue, cursor: &CapturedCursor) -> Option<CursorTextures> {
    match &cursor.image {
        CursorImage::AlphaBlended {
            bgra,
            width,
            height,
        } => {
            if *width == 0 || *height == 0 || bgra.is_empty() {
                return None;
            }
            let color = create_small_texture(device, queue, bgra, *width, *height, "cursor color");
            let mask = create_small_texture(device, queue, &[0, 0, 0, 0], 1, 1, "cursor mask placeholder");
            Some(CursorTextures {
                color,
                mask,
                cursor_type: 1,
                position: cursor.position,
                hotspot_x: cursor.hotspot_x,
                hotspot_y: cursor.hotspot_y,
                width: *width,
                height: *height,
                visible: true,
            })
        }
        CursorImage::Masked {
            and_mask_bgra,
            xor_color_bgra,
            width,
            height,
        } => {
            if *width == 0 || *height == 0 || and_mask_bgra.is_empty() || xor_color_bgra.is_empty() {
                return None;
            }
            let color = create_small_texture(device, queue, xor_color_bgra, *width, *height, "cursor xor color");
            let mask = create_small_texture(device, queue, and_mask_bgra, *width, *height, "cursor and mask");
            Some(CursorTextures {
                color,
                mask,
                cursor_type: 2,
                position: cursor.position,
                hotspot_x: cursor.hotspot_x,
                hotspot_y: cursor.hotspot_y,
                width: *width,
                height: *height,
                visible: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicksnap_shared::geometry::RectExt;

    #[test]
    fn monitor_upload_crops_wide_desktop_with_negative_origin() {
        let bounds = ScreenRect::from_xy_size(-9000, -10, 18000, 2);
        let mut bgra = vec![0; 18000 * 2 * 4];
        bgra[0] = 11;
        bgra[9000 * 4] = 22;
        bgra[(18000 + 9000) * 4] = 33;
        let desktop = CapturedDesktop {
            bgra,
            width: 18000,
            height: 2,
            bounds,
            cursor: None,
        };
        let left = monitor_pixels(&desktop, ScreenRect::from_xy_size(-9000, -10, 9000, 2)).unwrap();
        let right = monitor_pixels(&desktop, ScreenRect::from_xy_size(0, -10, 9000, 2)).unwrap();
        assert_eq!(left.len(), 9000 * 2 * 4);
        assert_eq!((left[0], right[0], right[9000 * 4]), (11, 22, 33));
        assert!(matches!(monitor_pixels(&desktop, bounds), Some(Cow::Borrowed(_))));
        assert!(monitor_pixels(&desktop, ScreenRect::from_xy_size(8999, -10, 2, 2)).is_none());
    }
}
