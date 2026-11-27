//! The selection overlay pass split out of the old monolithic desktop
//! shader: the selection border + handles (selection.wgsl). It draws small
//! vertex-shader-generated quads over the desktop pass with premultiplied
//! source-over blending, so its GPU cost scales with the border's own
//! pixel area — and it is skipped entirely (no draw call) whenever there
//! is no selection on screen.

use crate::gxi::{self, BlendMode, PipelineDesc, ShaderId};

/// The selection pass's uniform block, written once per frame. MUST stay
/// byte-identical to the `OverlayUniforms` struct in selection.wgsl; the
/// per-field meaning is documented there.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OverlayUniforms {
    /// (viewport_w, viewport_h, dpi_scale, fade).
    pub viewport: [f32; 4],
    pub accent_color: [f32; 4],
    /// Selection in window-local physical px (l, t, r, b); only read
    /// while the selection pass is drawn, so never observed empty.
    pub selection_rect: [f32; 4],
    /// (elapsed_secs, dash_period_px, corner_radius_px, handles_visible).
    pub sel_params: [f32; 4],
}

impl OverlayUniforms {
    pub fn zeroed() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

pub const OVERLAY_UNIFORMS_SIZE: u64 = std::mem::size_of::<OverlayUniforms>() as u64;

/// Vertex count for the selection pass's fixed vertex-shader-generated
/// geometry: 16 quads (4 border slabs, 4 corner patches, 8 handles —
/// unused ones degenerate to zero area).
pub const SELECTION_VERTICES: u32 = 16 * 6;

pub fn create_selection_pipeline(device: &gxi::Device) -> gxi::RenderPipeline {
    device.create_pipeline(&PipelineDesc {
        label: "selection pipeline",
        shader: ShaderId::Selection,
        vertex: None,
        blend: BlendMode::PremultipliedAlpha,
    })
}
