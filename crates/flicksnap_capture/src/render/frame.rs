use std::time::{Duration, Instant};

use crate::gpu::overlay::SELECTION_VERTICES;
use crate::gpu::WindowGpu;
use crate::gxi::{self, AcquireResult};
use crate::render::desktop::{OverlayVisibility, SnapshotState};
use crate::telemetry::perf::PerfSample;
use crate::ui::gpu::UiRenderer;

/// What [`draw_once`] wants the render loop to do next.
pub(crate) enum DrawStatus {
    Presented(PerfSample),
    /// A timeout or an occluded window can recover on the next frame.
    Retry,
    /// Rendering cannot continue on this surface.
    Failed,
}

pub(crate) fn draw_once(
    surface: &mut gxi::Surface,
    gpu: &WindowGpu,
    surface_size: (u32, u32),
    snapshot_state: &SnapshotState,
    // Which overlay feature passes draw this frame (decided CPU-side in
    // `update_uniforms`): a hidden feature costs no GPU time at all.
    overlay: OverlayVisibility,
    // The panel appears when its deferred GPU setup is ready.
    mut ui_renderer: Option<&mut UiRenderer>,
    gpu_timing: Option<&gxi::GpuTimings>,
) -> DrawStatus {
    let t_start = Instant::now();
    let mut frame = match surface.acquire(gpu_timing) {
        AcquireResult::Frame(f) => f,
        AcquireResult::Skip => return DrawStatus::Retry,
        // Metal's occlusion check returns immediately. Pace retries
        // while the compositor keeps the window occluded.
        AcquireResult::Occluded => {
            std::thread::sleep(Duration::from_millis(10));
            return DrawStatus::Retry;
        }
        AcquireResult::Failed => return DrawStatus::Failed,
    };
    // The sample's wait bucket is the swapchain acquire alone (the vsync
    // block); the encoder/render-pass setup `acquire` also did lands in
    // the draw bucket below, matching the pre-gxi bucketing.
    let wait = frame.acquire_wait();

    if let Some(ui) = ui_renderer.as_mut() {
        ui.prepare(&gpu.device, &gpu.queue, surface_size);
    }

    // Pass order is the painter's stack, background to front: desktop
    // (snapshot + cursor + region fade) → selection border + handles → UI
    // chrome.
    frame.set_pipeline(&gpu.pipeline);
    frame.set_bind_group(0, &snapshot_state.bind_group);
    frame.draw(0..3, 0..1);
    // `selection` is None while the deferred build is in flight.
    if let (true, Some(selection)) = (overlay.selection, gpu.selection.as_ref()) {
        frame.set_pipeline(selection);
        frame.set_bind_group(0, &snapshot_state.selection_bind_group);
        frame.draw(0..SELECTION_VERTICES, 0..1);
    }
    if let Some(ui) = ui_renderer.as_deref() {
        ui.draw(&mut frame);
    }

    // `present` submits and presents; it hands back the time spent in the
    // final compositor handoff so the sample's draw/present split matches
    // the pre-gxi bucketing (submit counts as draw work).
    let Some(present) = frame.present(gpu_timing) else {
        return DrawStatus::Failed;
    };
    let draw = t_start
        .elapsed()
        .saturating_sub(wait)
        .saturating_sub(present);

    DrawStatus::Presented(PerfSample {
        wait,
        draw,
        present,
        overall: Duration::ZERO,
        gpu: None,
    })
}
