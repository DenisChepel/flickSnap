use std::sync::atomic::AtomicUsize;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::app::App;
use crate::gxi;
use crate::render::protocol::{CycleParams, WorkerInput};
use crate::render::worker::{self, RenderWorkerParams, WorkerSetup};
use crate::settings::{CaptureMode, CapturerSettings};
use crate::sync::{Latch, VisibleLatch};
use crate::system::{CapturedCursor, CapturedDesktop, MonitorInfo, SystemInterop, WindowTarget};
use crate::telemetry::startup::StartupTimings;
use flicksnap_shared::geometry::ScreenPointF;

pub struct CaptureSession {
    app: App,
    timings: Arc<StartupTimings>,
}

impl CaptureSession {
    /// `t_start` is taken by `main` when this capture leaves standby. Process
    /// loading, the logger and Sentry stay warm and are intentionally excluded;
    /// every display/GPU/window resource constructed below remains measured.
    pub fn new(settings: Arc<CapturerSettings>, t_start: Instant) -> anyhow::Result<Self> {
        let monitors = SystemInterop::all_monitors();
        if monitors.is_empty() {
            anyhow::bail!("no monitors detected; nothing to render to");
        }
        // Sampled before the timings exist — the worker count sizes them, and
        // enumerating the monitors is what produces that count.
        let monitors_enumerated = t_start.elapsed();
        let timings = Arc::new(StartupTimings::new(t_start, monitors.len()));
        timings.mark_monitors_enumerated(monitors_enumerated);

        // Everything the desktop capture needs, gathered before anything else:
        // the capture is the longest pole in startup and the main thread blocks
        // on it below, so it is spawned FIRST and the GPU instance and the
        // render workers are built beside it instead of in front of it. Only the
        // cursor and the monitor list are genuine prerequisites; the worker
        // channels it also wants arrive later over `worker_channels_latch`.
        let initial_mouse = SystemInterop::get_mouse_position(&monitors);
        let initial_mouse_f = ScreenPointF::new(initial_mouse.x as f32, initial_mouse.y as f32);
        let ready_count = Arc::new(AtomicUsize::new(0));
        let visible_latch = Arc::new(VisibleLatch::new());

        // Only the explicit Active Window mode consults the window system, once,
        // before this cycle can create or focus any overlay windows.
        let active_window: Option<WindowTarget> = if settings.capture_mode == CaptureMode::Window {
            SystemInterop::active_window_target(&monitors, settings.rounded_window_corners)
        } else {
            None
        };

        // The screen freezes here: the picture every capture is picked and
        // drawn over is taken before anything else is built.
        let (screenshot_latch, worker_channels) = spawn_screenshot_job(ScreenshotJobParams {
            monitors: monitors.clone(),
            cursor: SystemInterop::capture_cursor(&monitors),
            accent_color: settings.accent_color,
            ready_count: ready_count.clone(),
            visible_latch: visible_latch.clone(),
            timings: timings.clone(),
        });

        let instance = gxi::Instance::new();
        timings.mark_instance_created();
        timings.mark_initialize();

        let worker_failed = Arc::new(AtomicUsize::new(0));
        let worker_setups = spawn_render_workers(&monitors, &instance, &timings, &worker_failed);
        timings.mark_workers_spawned();

        worker_channels.set(
            worker_setups
                .iter()
                .map(|s| s.input_tx.clone())
                .collect(),
        );

        // Deliberately NOT waited on here. The main thread used to block on this
        // latch (~50 ms — the single largest delta on the startup critical path),
        // serializing the event loop, every window/surface creation and every
        // surface configure behind the capture. `App` now takes the latch itself
        // and picks the buffer up inside the event loop
        // (`app.rs::try_pick_up_screenshot`), so all of that setup runs BESIDE
        // the capture instead of after it. What still waits for the bitmap waits
        // in the right place: the macOS frozen-desktop backdrop is installed —
        // and the windows ordered front — only once the buffer lands, and the
        // workers cannot reach frame 0 earlier anyway (`BeginCycle` carries the
        // snapshot and is broadcast by the capture thread).
        //
        // The old 30 s bound survives as `App`'s screenshot deadline, enforced in
        // the pickup: a wedged CG capture call must still end as a clean non-zero
        // exit (`App::fatal_result`), never a process idling forever with nothing
        // on screen and no way to close it from the shell.
        let app = App::new(
            settings,
            timings.clone(),
            instance,
            monitors,
            initial_mouse_f,
            worker_setups,
            screenshot_latch,
            active_window,
            ready_count,
            visible_latch,
            worker_failed,
        );

        Ok(Self {
            app,
            timings,
        })
    }

    /// Shared with `main` so the stages that happen after the session is
    /// built — the event loop, entering `run_app` — land on the same clock.
    pub fn timings(&self) -> &Arc<StartupTimings> {
        &self.timings
    }

    pub fn into_app(self) -> App {
        self.app
    }
}

fn spawn_render_workers(
    monitors: &[MonitorInfo],
    instance: &gxi::Instance,
    startup: &Arc<StartupTimings>,
    failed_count: &Arc<AtomicUsize>,
) -> Vec<WorkerSetup> {
    monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            worker::spawn_render_worker(RenderWorkerParams {
                monitor: m.clone(),
                monitor_index: i,
                instance: instance.clone(),
                startup: startup.clone(),
                failed_count: failed_count.clone(),
            })
        })
        .collect()
}

/// Inputs for the screenshot job, grouped into a struct to keep the
/// spawner's argument list manageable.
struct ScreenshotJobParams {
    pub monitors: Vec<MonitorInfo>,
    pub cursor: Option<CapturedCursor>,
    pub accent_color: [f32; 4],
    pub ready_count: Arc<AtomicUsize>,
    pub visible_latch: Arc<VisibleLatch>,
    /// This cycle's debug timings; the screenshot offsets are recorded here
    /// and the `Arc` rides to every worker on `CycleParams`.
    pub timings: Arc<StartupTimings>,
}

const SCREENSHOT_STAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// Capture the desktop bitmap on a background thread and publish it through the
/// returned latch; then, once `worker_channels_latch` delivers the render
/// workers' senders, broadcast `BeginCycle` (snapshot + per-cycle params) to
/// every worker.
///
/// The two stages are split so the capture can be kicked off before the workers
/// exist — it is one of the longest steps in startup, so it starts as early as the cursor and monitor list
/// allow. See the ordering note in `CaptureSession::new`.
type ScreenshotLatch = Arc<Latch<Arc<CapturedDesktop>>>;
type WorkerInputs = Arc<Latch<Vec<mpsc::Sender<WorkerInput>>>>;

fn spawn_screenshot_job(params: ScreenshotJobParams) -> (ScreenshotLatch, WorkerInputs) {
    let ScreenshotJobParams {
        monitors,
        cursor,
        accent_color,
        ready_count,
        visible_latch,
        timings,
    } = params;

    let screenshot_latch = Arc::new(Latch::new());
    let worker_channels_latch: WorkerInputs = Arc::new(Latch::new());
    let latch = screenshot_latch.clone();
    let channels_latch = worker_channels_latch.clone();
    std::thread::Builder::new()
        .name("screenshot".into())
        .spawn(move || {
            timings
                .background
                .screenshot_start
                .set_once(timings.t_start.elapsed());
            let captured = Arc::new(SystemInterop::capture_desktop_bitmap(&monitors, cursor));
            latch.set(captured.clone());
            timings
                .background
                .screenshot
                .set_once(timings.t_start.elapsed());

            let Some(channels) = channels_latch.wait_timeout(SCREENSHOT_STAGE_TIMEOUT) else {
                error!("screenshot: worker channels never arrived; no cycle broadcast");
                return;
            };
            let cycle = Arc::new(CycleParams {
                snapshot: captured,
                accent_color,
                ready_count,
                visible_latch: visible_latch.clone(),
            });
            for tx in channels.iter() {
                let _ = tx.send(WorkerInput::BeginCycle(cycle.clone()));
            }
        })
        .expect("spawn screenshot thread");
    (screenshot_latch, worker_channels_latch)
}
