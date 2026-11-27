use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use unicode_script::{Script, UnicodeScript};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
#[cfg(windows)]
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{CursorIcon, Window, WindowId};

use crate::capture_output::{copy_text_to_clipboard, copy_to_clipboard, ActionResult, Screenshot};
use crate::draw::{DrawState, Marks};
use crate::image_extract::extract_selection_bgra;
use crate::interaction::{InteractionController, InteractionEffects, InteractionState, OcrNotice, OcrNoticeKind, OcrState};
use crate::ocr::{self, OcrError, OcrOutcome, OcrRequest};
use crate::render::window::{set_hardware_cursor_visible, WindowHandle, WindowSet};
use crate::render::worker::WorkerSetup;
use crate::selection::{dpi_at_point, hit_test, move_and_crop, resize_with_clamp, DragMode, Hittest};
use crate::session_output::{
    write_image_search_action, write_ocr_upload_action, write_scroll_action, write_session, write_share_action, write_video_action,
    SessionAction,
};
use crate::settings::{CaptureMode, CapturerSettings};
use crate::sync::{Latch, VisibleLatch};
use crate::system::{CapturedDesktop, MonitorInfo, SystemInterop, WindowTarget};
use crate::telemetry::startup::StartupTimings;
use crate::ui::command::Command;
use crate::ui::components::panel;
use crate::ui::egui_host::{EguiHosts, FontInstall, PanelOutcome, SyncArgs};
use crate::ui::shared::{active_panel_set, UiMonitor, UiSharedState};
use flicksnap_shared::geometry::{
    to_screen_point, RectExt, ScreenPoint, ScreenPointF, ScreenRect, ScreenRectExt, ScreenRectRounded, WindowPoint,
};

/// One capture cycle's state. The persistent process drops this whole graph
/// before returning to standby, including its GPU instance, monitors,
/// windows, surfaces and render workers.
pub struct App {
    windows: WindowSet,
    monitors: Vec<MonitorInfo>,
    /// `monitors` mapped to the UI-state shape, built once — cloned into
    /// every `UiSharedState` instead of re-collected per mouse event.
    ui_monitors: Arc<[UiMonitor]>,
    vd_bounds: ScreenRect,
    startup: Arc<StartupTimings>,
    instance: crate::gxi::Instance,
    /// Consumed in resumed() — each one gets a window + surface handoff.
    worker_setups: Option<Vec<WorkerSetup>>,
    /// Window creation failures and unexpected render worker exits.
    worker_failed: Arc<AtomicUsize>,
    cycle: Option<CaptureCycle>,
    /// A capture or preview startup failure. `run_app` has no error channel of its
    /// own, so the failure is parked here and `fatal_result` hands it back
    /// to `main` after the loop exits, preserving the non-zero exit code
    /// (and the sentry capture) the old blocking-wait `?` produced.
    fatal: Option<anyhow::Error>,
    /// How the cycle ended, once `finish_cycle` ran: the one-shot run's `done`
    /// says `cancelled` or `finished` from it.
    finished: Option<CycleAction>,
}

/// All state for the single capture this process serves. Built in
/// [`App::new`] and dropped by [`App::finish_cycle`] on the way out.
pub struct CaptureCycle {
    settings: Arc<CapturerSettings>,
    /// `None` until the desktop capture lands — `App` is constructed with the
    /// screenshot still in flight, and `try_pick_up_screenshot` fills this in
    /// from `screenshot_latch`. Every consumer already tolerates `None` (the
    /// action dispatches fail with "no buffer", `broadcast_ui_state` takes an
    /// `Option`), and none of them can fire before the overlay is visible,
    /// which the show gate holds back until the buffer has long arrived.
    desktop_buffer: Option<Arc<CapturedDesktop>>,
    /// Source of `desktop_buffer`, set by the screenshot thread.
    screenshot_latch: Arc<Latch<Arc<CapturedDesktop>>>,
    /// When to give up on `screenshot_latch`. Carries the old blocking
    /// wait's 30 s bound through the event loop: a wedged CG capture call
    /// must end as a clean failing exit, not a process idling forever with
    /// hidden windows and no way to close it from the shell.
    screenshot_deadline: Instant,
    /// Last cursor icon set per window, to skip redundant `set_cursor`
    /// calls on every mouse move.
    last_cursor: HashMap<WindowId, CursorIcon>,
    input: InteractionState,
    pending_show: Option<PendingShow>,
    /// When launched with `--capture-mode screen|window`, the mode to
    /// pre-select once the overlay is up. Consumed (set to `None`) after the
    /// one-time pre-selection fires. `None` for free-region mode.
    pending_preselect: Option<WindowTarget>,
    /// One-shot guard for `--video` mode: set the first time a selection
    /// becomes captured so `Command::Video` is auto-dispatched exactly
    /// once, regardless of which capture path (drag / keyboard / preselect)
    /// fired (DESIGN §3.3).
    video_dispatched: bool,
    /// One-shot guard for `--share` mode, the exact twin of
    /// [`Self::video_dispatched`]: set the first time a selection becomes
    /// captured so `Command::Share` is auto-dispatched exactly once no
    /// matter which capture path (drag / keyboard / preselect) fired. Its
    /// own flag rather than a shared one because the two modes are
    /// mutually exclusive at the CLI but nothing downstream depends on
    /// that, and a single guard would silently couple them if that ever
    /// changed.
    share_dispatched: bool,
    /// Monotonic per-cycle OCR request id. Bumped on every dispatch AND on
    /// every BACK/cancel, so a late result from a superseded request is
    /// discarded on pickup: BACK leaves the same cycle alive, so the result
    /// cannot be told apart from a current one any other way.
    ocr_req: u64,
    /// The in-flight recognition job, if any.
    ocr_job: Option<OcrJob>,
    /// A result being held until its release time: `(release_secs, result)`
    /// where `release_secs` is measured on the Scanning anchor. Computed at
    /// pickup by `anim::scan_release_secs`: failures wait only the
    /// anti-flash floor (warm OCR is 6-35 ms), while a successful outcome
    /// is held until the looping sweep's current pass WRAPS — the one
    /// instant the band is fully off-screen — so the Lifted reveal pass
    /// (fresh anchor, band entering from the top) continues it seamlessly.
    ocr_ready: Option<(f32, Result<OcrOutcome, OcrError>)>,
    /// Defence against the panel's synchronous set swap turning one
    /// physical double-click into two different commands — see
    /// [`PanelSwapGuard`].
    panel_swap: PanelSwapGuard,
    /// The per-monitor egui contexts. On the cycle rather than on `App` so
    /// the free `broadcast_ui_state` can drive them through disjoint field
    /// borrows.
    egui: EguiHosts,
}

/// An in-flight OCR recognition: (request id, one-shot result latch,
/// cooperative cancel flag read by the worker around its blocking call).
type OcrJob = (u64, Arc<Latch<Result<OcrOutcome, OcrError>>>, Arc<AtomicBool>);

struct PendingShow {
    ready_count: Arc<AtomicUsize>,
    expected: usize,
    visible_latch: Arc<VisibleLatch>,
    /// When the gate started waiting, for [`SHOW_GATE_TIMEOUT`].
    since: Instant,
}

/// How long the show gate waits for the displays before canceling startup.
///
/// Load-bearing since the overlay windows are ordered front *before* frame 0
/// (the macOS `SurfaceError::Occluded` workaround): from window creation
/// onwards the user is looking at a frozen still of their desktop. A worker
/// that wedges without panicking — the `nextDrawable` stall of
/// gfx-rs/wgpu#8309, or a hung shader compile — bumps neither `ready_count`
/// nor the render failure guard, so an unbounded gate would leave that frozen still on
/// screen forever with the live desktop unreachable and no way to dismiss it.
/// Before the early order-front the same hang merely showed nothing.
///
/// Generous on purpose: a cold shader compile on a slow discrete GPU is
/// legitimately hundreds of ms, and firing early costs a monitor its overlay.
const SHOW_GATE_TIMEOUT: Duration = Duration::from_secs(5);

/// How a capture cycle ended. Logged only; the action the shell dispatches
/// on is written separately by `session_output::SessionAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleAction {
    Edit,
    Upload,
    Video,
    /// `--share` mode: the picked region was handed to the shell to mirror
    /// into a window a meeting app can share.
    Share,
    Scroll,
    /// The capture strip's SEARCH: the cropped image was handed to the
    /// shell to look up with a reverse image search.
    SearchImage,
    Copy,
    Save,
    /// OCR mode's COPY: the recognized text went to the clipboard.
    OcrCopy,
    /// OCR mode's SEARCH: a web search for the recognized text was opened.
    OcrSearch,
    /// OCR mode's UPLOAD: the recognized text was handed to the shell to
    /// upload as a paste.
    OcrUpload,
    Canceled,
}

// ── Free helpers over (app, cycle) state ────────────────────────────
// Plain functions (not methods) so callers holding a `&mut CaptureCycle`
// split off `self.cycle` can still use them alongside the `App` fields.

fn broadcast_mouse_state(windows: &WindowSet, input: &InteractionState) {
    for h in windows.values() {
        h.update_mouse_state(input.selection, input.selection_radius, input.mouse_down, input.captured);
    }
}

/// Take the overlay off screen on the way out of a cycle, handing our
/// foreground rights back to the shell on the way.
///
/// Every action dispatch hides before it writes its payload, and
/// `finish_cycle` hides again behind it — so this is where the capturer
/// stops owning a foreground window, and therefore the last moment
/// `AllowSetForegroundWindow` can succeed. The shell almost always opens
/// something straight afterwards (an editor, the recorder, the scrolling
/// capture driver) and cannot raise any of it without rights it no longer
/// holds. Handing them back costs nothing when nobody needs them: the grant
/// is consumed by a single `SetForegroundWindow` and otherwise expires.
///
/// Deliberately not folded into [`WindowSet::hide_all`], which also serves
/// the transient hides that a retry dialog re-shows from.
fn hide_overlay_for_action(windows: &WindowSet) {
    SystemInterop::hand_foreground_to_shell();
    windows.hide_all();
}

/// Put an overlay window on screen without activating it or making it key,
/// before its worker has drawn anything into it.
///
/// macOS only, and a correctness fix rather than a latency one: the metal
/// backend refuses `Surface::acquire` with `AcquireResult::Occluded`
/// whenever the hosting NSWindow lacks `NSWindowOcclusionStateVisible` (the
/// guard carried over from wgpu-hal's workaround for gfx-rs/wgpu#8309,
/// where a presented drawable on an occluded window wedges `nextDrawable`
/// for a second). Ordering the window first lets Metal acquire a drawable
/// while the frozen desktop is already visible underneath.
///
/// Nothing flashes: the caller has already installed (or filled — see
/// `try_pick_up_screenshot`) the frozen-desktop CALayer, and the render
/// subview stacked above it sits at opacity 0 until `WindowHandle::show`
/// fades it in. AppKit refreshes `occlusionState` from the window server
/// rather than synchronously here, so on its own this is only a head start,
/// not a guarantee: measured warm runs reach the frame-0 acquire ~0.2 ms
/// after this call, long before AppKit has caught up, and the acquire still
/// fails `Occluded`. The worker closes that remaining gap itself —
/// `render.rs::present_first_frame` waits the transition out with a bounded
/// retry. Both halves are needed: without this order-front the occlusion
/// state never flips at all and the retry only burns its budget.
#[cfg(target_os = "macos")]
fn order_window_front_early(window: &Window) {
    use objc2_app_kit::NSView;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Ok(handle) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return };

    unsafe {
        let ns_view: &NSView = &*(h.ns_view.as_ptr() as *const NSView);
        if let Some(ns_window) = ns_view.window() {
            // The Draw on Screen toolbar already sits above the standard overlay level.
            flicksnap_desktop::overlay::raise_above_overlays(&ns_window);
        }
    }
}

/// Whether the scroll-picker's reticle is standing in for the OS pointer
/// at this instant. When it is, the pointer is hidden and the reticle
/// drawn; when it is not, the ordinary pointer is back and no reticle is
/// drawn. The two halves must agree — hiding one without showing the other
/// leaves the user aiming at nothing — so `update_cursor_visibility` and
/// `ui::components::compose` both answer from this one rule.
///
/// The reticle is the pointer's stand-in over the selection's INTERIOR
/// only, because that is the only place a pick can land. Everywhere else
/// the mode keeps the ordinary pointer: the resize handles are still live
/// and their arrows have to be visible, and so does the arrow over the
/// tray.
///
/// The tray is the case that is easy to miss. The scroll-picker is locked
/// to a row ([`crate::ui::components::panel::model::PanelButtonSet::axis_lock`]),
/// so a selection with no room beneath it gets the strip placed INSIDE it
/// (`place::Side::Inside`) — where the hittest is `Inside` like anywhere
/// else in the region, but the pointer is over live buttons and has to be
/// visible.
///
/// Gated on `overlays_visible` for the same agreement reason. Unreachable
/// today — Q is swallowed in pick mode — and this keeps it that way if a
/// path is ever added.
pub fn reticle_stands_in(scroll_pick_mode: bool, overlays_visible: bool, hittest: Hittest, over_tray: bool) -> bool {
    scroll_pick_mode && overlays_visible && hittest == Hittest::Inside && !over_tray
}

/// The pointer's icon for the state as it is, given what the tray found
/// under the pointer in the broadcast that just ran.
///
/// Pick mode claims the selection's interior — the reticle stands in for
/// the pointer there — but only the interior: the tray and the resize
/// handles are live, and they keep their own cursors.
fn cursor_for(input: &InteractionState, over_button: bool, over_tray: bool) -> CursorIcon {
    if reticle_stands_in(
        input.scroll_pick_mode,
        input.overlays_visible,
        input.hittest,
        over_button || over_tray,
    ) {
        // The crosshair is only the fallback for a failed hardware hide;
        // the reticle is the real pointer wherever this branch is taken.
        CursorIcon::Crosshair
    } else if over_button {
        CursorIcon::Pointer
    } else if over_tray {
        // Dead tray (chassis, emblem, readout, padding): its clicks are
        // swallowed on press, so the arrow, never a resize handle that
        // would promise a drag the press cannot start.
        CursorIcon::Default
    } else if input.ocr.active() {
        // The selection is frozen for the whole of OCR mode: resize
        // arrows would promise an interaction it no longer offers.
        CursorIcon::Default
    } else if input.marks.is_drawing() || (input.draw.tool.is_some() && is_canvas(input.hittest)) {
        // With a tool picked the screen is the canvas, not a selection to
        // move: no move arrows, just the ordinary pointer.
        CursorIcon::Default
    } else {
        input.hittest.cursor()
    }
}

/// The system pointer is the overlay's pointer throughout — before the
/// selection as after it — except where the scroll picker's reticle
/// stands in for it.
fn update_cursor_visibility(windows: &WindowSet, cycle: &CaptureCycle) {
    let input = &cycle.input;
    if reticle_stands_in(
        input.scroll_pick_mode,
        input.overlays_visible,
        input.hittest,
        input.pointer_over_tray,
    ) {
        windows.hide_cursors();
    } else {
        windows.show_cursors();
    }
}

fn set_cursor_if_changed(windows: &WindowSet, last_cursor: &mut HashMap<WindowId, CursorIcon>, id: WindowId, cursor: CursorIcon) {
    if last_cursor.get(&id) == Some(&cursor) {
        return;
    }
    if let Some(window) = windows.get(&id) {
        window.set_cursor(cursor);
        last_cursor.insert(id, cursor);
    }
}

/// Where a press with a drawing tool picked starts a mark: anywhere but a
/// resize handle, inside the selection or out. Ink is clipped to the
/// selection on export. The handles keep resizing around it.
fn is_canvas(hittest: Hittest) -> bool {
    matches!(hittest, Hittest::Inside | Hittest::Outside)
}

/// The undo chord's modifiers: Ctrl alone, or Cmd alone on macOS.
fn is_undo_chord(m: ModifiersState) -> bool {
    m == if cfg!(target_os = "macos") {
        ModifiersState::SUPER
    } else {
        ModifiersState::CONTROL
    }
}

/// The letter an overlay hotkey answers to for a key press, lowercase, on
/// any keyboard layout. ASCII letters follow the active layout. Letters
/// from other scripts use the physical QWERTY position.
fn hotkey_letter(logical: &Key, physical: PhysicalKey) -> Option<char> {
    let Key::Character(ch) = logical else {
        return None;
    };
    let mut chars = ch.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_lowercase());
    }
    if !c.is_alphabetic() || c.script() == Script::Latin {
        return None;
    }
    const QWERTY: [(KeyCode, char); 26] = [
        (KeyCode::KeyA, 'a'),
        (KeyCode::KeyB, 'b'),
        (KeyCode::KeyC, 'c'),
        (KeyCode::KeyD, 'd'),
        (KeyCode::KeyE, 'e'),
        (KeyCode::KeyF, 'f'),
        (KeyCode::KeyG, 'g'),
        (KeyCode::KeyH, 'h'),
        (KeyCode::KeyI, 'i'),
        (KeyCode::KeyJ, 'j'),
        (KeyCode::KeyK, 'k'),
        (KeyCode::KeyL, 'l'),
        (KeyCode::KeyM, 'm'),
        (KeyCode::KeyN, 'n'),
        (KeyCode::KeyO, 'o'),
        (KeyCode::KeyP, 'p'),
        (KeyCode::KeyQ, 'q'),
        (KeyCode::KeyR, 'r'),
        (KeyCode::KeyS, 's'),
        (KeyCode::KeyT, 't'),
        (KeyCode::KeyU, 'u'),
        (KeyCode::KeyV, 'v'),
        (KeyCode::KeyW, 'w'),
        (KeyCode::KeyX, 'x'),
        (KeyCode::KeyY, 'y'),
        (KeyCode::KeyZ, 'z'),
    ];
    let PhysicalKey::Code(code) = physical else {
        return None;
    };
    QWERTY
        .iter()
        .find(|(k, _)| *k == code)
        .map(|&(_, c)| c)
}

/// Whether the drawing strip is up to take an UNDO: the capture strip is.
fn drawbar_is_up(input: &InteractionState) -> bool {
    active_panel_set(input.captured, input.scroll_pick_mode, &input.ocr) == Some(panel::model::PanelButtonSet::Normal)
}

/// Whether a press is still being dragged on the captured selection: a
/// mark being drawn, or the selection being moved or resized. Every
/// command waits for its release — a mode switch or an action mid-drag
/// would leave the gesture running against a strip, a mode or a capture
/// that is no longer the one it started on.
fn gesture_in_progress(input: &InteractionState) -> bool {
    input.captured && (input.marks.is_drawing() || input.drag_mode.is_some())
}

/// What one Escape press undoes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscapeStep {
    ClosePalette,
    ExitOcr,
    ExitScrollPick,
    Cancel,
}

/// Escape backs out one step at a time: an open colour popup shuts
/// (draw-on-screen's palette does the same), OCR mode unwinds through its
/// own ladder (Scanning cancels, Lifted starts the retract, Retracting
/// skips it), picking a scroll point returns to the panel with the
/// selection intact, and only outside any of those does Escape cancel the
/// whole cycle — and with it the selection and everything drawn on it.
fn escape_step(input: &InteractionState) -> EscapeStep {
    if input.draw.palette_open {
        EscapeStep::ClosePalette
    } else if input.ocr.active() {
        EscapeStep::ExitOcr
    } else if input.scroll_pick_mode {
        EscapeStep::ExitScrollPick
    } else {
        EscapeStep::Cancel
    }
}

/// The command Return fires — the panel's invisible default button — or
/// `None` when Return must stay inert.
///
/// Return is gated by the same state the panel is:
/// - While a scroll point is being picked the panel is hidden and every
///   accelerator it owns is swallowed, so Return must be inert too. Without
///   the pick-mode gate it would write a plain screenshot session and end
///   the cycle out from under the scroll capture the user is half-way
///   through configuring. Escape remains the only way out of pick mode.
/// - While OCR lines are lifted the default accept is COPY, the most likely
///   reason the user ran OCR at all. Returning `Edit` here would write a
///   plain screenshot session and destroy the OCR result.
/// - Scanning/Retracting are transitions with nothing to accept yet (or
///   any more), so Return waits them out.
fn default_action(input: &InteractionState) -> Option<Command> {
    if !input.captured || input.scroll_pick_mode {
        return None;
    }
    match input.ocr {
        OcrState::Scanning {
            ..
        }
        | OcrState::Retracting {
            ..
        } => None,
        OcrState::Lifted {
            ..
        } => Some(Command::OcrCopy),
        OcrState::Idle => Some(Command::Edit),
    }
}

/// Maximum time between the presses of a double-click. On Windows this
/// follows the user's system setting, with 500 ms as the fallback elsewhere.
fn panel_swap_guard_window() -> Duration {
    #[cfg(windows)]
    {
        // GetDoubleClickTime never fails; the clamp is belt-and-braces so a
        // hostile/broken registry value can neither disable the guard (0)
        // nor wedge the panel shut for whole seconds.
        let ms = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
        Duration::from_millis(u64::from(ms.clamp(100, 2000)))
    }
    #[cfg(not(windows))]
    {
        Duration::from_millis(500)
    }
}

/// Protect a mouse double-click from acting on two different layouts.
#[derive(Default)]
struct PanelSwapGuard {
    shown: Option<panel::model::PanelButtonSet>,
    revision: usize,
    press: Option<(ScreenPointF, Instant, usize)>,
}

impl PanelSwapGuard {
    fn observe(&mut self, set: Option<panel::model::PanelButtonSet>) {
        if self.shown != set {
            self.shown = set;
            self.revision += 1;
        }
    }

    fn press(&mut self, point: ScreenPointF, now: Instant, window: Duration, half_width: f32, half_height: f32) -> bool {
        if self
            .press
            .is_some_and(|(previous, at, revision)| {
                revision != self.revision
                    && now.saturating_duration_since(at) < window
                    && (point.x - previous.x).abs() <= half_width
                    && (point.y - previous.y).abs() <= half_height
            })
        {
            return true;
        }
        self.press = Some((point, now, self.revision));
        false
    }
}

fn double_click_half_size(dpi: f32) -> (f32, f32) {
    #[cfg(windows)]
    {
        use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
        use windows::Win32::UI::WindowsAndMessaging::{SM_CXDOUBLECLK, SM_CYDOUBLECLK};
        let dpi = (dpi * 96.0).round() as u32;
        unsafe {
            (
                GetSystemMetricsForDpi(SM_CXDOUBLECLK, dpi).max(2) as f32 / 2.0,
                GetSystemMetricsForDpi(SM_CYDOUBLECLK, dpi).max(2) as f32 / 2.0,
            )
        }
    }
    #[cfg(not(windows))]
    {
        (4.0 * dpi, 4.0 * dpi)
    }
}

/// Recompute the shared UI state, run the egui hosts that need it and
/// publish the result to every worker.
///
/// While the debug panels are up this runs on every `about_to_wait`: the
/// workers publish a perf snapshot per frame and the panel asks for a tick
/// at the host's repaint floor, so the app thread broadcasts at roughly
/// 60 Hz (one `Arc<UiSharedState>` per tick) for
/// as long as `D` is on. That is the steady state during that window only;
/// with the panels down nothing schedules a tick and the thread goes quiet
/// between events. Follow-up if it ever costs too much: when the primary
/// panel's `cpu` row shows the app thread above 2 ms per tick, memoise
/// `MonitorPanelData::write_lines` on `seq` and `PrimaryPanelData::write_lines`
/// on its input tuple.
fn broadcast_ui_state(
    windows: &WindowSet,
    monitors: &[MonitorInfo],
    ui_monitors: &Arc<[UiMonitor]>,
    cycle: &mut CaptureCycle,
) -> PanelOutcome {
    // Feed the double-click swap guard from the same decision function the
    // click routing and the renderers use (`active_panel_set`) — deriving
    // it any other way could let the guard and the panel disagree about
    // when a swap happened. Every renderer-visible state mutation flows
    // through this broadcast, so the guard is armed before any click can
    // be routed against the new set.
    cycle.panel_swap.observe(active_panel_set(
        cycle.input.captured,
        cycle.input.scroll_pick_mode,
        &cycle.input.ocr,
    ));
    let cursor_pt = to_screen_point(cycle.input.virtual_cursor);

    let hovered_monitor_name = monitors
        .iter()
        .find(|m| m.bounds.contains(cursor_pt))
        .map(|m| m.name.as_str());

    // This is the one call that
    // runs the hosts with everything this broadcast knows, so its outcome
    // is also the answer to what the pointer is over — the cursor ladder
    // in `CursorMoved` reads it rather than running the tray a second time.
    let panel = cycle.egui.sync(SyncArgs {
        input: &cycle.input,
        settings: &cycle.settings,
        monitors,
        ui_monitors,
        desktop_buffer: cycle.desktop_buffer.as_deref(),
        hovered_monitor_name,
    });
    // What the tray found under the pointer in the pass that just ran, so
    // every decision taken after this broadcast — the cursor ladder below,
    // `update_cursor_visibility` at its ten call sites — answers from this
    // layout rather than a stale or re-derived one.
    cycle.input.pointer_over_tray = panel.over_tray;
    let egui_frames = cycle.egui.frames();

    // What a worker still decides for itself, and the egui primitives it
    // paints. Built here rather than in a helper: the struct is now small
    // enough that a builder only hid which tick each field came from.
    let state = Arc::new(UiSharedState {
        debug_visible: cycle.input.debug_visible,
        overlays_visible: cycle.input.overlays_visible,
        cursor_overlay_visible: cycle.input.cursor_overlay_visible,
        scroll_pick_mode: cycle.input.scroll_pick_mode,
        // The cycle keeps its copy, so this one is a genuine clone — but
        // the only heap content is the outcome behind an Arc, so the cost
        // is one atomic increment (and one decrement when the previous
        // broadcast's Arc is finally dropped), even on the per-mouse-move
        // path. Everything else in the enum is Copy.
        ocr: cycle.input.ocr.clone(),
        egui: egui_frames,
    });

    for h in windows.values() {
        h.update_ui_state(state.clone());
    }

    panel
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        settings: Arc<CapturerSettings>,
        startup: Arc<StartupTimings>,
        instance: crate::gxi::Instance,
        monitors: Vec<MonitorInfo>,
        initial_mouse: ScreenPointF,
        worker_setups: Vec<WorkerSetup>,
        screenshot_latch: Arc<Latch<Arc<CapturedDesktop>>>,
        active_window: Option<WindowTarget>,
        ready_count: Arc<AtomicUsize>,
        visible_latch: Arc<VisibleLatch>,
        worker_failed: Arc<AtomicUsize>,
    ) -> Self {
        let vd_bounds = {
            let mut min_x = i32::MAX;
            let mut min_y = i32::MAX;
            let mut max_x = i32::MIN;
            let mut max_y = i32::MIN;
            for m in &monitors {
                min_x = min_x.min(m.bounds.min_x());
                min_y = min_y.min(m.bounds.min_y());
                max_x = max_x.max(m.bounds.max_x());
                max_y = max_y.max(m.bounds.max_y());
            }
            ScreenRect::from_exact(min_x, min_y, max_x, max_y)
        };

        let ui_monitors: Arc<[UiMonitor]> = monitors
            .iter()
            .map(|m| UiMonitor {
                bounds: m.bounds,
                dpi_scale: m.scale_factor,
                is_primary: m.is_primary,
            })
            .collect();

        let expected = worker_setups.len();

        let primary = monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| monitors.first())
            .expect("at least one monitor present");
        let cursor_overlay_visible = settings.cursor_visible_at_startup;
        let monitor_target = || {
            let point = to_screen_point(initial_mouse);
            let monitor = monitors
                .iter()
                .find(|m| m.bounds.contains(point))
                .unwrap_or(primary);
            WindowTarget {
                rect: monitor.bounds,
                corner_radius: 0.0,
            }
        };
        let pending_preselect = match settings.capture_mode {
            CaptureMode::Region => None,
            CaptureMode::Screen => Some(monitor_target()),
            CaptureMode::Window => Some(active_window.unwrap_or_else(monitor_target)),
        };

        let cycle = CaptureCycle {
            settings,
            desktop_buffer: None,
            screenshot_latch,
            // Anchored here rather than at the first pickup poll so a stall
            // anywhere between construction and the event loop counts
            // against the same budget the old blocking wait enforced.
            screenshot_deadline: Instant::now() + Duration::from_secs(30),
            last_cursor: HashMap::new(),
            pending_show: Some(PendingShow {
                ready_count,
                expected,
                visible_latch,
                since: Instant::now(),
            }),
            pending_preselect,
            video_dispatched: false,
            share_dispatched: false,
            ocr_req: 0,
            ocr_job: None,
            ocr_ready: None,
            panel_swap: PanelSwapGuard::default(),
            egui: EguiHosts::new(&ui_monitors, startup.clone()),
            input: InteractionState {
                virtual_cursor: initial_mouse,
                mouse_down: false,
                mouse_down_pt: None,
                mouse_down_dpi: 1.0,
                dragging: false,
                selection: None,
                selection_radius: 0.0,
                captured: false,
                hittest: Hittest::Outside,
                drag_mode: None,
                drag_anchor_selection: None,
                tips_visible: false,
                loupe_visible: false,
                debug_visible: false,
                overlays_visible: true,
                cursor_overlay_visible,
                pointer_over_tray: false,
                scroll_pick_mode: false,
                ocr: OcrState::Idle,
                ocr_notice: None,
                draw: DrawState::default(),
                marks: Marks::default(),
                modifiers: ModifiersState::empty(),
            },
        };

        Self {
            windows: WindowSet::new(),
            monitors,
            ui_monitors,
            vd_bounds,
            startup,
            instance,
            worker_setups: Some(worker_setups),
            worker_failed,
            cycle: Some(cycle),
            fatal: None,
            finished: None,
        }
    }

    /// Hand back a failure detected inside the event loop (see the `fatal`
    /// field). Called by `main` after `run_app` returns.
    pub fn fatal_result(&mut self) -> anyhow::Result<()> {
        match self.fatal.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// How the cycle ended. A loop that exited without finishing a cycle
    /// counts as canceled: nothing was handed to the shell.
    pub fn finished(&self) -> CycleAction {
        self.finished
            .unwrap_or(CycleAction::Canceled)
    }

    /// Window creation failed, so the show gate can cancel startup as
    /// soon as it checks the worker status.
    fn record_window_failure(&self) {
        self.worker_failed
            .fetch_add(1, Ordering::Release);
    }

    /// Tear down the current capture cycle: hide every window, restore the
    /// hardware cursor, drop the per-cycle state and exit this event-loop run.
    /// `main` then drops `App` itself and returns to minimal standby.
    fn finish_cycle(&mut self, event_loop: &ActiveEventLoop, action: CycleAction) {
        log::info!("capture cycle finished: {:?}", action);
        self.finished.get_or_insert(action);
        // Every exit path that has a live cycle comes through here, so this is
        // also the last hide before the process itself goes away. Hence the
        // foreground handback rather than a bare hide, even though the action
        // dispatches have usually done it already.
        hide_overlay_for_action(&self.windows);
        // Idempotent (guarded by a static in window.rs) even when cursors
        // were already restored via update_cursor_visibility.
        set_hardware_cursor_visible(true);
        if let Some(cycle) = self.cycle.take() {
            // A recognition may still be running; flag it canceled so the
            // worker skips setting a latch nobody will read. NEVER joined:
            // this path also serves ParentGone, and a cold recognize can
            // take hundreds of ms — blocking here would strand a visible
            // overlay behind a dead cycle.
            if let Some((_, _, cancel)) = &cycle.ocr_job {
                cancel.store(true, Ordering::Release);
            }
            if let Some(pending) = &cycle.pending_show {
                pending.visible_latch.signal_all();
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
        event_loop.exit();
    }

    fn show_all_windows(&self) {
        self.windows.show_all();
        if let Some(cycle) = self.cycle.as_ref() {
            update_cursor_visibility(&self.windows, cycle);
        }
    }

    /// Non-blocking pickup of the desktop screenshot, polled at the top of
    /// `resumed` (so a capture that beats the event loop still takes the
    /// install-at-creation path) and on every `about_to_wait` pass. The main
    /// thread used to block on this latch in `CaptureSession::new` — ~50 ms,
    /// the single largest delta on the startup critical path — serializing
    /// the event loop and every window/surface creation behind the capture.
    ///
    /// On arrival this is also where the macOS windows first go on screen:
    /// `resumed` skips the early order-front while the buffer is pending
    /// (an ordered window with an empty backdrop layer is an opaque black
    /// rectangle), so the backdrop fill and the order-front happen together
    /// here, and the show-gate clock is re-anchored to that moment — the
    /// frozen still it guards against stranding cannot be on screen earlier.
    ///
    /// Past the deadline (the old blocking wait's 30 s bound) the cycle is
    /// torn down and the error parked for `fatal_result`: a wedged CG
    /// capture call must end as a clean failing exit, not a process idling
    /// forever with hidden windows and no way to close it from the shell.
    fn try_pick_up_screenshot(&mut self, event_loop: &ActiveEventLoop) {
        {
            let Some(cycle) = self.cycle.as_mut() else {
                return;
            };
            if cycle.desktop_buffer.is_some() {
                return;
            }
            if let Some(buf) = cycle.screenshot_latch.try_get() {
                self.startup.mark_screenshot_latch_released();
                #[cfg(target_os = "macos")]
                for h in self.windows.values() {
                    h.install_backdrop(&buf);
                    order_window_front_early(h.winit_window());
                }
                if let Some(pending) = cycle.pending_show.as_mut() {
                    pending.since = Instant::now();
                }
                cycle.desktop_buffer = Some(buf);
                return;
            }
            if Instant::now() < cycle.screenshot_deadline {
                return;
            }
        }
        error!("timed out waiting for the desktop screenshot; tearing the cycle down");
        self.fatal = Some(anyhow!("timed out waiting for the desktop screenshot"));
        self.finish_cycle(event_loop, CycleAction::Canceled);
    }

    /// Non-blocking pickup of the OCR worker's result, plus the OCR phase
    /// clock. Called from `about_to_wait`; an active job or transition
    /// keeps a bounded wakeup scheduled even without pointer input.
    fn try_advance_ocr(&mut self) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        // Result pickup. A finished job is always consumed, but whether the
        // result is USED depends on its id still matching the current
        // Scanning phase: BACK bumps `ocr_req` while the same cycle stays
        // alive.
        if let Some((job_req, latch, _)) = cycle.ocr_job.as_ref() {
            if let Some(result) = latch.try_get() {
                let job_req = *job_req;
                cycle.ocr_job = None;
                if let OcrState::Scanning {
                    req,
                    anchor,
                    ..
                } = &cycle.input.ocr
                {
                    if *req == job_req {
                        // Held (not applied) until its release time. A
                        // successful outcome waits for the sweep's current
                        // pass to wrap so the Lifted reveal pass picks up
                        // with the band off-screen (no mid-region
                        // teleport); failures wait only the anti-flash
                        // floor — nothing is going to be revealed, so
                        // dragging the error out a full pass would be
                        // pure latency.
                        // Pickup is where the fallback faces go in, while
                        // the sweep is still running: `set_fonts` rebuilds
                        // every host's atlas, and that is a hitch the sweep
                        // can absorb but the reveal tick cannot. An ASCII
                        // page never pays for it.
                        if let Ok(o) = &result {
                            if !o.lines.is_empty() && crate::ui::fonts::needs_fallback(o.lines.iter().map(|l| l.text.as_str())) {
                                if let FontInstall::ScanPending = cycle.egui.install_fallback_fonts() {
                                    log::warn!("system font scan not finished; OCR bubbles may show '?' for non-Latin glyphs");
                                }
                            }
                        }
                        let align = matches!(&result, Ok(o) if !o.lines.is_empty());
                        let release = ocr::anim::scan_release_secs(anchor.elapsed().as_secs_f32(), align);
                        cycle.ocr_ready = Some((release, result));
                    } else {
                        log::info!("discarding OCR result for superseded request {job_req}");
                    }
                } else {
                    log::info!("discarding OCR result for superseded request {job_req}");
                }
            }
        }
        match &cycle.input.ocr {
            OcrState::Scanning {
                anchor,
                req,
                region,
            } => {
                let (anchor, req, region) = (*anchor, *req, *region);
                let elapsed = anchor.elapsed().as_secs_f32();
                // The second clause turns "the rebuild almost certainly
                // landed during the sweep" into a guarantee: every host that
                // was handed new fonts must have run the pass that applies
                // them before the reveal starts.
                if cycle
                    .ocr_ready
                    .as_ref()
                    .is_none_or(|(release, _)| elapsed < *release)
                    || !cycle.egui.fonts_settled()
                {
                    return;
                }
                let Some((_, result)) = cycle.ocr_ready.take() else {
                    return;
                };
                match result {
                    Ok(outcome) if !outcome.lines.is_empty() => {
                        // ONE dpi_scale for all lift geometry — the monitor
                        // containing the region's center — so a line
                        // crossing a mixed-DPI seam moves by the same
                        // physical amount on both halves instead of tearing.
                        let cx = region.center_x();
                        let cy = region.center_y();
                        let dpi_scale = self
                            .monitors
                            .iter()
                            .find(|m| {
                                let b = m.bounds;
                                cx >= b.left() && cx < b.right() && cy >= b.top() && cy < b.bottom()
                            })
                            .map(|m| m.scale_factor)
                            .unwrap_or(1.0);
                        // text_angle is deliberately logged and nowhere else
                        // consumed: the lift draws axis-aligned quads, so a
                        // skewed page still lifts straight. The number is here
                        // so a "the lift looks wrong on this page" report can
                        // be diagnosed from a log instead of a repro.
                        log::info!(
                            "OCR recognized {} lines (text angle {:.2} deg)",
                            outcome.lines.len(),
                            outcome.text_angle
                        );
                        // Fresh anchor: the reveal pass's t=0 is NOW — and
                        // NOW is (poll latency aside) the instant the
                        // scanning sweep wrapped, because the release
                        // above was pass-aligned. The band re-enters from
                        // above the region top exactly as the old pass
                        // exited below its bottom.
                        cycle.input.ocr = OcrState::Lifted {
                            anchor: Instant::now(),
                            // The Scanning phase's request id, carried over:
                            // the render side keys its shaped-bubble cache
                            // on it (see OcrState::Lifted).
                            req,
                            region,
                            dpi_scale,
                            outcome: Arc::new(outcome),
                        };
                        // Entering the mode clears any stale failure pill —
                        // it would sit there contradicting the lifted lines.
                        cycle.input.ocr_notice = None;
                    }
                    // All three failure shapes land back in the plain
                    // captured state — never in OCR mode — with a transient
                    // notice pill instead of a dialog: the overlay is
                    // topmost and fullscreen, so a dialog would open BEHIND
                    // it (every existing dialog call site hides the overlay
                    // first), and a silent return would be indistinguishable
                    // from a broken button.
                    Ok(_) => {
                        log::info!("OCR found no text");
                        cycle.input.ocr = OcrState::Idle;
                        cycle.input.ocr_notice = Some(OcrNotice {
                            anchor: Instant::now(),
                            kind: OcrNoticeKind::NoText,
                        });
                    }
                    Err(OcrError::Unavailable) => {
                        // The recognizer could not be spawned (flicksnap_ai does
                        // not ship on Intel macOS), or its ONNX Runtime failed
                        // to init — either cause is already logged at error
                        // level (ocr::client / flicksnap_ai).
                        log::warn!("OCR is unavailable on this machine");
                        cycle.input.ocr = OcrState::Idle;
                        cycle.input.ocr_notice = Some(OcrNotice {
                            anchor: Instant::now(),
                            kind: OcrNoticeKind::Unavailable,
                        });
                    }
                    Err(OcrError::Failed(msg)) => {
                        // The detail goes to the log only — a WinRT HRESULT
                        // string is noise on screen.
                        log::warn!("OCR failed: {msg}");
                        cycle.input.ocr = OcrState::Idle;
                        cycle.input.ocr_notice = Some(OcrNotice {
                            anchor: Instant::now(),
                            kind: OcrNoticeKind::Failed,
                        });
                    }
                }
                broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
            }
            OcrState::Retracting {
                anchor,
                ..
            } => {
                if anchor.elapsed().as_secs_f32() >= ocr::anim::RETRACT_DURATION_SECS {
                    cycle.input.ocr = OcrState::Idle;
                    broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                }
            }
            OcrState::Idle
            | OcrState::Lifted {
                ..
            } => {}
        }
    }

    /// Leave OCR mode one step: Scanning cancels the job outright, Lifted
    /// drops every bubble/crop AT ONCE and starts the region's color
    /// fade, Retracting (Escape pressed twice) skips straight to Idle.
    /// Serves both the panel's BACK button and Escape.
    ///
    /// Deliberately narrow, modelled on the scroll-pick Escape arm: it must
    /// NOT touch `InteractionController::reset` (which would destroy the
    /// selection BACK exists to preserve) and must NOT hide the overlay —
    /// the user lands back on the captured panel, not on the desktop.
    /// Leave scroll-point pick mode with the selection intact: what
    /// Escape and the strip's BACK button both do. The reticle, the
    /// hidden pointer and the pick strip all hang off the one flag, so
    /// dropping it and re-settling the cursor is the whole of it.
    fn exit_scroll_pick_mode(&mut self, window_id: WindowId) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        if !cycle.input.scroll_pick_mode {
            return;
        }
        cycle.input.scroll_pick_mode = false;
        self.broadcast_and_settle_pointer(window_id);
    }

    fn exit_ocr_mode(&mut self, window_id: WindowId) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        match std::mem::replace(&mut cycle.input.ocr, OcrState::Idle) {
            OcrState::Idle => return,
            OcrState::Scanning {
                ..
            } => {
                // Cooperative cancel: the worker checks the flag around its
                // blocking recognize call and the latch is simply dropped.
                // Never joined — this runs on the winit thread and a cold
                // recognize can take hundreds of ms.
                if let Some((_, _, cancel)) = cycle.ocr_job.take() {
                    cancel.store(true, Ordering::Release);
                }
                cycle.ocr_ready = None;
            }
            OcrState::Lifted {
                ..
            } => {
                // Fresh anchor: the exit fade starts NOW. The outcome is
                // deliberately dropped here — the text vanishes on this
                // very frame (no reverse animation, by design) and only
                // the region's fade back to color remains to play.
                cycle.input.ocr = OcrState::Retracting {
                    anchor: Instant::now(),
                };
            }
            // Second Escape mid-retract: the user wants out, not a replay —
            // the replace above already snapped to Idle.
            OcrState::Retracting {
                ..
            } => {}
        }
        // Bump the request id on EVERY exit path so a result still in
        // flight is discarded at pickup — within a live cycle this counter
        // is the only stale-result guard there is.
        cycle.ocr_req += 1;
        // The mode forced a Default cursor over the frozen selection;
        // whatever the pointer is on now decides what replaces it.
        self.broadcast_and_settle_pointer(window_id);
    }

    /// Capture a plain rect — a monitor, the whole desktop: square corners.
    fn finalize_selection(&mut self, rect: ScreenRect, event_loop: &ActiveEventLoop, window_id: WindowId) {
        self.finalize_selection_inner(rect, 0.0, event_loop, window_id);
    }

    fn finalize_selection_inner(&mut self, rect: ScreenRect, corner_radius: f32, event_loop: &ActiveEventLoop, window_id: WindowId) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        let effects = InteractionController::finalize_selection(&mut cycle.input, rect, corner_radius, &self.monitors);
        self.apply_interaction_effects(effects, Some(window_id));

        // Keyboard / preselect captured-transition site (DESIGN §3.3).
        self.on_captured(event_loop, window_id);
    }

    /// Auto-dispatch `Command::Video` (`--video`) or `Command::Share`
    /// (`--share`) the first time a selection becomes captured. Called from
    /// both captured-transition sites (mouse-release drag-select and the
    /// keyboard/preselect `finalize_selection` path) so the shell-driven
    /// modes work for every entry path (DESIGN §3.3).
    ///
    /// The command is decided and its guard set before anything is
    /// dispatched, because `dispatch_command` wants `&mut self` and the
    /// guards live behind the `cycle` borrow. The two modes are mutually
    /// exclusive on the command line (`--share` conflicts with `--video`),
    /// so the ordering below is a formality rather than a precedence rule.
    fn on_captured(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        let auto = if cycle.settings.video_mode && !cycle.video_dispatched {
            cycle.video_dispatched = true;
            Some(Command::Video)
        } else if cycle.settings.share_mode && !cycle.share_dispatched {
            cycle.share_dispatched = true;
            Some(Command::Share)
        } else {
            None
        };
        if let Some(command) = auto {
            self.dispatch_command(command, event_loop, window_id);
        }
    }

    fn try_preselect(&mut self, event_loop: &ActiveEventLoop) {
        let Some(cycle) = self.cycle.as_ref() else {
            return;
        };
        let Some(target) = cycle.pending_preselect else {
            return;
        };
        // Wait until the overlay is up (panel needs a shown window to render).
        if cycle.pending_show.is_some() {
            return;
        }
        let Some(window_id) = self.windows.first().map(|h| h.window_id()) else {
            return;
        };

        log::info!("pre-selecting capture rectangle {:?}", target.rect);
        self.finalize_selection_inner(target.rect, target.corner_radius, event_loop, window_id);
        // finalize_selection may have ended the cycle (--video auto-dispatch).
        if let Some(cycle) = self.cycle.as_mut() {
            cycle.pending_preselect = None;
        }
    }

    fn handle_reset(&mut self, window_id: WindowId) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        let effects = InteractionController::reset(&mut cycle.input);
        self.apply_interaction_effects(effects, Some(window_id));

        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        broadcast_mouse_state(&self.windows, &cycle.input);

        log::info!("selection reset");
    }

    /// Push the state to the renderers and settle the pointer from the
    /// very pass that push ran: its icon, and whether the OS pointer is
    /// shown at all.
    ///
    /// The cursor ladder otherwise only runs on `CursorMoved`, so a
    /// command that changes the mode without moving the mouse — OCR,
    /// SCROLL, and the two ways back out of them — left the pointer
    /// wearing whatever it had when the button went down (the hand over a
    /// button, or the picker's crosshair) until the user happened to move.
    ///
    /// Both halves have to come from the same broadcast. The mode change
    /// re-places the strip under the cursor and may swap the set
    /// altogether, so what the pointer is over afterwards is not something
    /// the caller can know before laying the new strip out — and
    /// `update_cursor_visibility` reads `pointer_over_tray`, which this
    /// broadcast is what sets.
    fn broadcast_and_settle_pointer(&mut self, window_id: WindowId) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        let panel = broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
        let cursor = cursor_for(&cycle.input, panel.over_button, panel.over_tray);
        set_cursor_if_changed(&self.windows, &mut cycle.last_cursor, window_id, cursor);
        update_cursor_visibility(&self.windows, cycle);
    }

    fn apply_interaction_effects(&mut self, effects: InteractionEffects, window_id: Option<WindowId>) {
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        if effects.update_cursor_visibility {
            update_cursor_visibility(&self.windows, cycle);
        }
        if let (Some(window_id), Some(cursor)) = (window_id, effects.set_cursor) {
            set_cursor_if_changed(&self.windows, &mut cycle.last_cursor, window_id, cursor);
        }
        if effects.broadcast_ui {
            broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
        }
        if effects.broadcast_mouse {
            broadcast_mouse_state(&self.windows, &cycle.input);
        }
    }

    /// Complete a scroll-point pick — the click that finishes what
    /// [`Command::ScrollCapture`] armed.
    ///
    /// Same shape as the Video dispatch arm (hide the overlay, write the
    /// action payload, finish the cycle), with the picked point and the
    /// window under it added to the marker. A click outside the selection
    /// is ignored and leaves pick mode armed: the driver may only aim the
    /// wheel inside the region it is going to stitch, and re-clicking is a
    /// friendlier correction than dropping the user back to the panel.
    fn dispatch_scroll_pick(&mut self, event_loop: &ActiveEventLoop) {
        use xdialog::XDialogIcon::Error as ErrorIcon;

        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        let Some(session_dir) = cycle.settings.session_dir.clone() else {
            log::info!("scroll pick ignored: no --session-dir provided");
            return;
        };
        let Some(selection) = cycle.input.selection else {
            log::info!("scroll pick ignored: no selection");
            return;
        };
        let point = to_screen_point(cycle.input.virtual_cursor);
        if !selection.contains(point) {
            log::info!("scroll pick ignored: click outside the selection");
            return;
        }
        // The foreground handback in here matters most on exactly this
        // action: the shell passes those rights straight to the scrolling
        // capture driver, which needs them to raise the window the user
        // picked over whatever is covering it.
        hide_overlay_for_action(&self.windows);
        match write_scroll_action(&session_dir, selection, point, 0, &self.monitors) {
            ActionResult::Success => self.finish_cycle(event_loop, CycleAction::Scroll),
            ActionResult::Canceled => self.show_all_windows(),
            ActionResult::Failed(msg) => {
                // Retry re-shows the overlay still in pick mode, so the
                // user lands back on the crosshair, not on the panel.
                if xdialog::show_message_retry_cancel("flickSnap Capture", "Scrolling Capture Failed", &msg, ErrorIcon).unwrap_or(false) {
                    self.show_all_windows();
                } else {
                    self.finish_cycle(event_loop, CycleAction::Canceled);
                }
            }
        }
    }

    /// Shut the drawing strip's colour popup, and show that it is shut.
    fn close_palette(&mut self) {
        if let Some(cycle) = self.cycle.as_mut() {
            if cycle.input.draw.close_palette() {
                broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
            }
        }
    }

    fn dispatch_command(&mut self, command: Command, event_loop: &ActiveEventLoop, window_id: WindowId) {
        use xdialog::XDialogIcon::Error as ErrorIcon;
        if self
            .cycle
            .as_ref()
            .is_some_and(|c| gesture_in_progress(&c.input))
        {
            log::info!("command {command:?} ignored: a drag is in progress");
            return;
        }
        log::info!("dispatch command: {:?}", command);

        // Every command but the colour popup's own shuts it, keyboard
        // accelerators included — the popup is a choice about to be made,
        // and doing anything else is the answer to it. Shown at once: some
        // of these hide the overlay or raise a dialog without broadcasting
        // again, and the popup must not be what the overlay comes back to.
        if !matches!(command, Command::DrawPalette | Command::DrawColor(_)) {
            self.close_palette();
        }

        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };
        let cursor_visible = cycle.input.cursor_overlay_visible;
        let screenshot = cycle
            .input
            .selection
            .zip(cycle.desktop_buffer.as_deref())
            .map(|(selection, buffer)| Screenshot {
                selection,
                buffer,
                corner_radius: cycle.input.selection_radius,
                cursor_visible,
                marks: cycle.input.marks.done(),
            });

        match command {
            Command::Copy => {
                hide_overlay_for_action(&self.windows);
                let result = match screenshot {
                    Some(screenshot) => copy_to_clipboard(screenshot),
                    None => ActionResult::Failed("No selection or buffer".into()),
                };
                match result {
                    ActionResult::Success => self.finish_cycle(event_loop, CycleAction::Copy),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Copy to Clipboard Failed", &msg, ErrorIcon)
                            .unwrap_or(false)
                        {
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
            Command::Save => {
                hide_overlay_for_action(&self.windows);
                let result = match screenshot {
                    Some(screenshot) => match self.windows.get(&window_id) {
                        Some(handle) => handle.save_to_file(screenshot, &cycle.settings),
                        None => ActionResult::Failed("No active window".into()),
                    },
                    None => ActionResult::Failed("No selection or buffer".into()),
                };
                match result {
                    ActionResult::Success => self.finish_cycle(event_loop, CycleAction::Save),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Save Failed", &msg, ErrorIcon).unwrap_or(false) {
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
            Command::Edit | Command::Upload => {
                // EDIT and UPLOAD write the same session payload for the
                // shell (CAPTURE_PROTOCOL.md); UPLOAD adds the action.txt
                // marker so the shell uploads instead of opening the
                // editor. Without a --session-dir there is no shell
                // listening — ignore.
                let Some(session_dir) = cycle.settings.session_dir.clone() else {
                    log::info!("command {:?} ignored: no --session-dir provided", command);
                    return;
                };
                let (action, cycle_action) = if command == Command::Upload {
                    (SessionAction::Upload, CycleAction::Upload)
                } else {
                    (SessionAction::Edit, CycleAction::Edit)
                };
                hide_overlay_for_action(&self.windows);
                let result = match screenshot {
                    Some(screenshot) => write_session(&session_dir, screenshot, cycle.settings.capture_mode == CaptureMode::Window, action),
                    None => ActionResult::Failed("No selection or buffer".into()),
                };
                match result {
                    ActionResult::Success => self.finish_cycle(event_loop, cycle_action),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Session Capture Failed", &msg, ErrorIcon)
                            .unwrap_or(false)
                        {
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
            Command::SearchImage => {
                // The image has to leave the machine for a reverse image
                // search, and the shell is the half that speaks HTTP — so
                // this writes the cropped image plus a `search-image`
                // marker and lets `CaptureSessionDispatcher` do the
                // lookup. Without a --session-dir there is no shell
                // listening: the Command::Video precedent, ignore.
                let Some(session_dir) = cycle.settings.session_dir.clone() else {
                    log::info!("command SearchImage ignored: no --session-dir provided");
                    return;
                };
                hide_overlay_for_action(&self.windows);
                let result = match screenshot {
                    Some(screenshot) => write_image_search_action(&session_dir, screenshot),
                    None => ActionResult::Failed("No selection or buffer".into()),
                };
                match result {
                    ActionResult::Success => self.finish_cycle(event_loop, CycleAction::SearchImage),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Image Search Failed", &msg, ErrorIcon).unwrap_or(false)
                        {
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
            Command::DrawTool(tool) => {
                cycle.input.draw.toggle_tool(tool);
                self.broadcast_and_settle_pointer(window_id);
            }
            Command::DrawPalette => {
                cycle.input.draw.toggle_palette();
                self.broadcast_and_settle_pointer(window_id);
            }
            Command::DrawColor(index) => {
                cycle.input.draw.pick_color(index);
                self.broadcast_and_settle_pointer(window_id);
            }
            Command::DrawUndo => {
                if cycle.input.marks.undo() {
                    self.broadcast_and_settle_pointer(window_id);
                }
            }
            Command::Reset => self.handle_reset(window_id),
            Command::Exit => {
                self.finish_cycle(event_loop, CycleAction::Canceled);
            }
            Command::Video => {
                // Mirrors Edit|Upload: writes the video action payload
                // (poster + `action.txt`) for the shell to start recording.
                // Without a --session-dir there is no shell listening —
                // ignore. (DESIGN §3.2/§3.3.)
                let Some(session_dir) = cycle.settings.session_dir.clone() else {
                    log::info!("command Video ignored: no --session-dir provided");
                    return;
                };
                hide_overlay_for_action(&self.windows);
                let result = match (cycle.input.selection, cycle.desktop_buffer.as_deref()) {
                    (Some(sel), Some(buf)) => {
                        write_video_action(&session_dir, sel, cycle.input.selection_radius, buf, cursor_visible, &self.monitors)
                    }
                    _ => ActionResult::Failed("No selection or buffer".into()),
                };
                match result {
                    ActionResult::Success => self.finish_cycle(event_loop, CycleAction::Video),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Video Capture Failed", &msg, ErrorIcon).unwrap_or(false)
                        {
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
            Command::Share => {
                // The twin of Command::Video above, minus the pixels. A
                // share marker carries the selection and nothing else, so
                // there is no desktop-buffer requirement here: what the
                // meeting sees is mirrored live off the screen by
                // `flicksnap_share_region`, long after this overlay is gone.
                // Without a --session-dir there is no shell listening —
                // ignore, exactly as Video does.
                let Some(session_dir) = cycle.settings.session_dir.clone() else {
                    log::info!("command Share ignored: no --session-dir provided");
                    return;
                };
                hide_overlay_for_action(&self.windows);
                let result = match cycle.input.selection {
                    Some(sel) => write_share_action(&session_dir, sel, &self.monitors),
                    None => ActionResult::Failed("No selection".into()),
                };
                match result {
                    ActionResult::Success => self.finish_cycle(event_loop, CycleAction::Share),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Share Region Failed", &msg, ErrorIcon).unwrap_or(false)
                        {
                            // Unlatch before re-showing, which VIDEO does not have to do: its
                            // retry can go back through the panel's VIDEO button, while SHARE is
                            // auto-dispatch only (no panel button, by design). Leaving the
                            // one-shot guard set would make Retry a dead end — the overlay comes
                            // back and no re-selection can ever dispatch Share again.
                            if let Some(c) = self.cycle.as_mut() {
                                c.share_dispatched = false;
                            }
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
            Command::ScrollCapture => {
                // SCROLL needs one more input than every other panel
                // command — the point the driver parks the cursor at — so
                // it does not write its payload here. It arms pick mode
                // and leaves the overlay up; the click handler below is
                // what writes `action.txt` and ends the cycle.
                //
                // Like Video, without a --session-dir there is no shell to
                // hand the session to, and without a captured selection
                // there is no region to scroll inside — ignore either way.
                if cycle.settings.session_dir.is_none() {
                    log::info!("command ScrollCapture ignored: no --session-dir provided");
                    return;
                }
                if !cycle.input.captured || cycle.input.selection.is_none() {
                    log::info!("command ScrollCapture ignored: no captured selection");
                    return;
                }
                // One mode at a time — the symmetric twin of the
                // Command::Ocr guard below. The only live path here is the
                // Normal strip's SCROLL button during Retracting (BACK
                // hands the Normal strip back while the exit fade still
                // owns the frozen selection); honouring it would arm
                // scroll-pick mid-fade. It also backstops the double-click
                // hazard: the strip recenters on the swap, so a stray
                // second press can land on ANY Normal button, and while
                // the swap guard covers the double-click window, the
                // 0.18 s fade can outlive it on slow double-click
                // settings.
                if cycle.input.ocr.active() {
                    log::info!("command ScrollCapture ignored: OCR mode is active");
                    return;
                }
                cycle.input.scroll_pick_mode = true;
                // Not unconditionally the crosshair: the picker's strip is
                // laid out by this call and may land under the pointer that
                // just pressed SCROLL, and there the ordinary pointer stays.
                self.broadcast_and_settle_pointer(window_id);
            }
            Command::Ocr => {
                // OCR reads a frozen region; without a captured selection
                // there is nothing to recognize. No session_dir requirement:
                // COPY and SEARCH work standalone, and UPLOAD checks at its
                // own dispatch (the Command::Video precedent).
                if !cycle.input.captured || cycle.input.selection.is_none() {
                    log::info!("command Ocr ignored: no captured selection");
                    return;
                }
                // One recognition at a time. The only live-mode path here is
                // the Normal strip's OCR button during Retracting; honouring
                // it would tear the retract animation out from under the
                // render workers mid-flight.
                if cycle.input.ocr.active() {
                    log::info!("command Ocr ignored: OCR mode already active");
                    return;
                }
                let (Some(sel), Some(buf)) = (cycle.input.selection, cycle.desktop_buffer.as_deref()) else {
                    log::info!("command Ocr ignored: no selection or buffer");
                    return;
                };
                let Some((bgra, width, height, covered)) = extract_selection_bgra(sel, buf) else {
                    log::info!("command Ocr ignored: selection is outside the desktop bitmap");
                    return;
                };
                let request = OcrRequest {
                    bgra,
                    width,
                    height,
                    origin: covered,
                };
                // Fresh id + latch + flag per request: the Latch is
                // one-shot, so reusing one would replay the previous run's
                // result forever; the id is what lets a BACK-superseded
                // result be recognized as stale on pickup.
                cycle.ocr_req += 1;
                let req = cycle.ocr_req;
                let latch: Arc<Latch<Result<OcrOutcome, OcrError>>> = Arc::new(Latch::new());
                let cancel = Arc::new(AtomicBool::new(false));
                // First OCR press starts the system-font scan, overlapping
                // it with the recognizer child's own cold start. By
                // decision, neither runs before OCR is actually used.
                crate::ui::fonts::begin_system_font_scan();
                // Where the recognizer leaves its response file and `ocr.log`
                // (see ocr::recognize). None is normal: OCR has no session_dir
                // requirement, and only UPLOAD needs one.
                let session_dir = cycle.settings.session_dir.clone();
                // A dedicated detached thread, never joined: recognize()
                // blocks for the whole of the child process's run, while this
                // (winit) thread must never block — a join would freeze the
                // overlay for the entire recognition.
                let spawned = {
                    let latch = latch.clone();
                    let cancel = cancel.clone();
                    std::thread::Builder::new()
                        .name("ocr".into())
                        .spawn(move || {
                            // Checked on both sides of the expensive call
                            // (and polled inside it — recognize() kills the
                            // child when the flag goes up): before, to skip
                            // work the user already backed out of; after,
                            // to avoid setting a latch whose reader is
                            // gone, and because a canceled recognize()
                            // returns a placeholder error that must never
                            // be surfaced as a real outcome.
                            if cancel.load(Ordering::Acquire) {
                                return;
                            }
                            let result = ocr::recognize(&request, &cancel, session_dir.as_deref());
                            if cancel.load(Ordering::Acquire) {
                                return;
                            }
                            // Hold the result until the fallback-font scan
                            // lands, but only for a page that actually needs
                            // a face the bundled Inter lacks: publishing
                            // sooner would let the reveal lay non-Latin lines
                            // out against an Inter-only font set (tofu,
                            // cached for the request's lifetime), while an
                            // ASCII page has nothing to wait for. The scan
                            // started with this request and runs beside the
                            // child, so warm it is long done; the timeout
                            // covers a wedged cold scan.
                            if matches!(&result, Ok(o) if crate::ui::fonts::needs_fallback(o.lines.iter().map(|l| l.text.as_str()))) {
                                crate::ui::fonts::wait_for_system_font_scan(std::time::Duration::from_secs(10));
                            }
                            latch.set(result);
                        })
                };
                if let Err(e) = spawned {
                    log::warn!("failed to spawn the OCR worker thread: {e}");
                    cycle.input.ocr_notice = Some(OcrNotice {
                        anchor: Instant::now(),
                        kind: OcrNoticeKind::Failed,
                    });
                    broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                    return;
                }
                // A new request starts with a clean slate: a leftover
                // failure pill under the scan sweep would be read as this
                // attempt's verdict.
                cycle.input.ocr_notice = None;
                cycle.ocr_job = Some((req, latch, cancel));
                cycle.input.ocr = OcrState::Scanning {
                    anchor: Instant::now(),
                    req,
                    region: covered,
                };
                // The strip goes away for the scan, so the hand the press
                // left behind has nothing under it any more.
                self.broadcast_and_settle_pointer(window_id);
            }
            Command::OcrBack => self.exit_ocr_mode(window_id),
            Command::ScrollBack => self.exit_scroll_pick_mode(window_id),
            Command::OcrCopy => {
                // Lifted only: during Scanning there is no text yet (and
                // no strip on screen — this arm is belt-and-braces against
                // a stray dispatch, e.g. Enter's default_action racing a
                // state change).
                let OcrState::Lifted {
                    outcome,
                    ..
                } = &cycle.input.ocr
                else {
                    log::info!("command OcrCopy ignored: no lifted OCR result");
                    return;
                };
                let text = outcome.full_text.clone();
                hide_overlay_for_action(&self.windows);
                match copy_text_to_clipboard(&text) {
                    ActionResult::Success => self.finish_cycle(event_loop, CycleAction::OcrCopy),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Copy to Clipboard Failed", &msg, ErrorIcon)
                            .unwrap_or(false)
                        {
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
            Command::OcrSearch => {
                let OcrState::Lifted {
                    outcome,
                    ..
                } = &cycle.input.ocr
                else {
                    log::info!("command OcrSearch ignored: no lifted OCR result");
                    return;
                };
                let Some(url) = ocr::search::search_url(&outcome.full_text) else {
                    // Whitespace-only text: nothing to search for. Stay in
                    // the mode — the lifted lines are still on screen and
                    // the other buttons still work.
                    log::info!("command OcrSearch ignored: no searchable text");
                    return;
                };
                // ShellExecuteW may hand the URL to an ALREADY RUNNING
                // browser, which holds no activation rights of its own —
                // the usual hand-to-shell grant names the wrong pid and
                // would leave it flashing in the taskbar. ASFW_ANY lets
                // whoever takes the foreground next have it, and only works
                // while we are still the foreground window, so it must come
                // before the hide.
                SystemInterop::allow_any_foreground();
                // Deliberately NOT hide_overlay_for_action: that would hand
                // the (single-use) foreground grant to FlickSnap.Ui instead of
                // the browser — see the comment on hide_overlay_for_action.
                self.windows.hide_all();
                // Must stay on this (winit) thread: ShellExecuteW rides the
                // STA COM that SystemInterop::init established here and
                // fails silently from a worker thread.
                if SystemInterop::open_url(&url) {
                    self.finish_cycle(event_loop, CycleAction::OcrSearch);
                } else {
                    // Nothing was launched, so the overlay still owns the
                    // screen — re-show it and stay Lifted.
                    log::warn!("failed to open the browser for OCR search; returning to the overlay");
                    self.show_all_windows();
                }
            }
            Command::OcrUpload => {
                let OcrState::Lifted {
                    outcome,
                    ..
                } = &cycle.input.ocr
                else {
                    log::info!("command OcrUpload ignored: no lifted OCR result");
                    return;
                };
                // The Command::Video precedent: without a --session-dir
                // there is no shell listening for the marker — ignore.
                let Some(session_dir) = cycle.settings.session_dir.clone() else {
                    log::info!("command OcrUpload ignored: no --session-dir provided");
                    return;
                };
                let text = outcome.full_text.clone();
                // The shell treats an ocr-upload marker with whitespace-only
                // text as a canceled capture; don't emit one at all rather
                // than lean on that fallback. Unreachable from a Lifted
                // outcome (empty results never lift), hence belt-and-braces.
                if text.trim().is_empty() {
                    log::info!("command OcrUpload ignored: recognized text is empty");
                    return;
                }
                hide_overlay_for_action(&self.windows);
                match write_ocr_upload_action(&session_dir, &text) {
                    ActionResult::Success => self.finish_cycle(event_loop, CycleAction::OcrUpload),
                    ActionResult::Canceled => self.show_all_windows(),
                    ActionResult::Failed(msg) => {
                        if xdialog::show_message_retry_cancel("flickSnap Capture", "Text Upload Failed", &msg, ErrorIcon).unwrap_or(false) {
                            self.show_all_windows();
                        } else {
                            self.finish_cycle(event_loop, CycleAction::Canceled);
                        }
                    }
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.windows.is_empty() {
            return;
        }

        // Consume worker setups — each one gets a window + surface handoff.
        let worker_setups = match self.worker_setups.take() {
            Some(s) => s,
            None => return,
        };

        // The screenshot usually loses this race (the capture is ~90 ms, the
        // event loop reaches here at ~60), and then the windows are built
        // backdropless and hidden and try_pick_up_screenshot finishes the
        // job. When it HAS landed, this pickup routes us through the
        // original install-at-creation path below.
        self.try_pick_up_screenshot(event_loop);

        let mut windows = WindowSet::new();
        self.startup.mark_window_create_start();

        let desktop_buffer = self
            .cycle
            .as_ref()
            .and_then(|c| c.desktop_buffer.clone());

        for (i, setup) in worker_setups.into_iter().enumerate() {
            let m = &self.monitors[i];
            let width = m.bounds.size.width.max(1) as u32;
            let height = m.bounds.size.height.max(1) as u32;

            #[cfg(windows)]
            let (win_pos, win_size): (winit::dpi::Position, winit::dpi::Size) = (
                winit::dpi::PhysicalPosition::new(m.bounds.origin.x, m.bounds.origin.y).into(),
                winit::dpi::PhysicalSize::new(width, height).into(),
            );
            #[cfg(target_os = "macos")]
            let (win_pos, win_size): (winit::dpi::Position, winit::dpi::Size) = {
                let logical_pos = m.screen_to_logical(ScreenPoint::new(m.bounds.origin.x, m.bounds.origin.y));
                let logical_size = m.physical_to_logical_size(width, height);
                (
                    winit::dpi::LogicalPosition::new(logical_pos.x, logical_pos.y).into(),
                    winit::dpi::LogicalSize::new(logical_size.width, logical_size.height).into(),
                )
            };

            #[allow(unused_mut)]
            let mut attrs = Window::default_attributes()
                .with_title("flicksnap capture")
                .with_decorations(false)
                .with_resizable(false)
                .with_visible(false)
                .with_active(false)
                .with_position(win_pos)
                .with_inner_size(win_size);
            #[cfg(windows)]
            {
                // These are desktop overlays, never application windows. Apply
                // WS_EX_TOOLWINDOW at HWND creation so Explorer cannot briefly
                // add one taskbar button while the hidden window is shown and
                // focused (particularly visible after the standby transition).
                attrs = attrs.with_skip_taskbar(true);
                attrs = attrs.with_no_redirection_bitmap(true);
                // The overlay never accepts a drop, but winit registers every window
                // it creates as an OLE drop target — an OleInitialize plus a
                // RegisterDragDrop per monitor, on the startup path.
                attrs = attrs.with_drag_and_drop(false);
                // The capture must cover existing topmost windows, including Draw on
                // Screen. Leave room for an attached debugger if it pauses a debug build.
                if !cfg!(debug_assertions) || !unsafe { windows::Win32::System::Diagnostics::Debug::IsDebuggerPresent() }.as_bool() {
                    attrs = attrs.with_window_level(winit::window::WindowLevel::AlwaysOnTop);
                }
            }
            let window = match event_loop.create_window(attrs) {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    error!("failed to create window for monitor {i}: {e:?}");
                    self.record_window_failure();
                    continue;
                }
            };

            self.startup.background.workers[i]
                .surface_start
                .set_once(self.startup.t_start.elapsed());
            let handle = match WindowHandle::new(window, setup, &self.instance, desktop_buffer.as_deref()) {
                Ok(h) => h,
                Err(e) => {
                    error!("failed to create window handle for monitor {i}: {e:?}");
                    self.record_window_failure();
                    continue;
                }
            };
            self.startup.background.workers[i]
                .surface_bind
                .set_once(self.startup.t_start.elapsed());

            // Earliest point the window can go on screen without flashing —
            // `WindowHandle::new` has applied the window tweaks and installed the
            // frozen-desktop layer — and the last point before the worker it just
            // handed the surface to reaches frame 0. See `order_window_front_early`.
            // Only once the screenshot has landed: with the buffer still pending
            // the backdrop layer is empty and an ordered window is an opaque
            // black rectangle, so `try_pick_up_screenshot` fills the layer and
            // orders front in one motion when the capture arrives.
            #[cfg(target_os = "macos")]
            if desktop_buffer.is_some() {
                order_window_front_early(handle.winit_window());
            }

            windows.insert(handle);
        }

        if windows.is_empty() {
            error!("no windows created; exiting");
            // `App::new` hid the cursor before the event loop ran, and this is
            // the one exit that never reaches `finish_cycle` to put it back.
            set_hardware_cursor_visible(true);
            event_loop.exit();
            return;
        }

        self.startup.mark_window_create();
        self.windows = windows;
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.cycle.is_some() && self.worker_failed.load(Ordering::Acquire) != 0 {
            self.fatal = Some(anyhow!("a display preview could not be rendered"));
            self.finish_cycle(event_loop, CycleAction::Canceled);
            return;
        }
        // The screenshot — this is the pass that normally lands it,
        // installing the backdrops and ordering the windows front.
        self.try_pick_up_screenshot(event_loop);
        self.try_advance_ocr();

        // A host that scheduled a repaint (a hover fade, an Area sizing
        // pass) must be run even though nothing it depends on changed, and
        // so must one whose worker has published fresher frame timings for
        // the debug panel.
        if let Some(cycle) = self.cycle.as_mut() {
            let now = Instant::now();
            let repaint = cycle.egui.repaint_due(now);
            let fresh_perf = cycle
                .egui
                .debug_poll(&self.windows, &self.ui_monitors, cycle.input.debug_visible);
            if repaint || fresh_perf {
                broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
            }
        }

        let mut bench_done = false;
        if let Some(cycle) = self.cycle.as_mut() {
            // Frame 0 needs the snapshot. While it is in flight,
            // try_pick_up_screenshot handles the capture deadline.
            if cycle.desktop_buffer.is_none() {
                event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(4)));
                return;
            }
            if let Some(ref pending) = cycle.pending_show {
                let ready = pending.ready_count.load(Ordering::Acquire);
                let timed_out = pending.since.elapsed() >= SHOW_GATE_TIMEOUT;
                if timed_out && ready < pending.expected {
                    self.fatal = Some(anyhow!(
                        "timed out preparing the capture preview ({ready} of {} displays ready)",
                        pending.expected
                    ));
                    self.finish_cycle(event_loop, CycleAction::Canceled);
                    return;
                }
                if ready >= pending.expected {
                    self.startup.mark_show_start();
                    self.windows.show_all();
                    self.startup.mark_shown();
                    if let Some(h) = self.windows.first() {
                        h.focus();
                    }
                    update_cursor_visibility(&self.windows, cycle);
                    pending.visible_latch.signal_all();
                    cycle.pending_show = None;
                    broadcast_mouse_state(&self.windows, &cycle.input);
                    broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                    // The one place the whole startup timeline is complete and
                    // nothing the user does can still perturb it. Emitted
                    // unconditionally: without a log record these marks only
                    // ever existed inside the debug panel.
                    info!("{}", self.startup.report());

                    // No OCR pre-warm here, by decision: the flicksnap_ai child
                    // (84 MB executable + embedded models) and the system-font
                    // scan load lazily at the first OCR press instead
                    // (`Command::Ocr` above). A cold pre-warm at show competed
                    // with the deferred UI builds and the
                    // render loop for disk and every core — the post-show
                    // freeze — to pre-pay for a rarely-used feature.

                    bench_done = cycle.settings.bench_startup;
                }
            }
        }

        // `--bench-startup`: the run's only purpose was the record above, so
        // end the cycle the same way a cancel does — windows hidden, cursor
        // restored, no payload written.
        if bench_done {
            self.finish_cycle(event_loop, CycleAction::Canceled);
            return;
        }

        // Pre-select the active screen / foreground window when launched with
        // `--capture-mode screen|window` (no-op in free-region mode).
        self.try_preselect(event_loop);

        if let Some(cycle) = self.cycle.as_ref() {
            let now = Instant::now();
            let timer = if cycle.pending_show.is_some() {
                Some(now + Duration::from_millis(4))
            } else if cycle.ocr_job.is_some()
                || matches!(cycle.input.ocr, OcrState::Scanning { .. } | OcrState::Retracting { .. })
                || cycle.input.debug_visible
            {
                Some(now + crate::ui::egui_host::REPAINT_FLOOR)
            } else {
                None
            };
            let next = cycle
                .egui
                .next_repaint()
                .into_iter()
                .chain(timer)
                .min();
            event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let this_monitor_bounds = match self.windows.get(&id) {
            Some(h) => h.monitor_bounds(),
            None => return,
        };
        let handle_monitor_bounds = this_monitor_bounds;

        // The cycle is already finishing (the process is exiting).
        let Some(cycle) = self.cycle.as_mut() else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => {
                self.finish_cycle(event_loop, CycleAction::Canceled);
            }
            // UNDO's chord (`hotkey_letter` says which key is its Z), only
            // while the drawing strip is up to take it. Held down it
            // repeats, as an editor's undo does.
            WindowEvent::KeyboardInput {
                ref event,
                ..
            } if event.state == ElementState::Pressed
                && is_undo_chord(cycle.input.modifiers)
                && hotkey_letter(&event.key_without_modifiers(), event.physical_key) == Some('z') =>
            {
                if drawbar_is_up(&cycle.input) {
                    self.dispatch_command(Command::DrawUndo, event_loop, id);
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        logical_key: Key::Named(NamedKey::Escape),
                        // One step per physical press: held down, the key's
                        // auto-repeat would walk on past the step it was
                        // pressed for — shutting the colour popup, then
                        // cancelling the capture under it.
                        repeat: false,
                        ..
                    },
                ..
            } => match escape_step(&cycle.input) {
                EscapeStep::ClosePalette => self.close_palette(),
                EscapeStep::ExitOcr => self.exit_ocr_mode(id),
                EscapeStep::ExitScrollPick => self.exit_scroll_pick_mode(id),
                EscapeStep::Cancel => self.finish_cycle(event_loop, CycleAction::Canceled),
            },
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        logical_key: Key::Named(NamedKey::Enter),
                        repeat: false,
                        ..
                    },
                ..
            } if cycle.input.modifiers.is_empty() => {
                // Mirrors the Dx capturer: Return acts as the default
                // accept once a selection is made — "open in editor"
                // normally, COPY while OCR lines are lifted. Which (if
                // either) is `default_action`'s decision.
                if let Some(cmd) = default_action(&cycle.input) {
                    self.dispatch_command(cmd, event_loop, id);
                }
            }
            // The letter hotkeys, by `hotkey_letter`, so they answer on any
            // keyboard layout.
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: ElementState::Pressed,
                        ref logical_key,
                        physical_key,
                        repeat: false,
                        ..
                    },
                ..
            } if cycle.input.modifiers.is_empty() => {
                if let Some(c) = hotkey_letter(logical_key, physical_key) {
                    if c == 'd' {
                        cycle.input.debug_visible = !cycle.input.debug_visible;
                        update_cursor_visibility(&self.windows, cycle);
                        broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                    } else if cycle.input.scroll_pick_mode {
                        // Accelerators are scoped to the strip on screen,
                        // and while picking that is the scroll-pick strip:
                        // BACK and EXIT answer, every other letter — the
                        // Normal strip's included — is swallowed, so a
                        // button that is not up can never fire.
                        //
                        // M is swallowed here too, ahead of its handler
                        // below. Picking suppresses both of that toggle's
                        // feedback channels — the [M] hint is gone and the
                        // frozen cursor is force-hidden — so honouring the
                        // key would silently change whether the cursor lands
                        // in the saved image, with nothing on screen to say
                        // so. D stays live: it is a developer affordance and
                        // the debug panel is its own feedback.
                        let features = cycle.settings.panel_features;
                        if let Some(cmd) = panel::lookup_command_by_key(panel::model::PanelButtonSet::ScrollPick, features, c) {
                            self.dispatch_command(cmd, event_loop, id);
                        }
                    } else if cycle.input.ocr.active() {
                        // Accelerators are scoped to the strip on screen:
                        // only while the OCR strip is up (Lifted) do its
                        // keys dispatch; everything else is swallowed so
                        // an invisible button can never fire. Scanning
                        // shows NO panel (the sweep is still working, so
                        // there is nothing to act on — Escape above is the
                        // only exit) and swallows everything for the same
                        // reason. Retracting swallows everything too — the
                        // strip is mid-swap and the mode ends by itself in
                        // a fraction of a second.
                        //
                        // M is swallowed too, for the scroll-pick reasoning
                        // above: both of that toggle's feedback channels
                        // (the [M] hint and any image output) are suppressed
                        // in this mode, so honouring it would silently
                        // change the eventual image with nothing on screen
                        // to say so. D stays live (the arm above precedes).
                        if cycle.input.ocr.shows_ocr_panel() {
                            let features = cycle.settings.panel_features;
                            if let Some(cmd) = panel::lookup_command_by_key(panel::model::PanelButtonSet::Ocr, features, c) {
                                self.dispatch_command(cmd, event_loop, id);
                            }
                        }
                    } else if c == 'm' {
                        cycle.input.cursor_overlay_visible = !cycle.input.cursor_overlay_visible;
                        broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                    } else if c == 'x' {
                        cycle.input.loupe_visible = !cycle.input.loupe_visible;
                        broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                    } else if cycle.input.captured {
                        // Normal stays hardcoded here: the OCR arm above
                        // claims every keypress while ocr.active(), so this
                        // branch can only run with the Normal strip up —
                        // the two sets deliberately reuse letters. The
                        // feature switches are NOT hardcoded: a button the
                        // user turned off must not answer to its letter
                        // either, or the strip would be missing a button
                        // that still fires.
                        let features = cycle.settings.panel_features;
                        if let Some(cmd) = panel::lookup_command_by_key(panel::model::PanelButtonSet::Normal, features, c) {
                            self.dispatch_command(cmd, event_loop, id);
                        }
                    } else if cycle.input.mouse_down {
                        // Mid-drag: swallow keys.
                    } else {
                        match c {
                            't' => {
                                cycle.input.tips_visible = !cycle.input.tips_visible;
                                broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                            }
                            'q' => {
                                cycle.input.overlays_visible = !cycle.input.overlays_visible;
                                broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                                broadcast_mouse_state(&self.windows, &cycle.input);
                            }
                            'f' => {
                                let pt = to_screen_point(cycle.input.virtual_cursor);
                                if let Some(bounds) = self
                                    .monitors
                                    .iter()
                                    .find(|m| m.bounds.contains(pt))
                                    .map(|m| m.bounds)
                                {
                                    self.finalize_selection(bounds, event_loop, id);
                                }
                            }
                            'a' => {
                                self.finalize_selection(self.vd_bounds, event_loop, id);
                            }
                            _ => {}
                        }
                    }
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let was_shift = cycle.input.modifiers.shift_key();
                cycle.input.modifiers = modifiers.state();
                let shift = cycle.input.modifiers.shift_key();
                // Draw-on-screen re-snaps a mark the moment Shift goes down
                // or up, without waiting for the pointer to move.
                if shift != was_shift && cycle.input.marks.is_drawing() {
                    cycle.input.marks.resnap(shift);
                    broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                }
            }
            WindowEvent::CursorMoved {
                position,
                ..
            } => {
                let bounds = handle_monitor_bounds;
                let win_pt = WindowPoint::new(position.x as f32, position.y as f32);
                let os_vd = ScreenPoint::new(bounds.min_x() + win_pt.x.round() as i32, bounds.min_y() + win_pt.y.round() as i32);

                cycle.input.virtual_cursor = ScreenPointF::new(os_vd.x as f32, os_vd.y as f32);

                if cycle.input.mouse_down && !cycle.input.captured {
                    if let Some(start) = cycle.input.mouse_down_pt {
                        let psel = ScreenRect::from_rounded_threshold(
                            start.x,
                            start.y,
                            cycle.input.virtual_cursor.x,
                            cycle.input.virtual_cursor.y,
                        );
                        if !cycle.input.dragging {
                            let threshold = 6.0 / cycle.input.mouse_down_dpi;
                            let crossed = psel.is_some_and(|r| (r.width() as f32) > threshold || (r.height() as f32) > threshold);
                            if crossed {
                                cycle.input.dragging = true;
                            }
                        }
                        if cycle.input.dragging {
                            // A dragged rect is the user's own shape, not a
                            // window's: square however round the window the
                            // drag started over.
                            cycle.input.selection = psel;
                            cycle.input.selection_radius = 0.0;
                        }
                    }
                }

                // Set below once this move has a selection to hit-test
                // against, and acted on after the broadcast: the broadcast
                // is what runs the tray, and it is the only run that sees
                // this move's debug rows.
                let mut settle_cursor = false;
                if cycle.input.captured {
                    if cycle.input.marks.is_drawing() {
                        let snap = cycle.input.modifiers.shift_key();
                        cycle
                            .input
                            .marks
                            .drag_to(cycle.input.virtual_cursor, snap);
                    } else if let (Some(mode), Some(anchor), Some(start)) =
                        (cycle.input.drag_mode, cycle.input.drag_anchor_selection, cycle.input.mouse_down_pt)
                    {
                        let cur_x = cycle.input.virtual_cursor.x.floor() as i32;
                        let cur_y = cycle.input.virtual_cursor.y.floor() as i32;
                        let new_sel = match mode {
                            DragMode::Move => {
                                let dx = (cycle.input.virtual_cursor.x - start.x).round() as i32;
                                let dy = (cycle.input.virtual_cursor.y - start.y).round() as i32;
                                move_and_crop(anchor, dx, dy, self.vd_bounds)
                            }
                            DragMode::Resize(handle) => Some(resize_with_clamp(anchor, handle, cur_x, cur_y, self.vd_bounds)),
                        };
                        cycle.input.selection = new_sel;
                        // Moved or resized, the rect no longer outlines the
                        // window it came from; its corners go square.
                        cycle.input.selection_radius = 0.0;
                        // No broadcast here — the unconditional
                        // broadcast_ui_state at the end of CursorMoved
                        // covers this path.
                    } else if let Some(sel) = cycle.input.selection {
                        let dpi = dpi_at_point(cycle.input.virtual_cursor, &self.monitors);
                        cycle.input.hittest = hit_test(cycle.input.virtual_cursor, sel, dpi);
                        settle_cursor = true;
                        // In pick mode the OS pointer is hidden over the
                        // selection's interior and shown everywhere else,
                        // so this move may have crossed that line. Both
                        // hide paths are idempotent (a global atomic
                        // guards the one syscall), so calling it per move
                        // costs nothing when nothing changed.
                        if cycle.input.scroll_pick_mode {
                            update_cursor_visibility(&self.windows, cycle);
                        }
                    }
                }

                let panel = broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                if settle_cursor {
                    // The tray is egui's now, and the broadcast above just
                    // laid the strip out with this move's pointer, so what
                    // it found under that pointer is what the cursor
                    // follows.
                    let cursor = cursor_for(&cycle.input, panel.over_button, panel.over_tray);
                    set_cursor_if_changed(&self.windows, &mut cycle.last_cursor, id, cursor);
                }
                broadcast_mouse_state(&self.windows, &cycle.input);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if !cycle.input.overlays_visible {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        let point = cycle.input.virtual_cursor;
                        let (half_width, half_height) = double_click_half_size(dpi_at_point(point, &self.monitors));
                        let blocks_click =
                            cycle
                                .panel_swap
                                .press(point, Instant::now(), panel_swap_guard_window(), half_width, half_height);
                        if cycle.input.captured {
                            // One egui tick that both lays the strip out
                            // and answers the press: egui reports the
                            // click synchronously, so routing stays on the
                            // app thread.
                            let out = cycle
                                .egui
                                .panel_press(&cycle.input, &cycle.settings, &self.ui_monitors);
                            if let Some(cmd) = out.clicked {
                                if blocks_click {
                                    log::info!("panel click ignored: second press of a double-click across a panel swap");
                                    return;
                                }
                                // The button ids encode their set, so the
                                // command can only come from the strip
                                // this tick actually laid out.
                                self.dispatch_command(cmd, event_loop, id);
                                return;
                            }
                            // A press anywhere but on the colour popup
                            // shuts it, and then does what it would have
                            // done with the popup shut.
                            if !out.over_palette {
                                self.close_palette();
                            }
                            let Some(cycle) = self.cycle.as_mut() else {
                                return;
                            };
                            if out.over_tray {
                                // Dead tray: chassis, emblem, readout,
                                // padding. The tray owns the click, as
                                // the C# strips do, so it cannot arm a
                                // move/resize drag of the selection
                                // underneath. Deliberately not gated by
                                // PanelSwapGuard: nothing is dispatched.
                                return;
                            }
                            // While a scroll point is being picked, a
                            // press in the selection's interior IS the
                            // pick; the handles stay live around it so the
                            // region can still be trimmed to the scrolling
                            // area, and a press outside does nothing (the
                            // driver may only aim inside the region it is
                            // going to stitch). Moving the selection is the
                            // one gesture the mode takes away: its whole
                            // interior belongs to the aim.
                            if cycle.input.scroll_pick_mode {
                                match cycle.input.hittest {
                                    Hittest::Inside => {
                                        // Pick mode is armed by a panel press
                                        // (SCROLL) that also swaps the strip —
                                        // a set change the swap guard records.
                                        // Without this check the second press
                                        // of a double-click on SCROLL would be
                                        // taken as the pick and write the
                                        // scroll action aimed at the button's
                                        // own location. Same property as the
                                        // panel guard above: one physical
                                        // double-click, one command.
                                        if blocks_click {
                                            log::info!("scroll pick ignored: second press of a double-click across a panel swap");
                                            return;
                                        }
                                        self.dispatch_scroll_pick(event_loop);
                                    }
                                    Hittest::Outside => {}
                                    handle => {
                                        cycle.input.mouse_down = true;
                                        cycle.input.mouse_down_pt = Some(cycle.input.virtual_cursor);
                                        cycle.input.drag_mode = Some(DragMode::Resize(handle));
                                        cycle.input.drag_anchor_selection = cycle.input.selection;
                                    }
                                }
                                return;
                            }
                            // The selection is frozen under the lifted
                            // lines — their geometry was computed against
                            // it — so drag/resize never arms while OCR mode
                            // is active; a click outside the panel simply
                            // does nothing there.
                            // With a drawing tool picked the screen is the
                            // canvas: a press inside the selection or out
                            // starts a mark instead of a move. The handles
                            // still resize, and the strips took their own
                            // presses above.
                            if !cycle.input.ocr.active() && is_canvas(cycle.input.hittest) {
                                if let Some(tool) = cycle.input.draw.tool {
                                    let at = cycle.input.virtual_cursor;
                                    let scale = dpi_at_point(at, &self.monitors);
                                    let ink = cycle.input.draw.ink();
                                    cycle.input.marks.begin(tool, at, ink, scale);
                                    cycle.input.mouse_down = true;
                                    cycle.input.mouse_down_pt = Some(at);
                                    broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                                    return;
                                }
                            }
                            if !cycle.input.ocr.active() {
                                let drag_mode = match cycle.input.hittest {
                                    Hittest::Inside => Some(DragMode::Move),
                                    Hittest::Outside => None,
                                    handle => Some(DragMode::Resize(handle)),
                                };
                                if drag_mode.is_some() {
                                    cycle.input.mouse_down = true;
                                    cycle.input.mouse_down_pt = Some(cycle.input.virtual_cursor);
                                    cycle.input.drag_mode = drag_mode;
                                    cycle.input.drag_anchor_selection = cycle.input.selection;
                                }
                            }
                            return;
                        }
                        cycle.input.mouse_down = true;
                        cycle.input.mouse_down_pt = Some(cycle.input.virtual_cursor);
                        cycle.input.mouse_down_dpi = dpi_at_point(cycle.input.virtual_cursor, &self.monitors);
                        cycle.input.dragging = false;
                        broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                    }
                    ElementState::Released => {
                        // The pointer is not hit-tested while a captured
                        // gesture drags, so the release settles it where
                        // it let go — the next press must not act on where
                        // the gesture began.
                        let gesture_ended = gesture_in_progress(&cycle.input);
                        // A mark being dragged out is finished where the
                        // pointer let go.
                        cycle.input.marks.finish();
                        let finalizing = cycle.input.mouse_down && !cycle.input.captured && cycle.input.selection.is_some();
                        let was_move_drag = matches!(cycle.input.drag_mode, Some(DragMode::Move));
                        cycle.input.mouse_down = false;
                        cycle.input.mouse_down_pt = None;
                        cycle.input.dragging = false;
                        cycle.input.drag_mode = None;
                        cycle.input.drag_anchor_selection = None;
                        if was_move_drag && cycle.input.captured && cycle.input.selection.is_none() {
                            cycle.input.captured = false;
                            // Unreachable while OCR is active (drags never
                            // arm there), but this un-capture path bypasses
                            // InteractionController::reset, so clear the
                            // mode and its notice explicitly — both are
                            // anchored to the selection that just ceased to
                            // exist.
                            cycle.input.ocr = OcrState::Idle;
                            cycle.input.ocr_notice = None;
                            // So are the drawing tool and the marks.
                            cycle.input.draw.reset();
                            cycle.input.marks.clear();
                            cycle.input.hittest = Hittest::Outside;
                            update_cursor_visibility(&self.windows, cycle);
                            set_cursor_if_changed(&self.windows, &mut cycle.last_cursor, id, CursorIcon::Default);
                            broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                        }
                        if finalizing {
                            cycle.input.captured = true;
                            update_cursor_visibility(&self.windows, cycle);
                            if let Some(sel) = cycle.input.selection {
                                let dpi = dpi_at_point(cycle.input.virtual_cursor, &self.monitors);
                                let ht = hit_test(cycle.input.virtual_cursor, sel, dpi);
                                cycle.input.hittest = ht;
                                set_cursor_if_changed(&self.windows, &mut cycle.last_cursor, id, ht.cursor());
                            }
                            // Mouse-release drag-select captured-transition
                            // site (DESIGN §3.3).
                            self.on_captured(event_loop, id);
                        }
                        // on_captured may have finished the cycle (--video);
                        // re-borrow instead of assuming it is still alive.
                        let Some(cycle) = self.cycle.as_mut() else {
                            return;
                        };
                        let settle = gesture_ended && cycle.input.captured;
                        if settle {
                            if let Some(sel) = cycle.input.selection {
                                let dpi = dpi_at_point(cycle.input.virtual_cursor, &self.monitors);
                                cycle.input.hittest = hit_test(cycle.input.virtual_cursor, sel, dpi);
                            }
                            update_cursor_visibility(&self.windows, cycle);
                        }
                        let panel = broadcast_ui_state(&self.windows, &self.monitors, &self.ui_monitors, cycle);
                        if settle {
                            let cursor = cursor_for(&cycle.input, panel.over_button, panel.over_tray);
                            set_cursor_if_changed(&self.windows, &mut cycle.last_cursor, id, cursor);
                        }
                        broadcast_mouse_state(&self.windows, &cycle.input);
                    }
                }
            }
            // Without this override winit resizes the window by
            // new_scale/old_scale, permanently shrinking any overlay whose
            // monitor has a different DPI than the primary.
            #[cfg(windows)]
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                let _ = inner_size_writer.request_inner_size(winit::dpi::PhysicalSize::new(
                    handle_monitor_bounds.width() as u32,
                    handle_monitor_bounds.height() as u32,
                ));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A freshly-shown overlay: nothing selected, no modes engaged.
    fn input() -> InteractionState {
        InteractionState {
            virtual_cursor: ScreenPointF::new(0.0, 0.0),
            mouse_down: false,
            mouse_down_pt: None,
            mouse_down_dpi: 1.0,
            dragging: false,
            selection: None,
            selection_radius: 0.0,
            captured: false,
            hittest: Hittest::Outside,
            drag_mode: None,
            drag_anchor_selection: None,
            tips_visible: false,
            loupe_visible: false,
            debug_visible: false,
            overlays_visible: true,
            pointer_over_tray: false,
            cursor_overlay_visible: false,
            scroll_pick_mode: false,
            ocr: OcrState::Idle,
            ocr_notice: None,
            draw: DrawState::default(),
            marks: Marks::default(),
            modifiers: ModifiersState::empty(),
        }
    }

    /// The reticle stands in for the OS pointer over the selection's
    /// interior while a scroll point is being picked — and nowhere else.
    /// The tray is the case worth pinning: the scroll-picker is locked to
    /// a row, so a selection with no room beneath it gets the strip placed
    /// INSIDE the region, where the hittest is `Inside` but the pointer is
    /// over live buttons and has to stay visible.
    #[test]
    fn the_reticle_stands_in_over_the_interior_but_never_over_the_tray() {
        // Picking, over the interior: the reticle is the pointer.
        assert!(reticle_stands_in(true, true, Hittest::Inside, false));
        // Same place, but the strip is under the pointer there.
        assert!(!reticle_stands_in(true, true, Hittest::Inside, true));
        // The handles and the desktop keep their own pointers.
        for hittest in [Hittest::Outside, Hittest::TopLeft, Hittest::Right] {
            assert!(!reticle_stands_in(true, true, hittest, false), "{hittest:?}");
        }
        // Not picking at all.
        assert!(!reticle_stands_in(false, true, Hittest::Inside, false));
        // Overlays hidden: the reticle is suppressed, so the pointer must
        // come back or there would be nothing to aim with.
        assert!(!reticle_stands_in(true, false, Hittest::Inside, false));
    }

    /// With a drawing tool picked the selection's interior and the screen
    /// around it wear the ordinary pointer rather than the move arrows,
    /// the handles keep their resize arrows, and the strips keep theirs.
    #[test]
    fn a_picked_tool_turns_the_screen_into_a_canvas() {
        let canvas = |hittest: Hittest, picked: bool| {
            let mut input = InteractionState {
                captured: true,
                hittest,
                ..input()
            };
            if picked {
                input
                    .draw
                    .toggle_tool(crate::draw::DrawTool::Arrow);
            }
            input
        };
        assert_eq!(cursor_for(&canvas(Hittest::Inside, true), false, false), CursorIcon::Default);
        assert_eq!(cursor_for(&canvas(Hittest::Inside, true), true, true), CursorIcon::Pointer);
        assert_eq!(
            cursor_for(&canvas(Hittest::BottomRight, true), false, false),
            Hittest::BottomRight.cursor()
        );
        assert_eq!(cursor_for(&canvas(Hittest::Outside, true), false, false), CursorIcon::Default);
        assert_eq!(cursor_for(&canvas(Hittest::Inside, false), false, false), Hittest::Inside.cursor());
        assert!(is_canvas(Hittest::Inside) && is_canvas(Hittest::Outside));
        assert!(!is_canvas(Hittest::BottomRight) && !is_canvas(Hittest::Top));
    }

    /// The pointer's look is a function of the state and what the tray
    /// found under it, so a mode change can settle it without waiting for
    /// the user to move the mouse. Clicking OCR or SCROLL used to leave
    /// the hand (or the picker's crosshair) behind until the next
    /// `CursorMoved`.
    #[test]
    fn the_cursor_follows_the_state_and_what_the_tray_found_under_it() {
        let captured = || InteractionState {
            captured: true,
            hittest: Hittest::Inside,
            ..input()
        };

        // On a button, in every mode: the hand.
        assert_eq!(cursor_for(&captured(), true, true), CursorIcon::Pointer);
        assert_eq!(
            cursor_for(
                &InteractionState {
                    scroll_pick_mode: true,
                    ..captured()
                },
                true,
                true
            ),
            CursorIcon::Pointer
        );

        // On the dead chassis: the arrow, never a resize handle — the
        // press is swallowed, so no drag can start.
        assert_eq!(cursor_for(&captured(), false, true), CursorIcon::Default);

        // Picking, off the strip, inside the selection: the reticle is the
        // pointer and the crosshair is only the fallback for a failed
        // hardware hide.
        assert_eq!(
            cursor_for(
                &InteractionState {
                    scroll_pick_mode: true,
                    ..captured()
                },
                false,
                false
            ),
            CursorIcon::Crosshair
        );

        // OCR freezes the selection, so no resize arrows over it.
        assert_eq!(
            cursor_for(
                &InteractionState {
                    ocr: OcrState::Scanning {
                        anchor: Instant::now(),
                        req: 1,
                        region: ScreenRect::from_xy_size(0, 0, 10, 10),
                    },
                    ..captured()
                },
                false,
                false
            ),
            CursorIcon::Default
        );

        // Otherwise the hit-test decides.
        assert_eq!(cursor_for(&captured(), false, false), Hittest::Inside.cursor());
        let outside = InteractionState {
            hittest: Hittest::Outside,
            ..captured()
        };
        assert_eq!(cursor_for(&outside, false, false), Hittest::Outside.cursor());
    }

    /// A result-shaped payload for gating assertions; recognition itself is
    /// never exercised here.
    fn dummy_outcome() -> Arc<OcrOutcome> {
        Arc::new(OcrOutcome {
            lines: Vec::new(),
            full_text: String::new(),
            text_angle: 0.0,
        })
    }

    #[test]
    fn enter_opens_editor_once_captured() {
        let mut i = input();
        assert_eq!(default_action(&i), None);
        i.captured = true;
        assert_eq!(default_action(&i), Some(Command::Edit));
    }

    /// The panel is hidden while a scroll point is being picked, and Enter
    /// is its default button: firing Edit here would silently replace the
    /// scrolling capture with a plain screenshot.
    #[test]
    fn enter_inert_while_picking_scroll_point() {
        let mut i = input();
        i.captured = true;
        i.scroll_pick_mode = true;
        assert_eq!(default_action(&i), None);
    }

    /// With OCR lines lifted, the default accept is the recognized text,
    /// not the editor: Edit would write a plain screenshot session and
    /// destroy the OCR result.
    #[test]
    fn enter_copies_while_ocr_lifted() {
        let mut i = input();
        i.captured = true;
        i.ocr = OcrState::Lifted {
            anchor: Instant::now(),
            req: 1,
            region: ScreenRect::from_xy_size(0, 0, 10, 10),
            dpi_scale: 1.0,
            outcome: dummy_outcome(),
        };
        assert_eq!(default_action(&i), Some(Command::OcrCopy));
    }

    /// The transitional phases have nothing to accept: Scanning has no text
    /// yet, Retracting is on its way out of the mode.
    #[test]
    fn enter_inert_while_ocr_scanning() {
        let mut i = input();
        i.captured = true;
        i.ocr = OcrState::Scanning {
            anchor: Instant::now(),
            req: 1,
            region: ScreenRect::from_xy_size(0, 0, 10, 10),
        };
        assert_eq!(default_action(&i), None);

        i.ocr = OcrState::Retracting {
            anchor: Instant::now(),
        };
        assert_eq!(default_action(&i), None);
    }

    /// Ctrl+Z alone is the chord (Cmd+Z on macOS); with Shift or Alt it is
    /// something else.
    #[test]
    fn the_undo_chord_is_the_command_key_alone() {
        let command = if cfg!(target_os = "macos") {
            ModifiersState::SUPER
        } else {
            ModifiersState::CONTROL
        };
        assert!(is_undo_chord(command));
        assert!(!is_undo_chord(ModifiersState::empty()));
        assert!(!is_undo_chord(command | ModifiersState::SHIFT));
        assert!(!is_undo_chord(command | ModifiersState::ALT));
        assert!(!is_undo_chord(ModifiersState::CONTROL | ModifiersState::SUPER));
    }

    /// Ctrl+Z answers only while the drawing strip is up to take it: not
    /// before a capture, not in the scroll-picker, not mid-drag.
    #[test]
    fn the_undo_chord_needs_the_drawing_strip() {
        let mut i = input();
        assert!(!drawbar_is_up(&i), "nothing captured");
        i.captured = true;
        assert!(drawbar_is_up(&i));
        i.scroll_pick_mode = true;
        assert!(!drawbar_is_up(&i), "the scroll-picker's strip is up");
    }

    /// A hotkey's letter is the Latin letter the key types — so Z is the
    /// QWERTZ Z, not its Y — and on a layout without Latin letters the
    /// letter in the key's QWERTY place, so T, F, A… and Ctrl+Z answer on a
    /// Russian layout too.
    #[test]
    fn hotkeys_answer_to_the_layouts_letter_on_any_layout() {
        let key = |s: &str| Key::Character(s.into());
        let at = PhysicalKey::Code;
        assert_eq!(hotkey_letter(&key("z"), at(KeyCode::KeyZ)), Some('z'), "QWERTY");
        assert_eq!(hotkey_letter(&key("Z"), at(KeyCode::KeyZ)), Some('z'), "Caps Lock");
        assert_eq!(hotkey_letter(&key("z"), at(KeyCode::KeyY)), Some('z'), "QWERTZ's Z");
        assert_eq!(hotkey_letter(&key("y"), at(KeyCode::KeyZ)), Some('y'), "QWERTZ's Y");
        assert_eq!(hotkey_letter(&key("я"), at(KeyCode::KeyZ)), Some('z'), "Russian Z");
        assert_eq!(hotkey_letter(&key("е"), at(KeyCode::KeyT)), Some('t'), "Russian T");
        assert_eq!(hotkey_letter(&key("1"), at(KeyCode::Digit1)), None, "not a letter");
        assert_eq!(hotkey_letter(&key("'"), at(KeyCode::KeyQ)), None, "Dvorak punctuation");
        assert_eq!(hotkey_letter(&key("€"), at(KeyCode::KeyE)), None, "AltGr symbol");
        assert_eq!(hotkey_letter(&key("é"), at(KeyCode::KeyE)), None, "extended Latin");
        assert_eq!(hotkey_letter(&Key::Named(NamedKey::Escape), at(KeyCode::Escape)), None);
    }

    /// Every command waits while a press drags on the capture — a mark
    /// being drawn, the selection moved or resized — and none waits on a
    /// drag that has not captured yet.
    #[test]
    fn a_gesture_holds_the_commands_until_its_release() {
        let mut i = input();
        i.captured = true;
        assert!(!gesture_in_progress(&i));
        i.marks
            .begin(crate::draw::DrawTool::Line, ScreenPointF::new(0.0, 0.0), egui::Color32::RED, 1.0);
        assert!(gesture_in_progress(&i), "a mark is being drawn");
        i.marks.finish();
        assert!(!gesture_in_progress(&i));
        i.drag_mode = Some(DragMode::Move);
        assert!(gesture_in_progress(&i), "the selection is being moved");
        i.captured = false;
        assert!(!gesture_in_progress(&i), "a selection still being dragged out");
    }

    /// Escape shuts an open colour popup and nothing more: the capture,
    /// its selection and the marks on it survive the press.
    #[test]
    fn escape_shuts_the_colour_popup_before_it_cancels_anything() {
        let mut i = input();
        i.captured = true;
        assert_eq!(escape_step(&i), EscapeStep::Cancel);

        i.draw.toggle_palette();
        assert_eq!(escape_step(&i), EscapeStep::ClosePalette);
        i.draw.close_palette();
        assert_eq!(escape_step(&i), EscapeStep::Cancel);

        i.scroll_pick_mode = true;
        assert_eq!(escape_step(&i), EscapeStep::ExitScrollPick);
    }

    #[test]
    fn panel_swap_blocks_a_second_press_at_the_same_position_only() {
        use panel::model::PanelButtonSet::{Normal, Ocr, ScrollPick};
        let t = Instant::now();
        let window = Duration::from_millis(500);
        let soon = t + Duration::from_millis(100);
        let here = ScreenPointF::new(20.0, 20.0);
        let nearby = ScreenPointF::new(21.0, 21.0);
        let elsewhere = ScreenPointF::new(55.0, 60.0);
        let mut guard = PanelSwapGuard::default();

        guard.observe(Some(Normal));
        assert!(
            !guard.press(here, t, window, 2.0, 2.0),
            "the first click after preselect is independent"
        );
        guard.observe(Some(Ocr));
        assert!(
            guard.press(nearby, soon, window, 2.0, 2.0),
            "the next click at the same spot hits a different panel"
        );
        assert!(
            !guard.press(elsewhere, soon, window, 2.0, 2.0),
            "a fast click elsewhere still works"
        );
        guard.observe(Some(Ocr));
        assert!(
            !guard.press(elsewhere, soon, window, 2.0, 2.0),
            "an unchanged set does not block repeat commands"
        );
        guard.observe(Some(ScrollPick));
        assert!(
            !guard.press(elsewhere, soon + window, window, 2.0, 2.0),
            "the double-click interval has passed"
        );
    }

    #[test]
    fn first_panel_click_works_at_the_end_of_a_drag() {
        let t = Instant::now();
        let window = Duration::from_millis(500);
        let mut guard = PanelSwapGuard::default();
        assert!(!guard.press(ScreenPointF::new(10.0, 10.0), t, window, 2.0, 2.0));
        guard.observe(Some(panel::model::PanelButtonSet::Normal));
        assert!(!guard.press(ScreenPointF::new(210.0, 100.0), t + Duration::from_millis(260), window, 2.0, 2.0));
    }

    /// The live window comes from GetDoubleClickTime; the clamp guarantees
    /// a broken registry value can neither disable the guard nor wedge the
    /// panel shut.
    #[cfg(windows)]
    #[test]
    fn panel_swap_guard_window_is_sane() {
        let w = panel_swap_guard_window();
        assert!(w >= Duration::from_millis(100) && w <= Duration::from_millis(2000), "window {w:?}");
    }
}
