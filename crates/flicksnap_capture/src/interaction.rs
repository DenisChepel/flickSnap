use std::sync::Arc;
use std::time::Instant;

use crate::draw::{DrawState, Marks};
use crate::ocr::OcrOutcome;
use crate::selection::{dpi_at_point, hit_test, DragMode, Hittest};
use crate::system::MonitorInfo;
use flicksnap_shared::geometry::{ScreenPointF, ScreenRect};
use winit::keyboard::ModifiersState;
use winit::window::CursorIcon;

/// How long an OCR notice stays up before it expires, in seconds. The last
/// [`NOTICE_FADE_SECS`] of that window fades it out.
pub const NOTICE_SECS: f32 = 2.5;
pub const NOTICE_FADE_SECS: f32 = 0.5;

/// Which "OCR did not give you anything" message the overlay is showing.
///
/// This exists because the alternative is worse than missing polish: press
/// OCR, get nothing back, and the overlay silently snaps to the captured
/// state — indistinguishable from a dead button. A dialog is not an option
/// (the overlay is topmost and fullscreen, so a dialog would open *behind*
/// it), hence an in-overlay pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrNoticeKind {
    /// Recognition succeeded but produced zero lines.
    NoText,
    /// No recognizer at all — no language pack, or a platform with no OCR.
    Unavailable,
    /// The engine errored. The detail goes to the log, not to the user:
    /// a WinRT HRESULT string is noise on screen.
    Failed,
}

impl OcrNoticeKind {
    /// Short, plain user-facing text. Deliberately terse — the pill is
    /// small, transient, and sits over the user's own screen content.
    pub fn message(&self) -> &'static str {
        match self {
            OcrNoticeKind::NoText => "No text found",
            OcrNoticeKind::Unavailable => "OCR is not available on this PC",
            OcrNoticeKind::Failed => "OCR failed",
        }
    }
}

/// A transient notice pill shown over the selection.
///
/// `anchor` is an absolute `Instant` for the same reason [`OcrState`]'s is:
/// the fade must be a pure function of wall-clock elapsed time, not of
/// frame counts, so every host computes the same opacity at the same
/// moment.
///
/// The egui host schedules the fade: the pill's inputs exist only while it
/// is visible, and the pass that paints it asks for another run at the
/// repaint floor until it expires.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OcrNotice {
    pub anchor: Instant,
    pub kind: OcrNoticeKind,
}

impl OcrNotice {
    /// Whether the notice should still be drawn at all.
    pub fn visible(&self) -> bool {
        self.anchor.elapsed().as_secs_f32() < NOTICE_SECS
    }
}

/// The fade curve: solid, then a linear ramp to zero across the final
/// [`NOTICE_FADE_SECS`]. A pure function of elapsed seconds so it is
/// testable without doing arithmetic on `Instant` (subtracting from
/// `Instant::now()` can panic on a freshly-booted machine, and adding to it
/// cannot be observed by `elapsed()`), and so the pass that paints the pill
/// reads it straight off the anchor it was handed.
pub fn notice_alpha(elapsed: f32) -> f32 {
    let fade_starts = NOTICE_SECS - NOTICE_FADE_SECS;
    if elapsed <= fade_starts {
        return 1.0;
    }
    // Clamped rather than assumed in-range: a notice left up across a
    // suspend/resume can come back with an arbitrarily large elapsed.
    (1.0 - (elapsed - fade_starts) / NOTICE_FADE_SECS).clamp(0.0, 1.0)
}

/// Where the OCR "lift-and-act" mode is in its lifecycle. One enum rather
/// than a bool plus a result pair, so illegal states — a pending result
/// outside the mode, a mode with no result — are unrepresentable.
///
/// Every variant that animates carries an `anchor`: the animation clock's
/// t=0 for the CURRENT phase, as an absolute `Instant`. That is the whole
/// point of the shape. Each render worker free-runs at its own monitor's
/// refresh rate and would otherwise anchor its clock at BeginCycle delivery
/// time; the resulting skew would pull the per-line stagger apart across
/// monitors. `Instant` is `Copy + Send` off one process-global monotonic
/// source, so every worker derives byte-identical geometry from
/// `anchor.elapsed()`.
///
#[derive(Debug, Clone)]
pub enum OcrState {
    Idle,
    /// Recognition in flight; the scanning sweep plays over `region`.
    /// `req` is a per-cycle monotonic id — a late result whose id no longer
    /// matches is discarded. BACK leaves the same cycle alive, so nothing
    /// coarser than this id can tell a stale result from a current one.
    Scanning {
        anchor: Instant,
        req: u64,
        region: ScreenRect,
    },
    /// Lines recognized; the reveal pass sweeps top→bottom raising them,
    /// and the OCR button set is live.
    /// `req` is the id of the request that produced `outcome` (same
    /// counter Scanning carries): unique within the cycle, so the overlay's
    /// inputs compare on it instead of the recognised text — an `Arc`
    /// address could be reused by a later outcome, an id cannot.
    /// `dpi_scale` is the scale of the monitor containing the region's
    /// center — ONE value for all lift geometry, so a line crossing a
    /// mixed-DPI seam moves by the same physical amount on both halves
    /// instead of tearing at the seam.
    Lifted {
        anchor: Instant,
        req: u64,
        region: ScreenRect,
        dpi_scale: f32,
        outcome: Arc<OcrOutcome>,
    },
    /// BACK/Escape pressed. The text does NOT animate out — every bubble
    /// and crop vanishes on the first frame of this phase (see
    /// `anim::RETRACT_DURATION_SECS`) — so all this phase does is fade the
    /// region's dim/desaturation back to color, which is why it carries
    /// no outcome: there is nothing left to draw that needs one. The app
    /// thread flips to `Idle` once `anchor.elapsed()` passes the fade
    /// duration.
    Retracting {
        anchor: Instant,
    },
}

impl OcrState {
    /// Any non-Idle phase. Used to freeze selection drag/resize and gate
    /// input: the selection must not move out from under lifted lines whose
    /// geometry was computed against it.
    pub fn active(&self) -> bool {
        !matches!(self, OcrState::Idle)
    }

    /// Phases that show the OCR button set: `Lifted` ONLY.
    ///
    /// `Scanning` deliberately shows no panel at all (see [`Self::hides_panel`]
    /// and `ui::shared::active_panel_set`): while the sweep is looping there
    /// is no text yet, so COPY/SEARCH/UPLOAD would be lit but dead — the
    /// strip appears exactly when it becomes usable. `Retracting` shows the
    /// Normal set again — BACK restores the familiar buttons instantly
    /// while the retract animation plays out purely cosmetically.
    pub fn shows_ocr_panel(&self) -> bool {
        matches!(self, OcrState::Lifted { .. })
    }

    /// Phases where no panel may be on screen at all. Split from
    /// [`Self::shows_ocr_panel`] so `active_panel_set` reads as policy
    /// rather than pattern-matching: during the scan the only live inputs
    /// are Escape and waiting, and nothing clickable may suggest otherwise.
    pub fn hides_panel(&self) -> bool {
        matches!(self, OcrState::Scanning { .. })
    }
}

pub(crate) struct InteractionState {
    /// The pointer, in virtual-desktop physical px.
    pub virtual_cursor: ScreenPointF,
    pub mouse_down: bool,
    pub mouse_down_pt: Option<ScreenPointF>,
    pub mouse_down_dpi: f32,
    pub dragging: bool,
    pub selection: Option<ScreenRect>,
    /// Corner radius of `selection` in physical px; 0 = square. Non-zero
    /// only while the selection IS a window the user picked (hover, click
    /// without drag, `W`, `--capture-mode window`) — it is the radius the
    /// OS composites that window with. Every other way a rect comes to be
    /// (drag-select, F / A, moving or resizing a captured window selection)
    /// sets it back to 0: the rect no longer describes a window's shape.
    /// Rides along to the render workers with the selection and into the
    /// copy / save / preview crop.
    pub selection_radius: f32,
    pub captured: bool,
    pub hittest: Hittest,
    pub drag_mode: Option<DragMode>,
    pub drag_anchor_selection: Option<ScreenRect>,
    /// The Tips & Hotkeys panel is up (T shows and hides it).
    pub tips_visible: bool,
    pub loupe_visible: bool,
    pub debug_visible: bool,
    pub overlays_visible: bool,
    pub cursor_overlay_visible: bool,
    /// SCROLL was pressed and we are waiting for the user to click the
    /// point the driver will aim wheel events from. The selection is
    /// already captured and frozen — this mode only collects a point, so
    /// the panel hides, a reticle stands in for the pointer, and clicks are
    /// routed to the picker instead of the panel/drag machinery. Escape
    /// leaves the mode without canceling the cycle.
    pub scroll_pick_mode: bool,
    /// Whether the pointer was on the button tray when the last broadcast
    /// laid it out (`PanelOutcome::over_tray`, chassis and padding
    /// included). Carried here so the decisions taken outside that pass —
    /// `app::update_cursor_visibility` above all — answer from the same
    /// layout the tray itself did, rather than hit-testing it a second
    /// time against a rect that may have moved.
    ///
    /// False whenever there is no tray.
    pub pointer_over_tray: bool,
    /// Where the OCR lift-and-act mode is in its lifecycle — see
    /// [`OcrState`]. Mirrored verbatim onto `UiSharedState` so the lifted
    /// lines, the modal input gates and the panel set all swap in one
    /// atomic broadcast.
    pub ocr: OcrState,
    /// A transient "that produced nothing" pill — see [`OcrNotice`]. It
    /// deliberately outlives the OCR attempt that raised it: by the time
    /// the user reads it, `ocr` is back to `Idle`, so it cannot live inside
    /// the state enum.
    pub ocr_notice: Option<OcrNotice>,
    /// The drawing strip's picks: the tool, the colour, whether the colour
    /// popup is open. A reset puts the tool down with the selection it was
    /// picked for (`DrawState::reset`).
    pub draw: DrawState,
    /// What has been drawn on the selection, and the mark being dragged.
    pub marks: Marks,
    /// The keyboard modifiers held: Shift snaps a mark being drawn
    /// (`draw::geometry::snap_for`), and Ctrl+Z (Cmd+Z on macOS) is UNDO.
    pub modifiers: ModifiersState,
}

#[derive(Default)]
pub(crate) struct InteractionEffects {
    pub broadcast_mouse: bool,
    pub broadcast_ui: bool,
    pub update_cursor_visibility: bool,
    pub set_cursor: Option<CursorIcon>,
}

impl InteractionState {
    /// A neutral state for tests: the cursor at the virtual-desktop
    /// origin, nothing selected, nothing captured and the overlays on. Every overlay's per-host rule is tested by taking one
    /// of these and setting only the fields that rule reads, so the
    /// fixture lives beside the struct rather than being copied into each
    /// component.
    #[cfg(test)]
    pub fn new() -> Self {
        Self {
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
            cursor_overlay_visible: true,
            scroll_pick_mode: false,
            ocr: OcrState::Idle,
            ocr_notice: None,
            draw: DrawState::default(),
            marks: Marks::default(),
            modifiers: ModifiersState::empty(),
        }
    }
}

pub(crate) struct InteractionController;

impl InteractionController {
    pub fn finalize_selection(
        input: &mut InteractionState,
        rect: ScreenRect,
        corner_radius: f32,
        monitors: &[MonitorInfo],
    ) -> InteractionEffects {
        input.selection = Some(rect);
        input.selection_radius = corner_radius;
        input.mouse_down = false;
        input.mouse_down_pt = None;
        input.dragging = false;
        input.drag_mode = None;
        input.drag_anchor_selection = None;
        input.captured = true;
        input.overlays_visible = true;

        let dpi = dpi_at_point(input.virtual_cursor, monitors);
        let ht = hit_test(input.virtual_cursor, rect, dpi);
        input.hittest = ht;

        InteractionEffects {
            broadcast_mouse: true,
            broadcast_ui: true,
            update_cursor_visibility: true,
            set_cursor: Some(ht.cursor()),
        }
    }

    pub fn reset(input: &mut InteractionState) -> InteractionEffects {
        input.selection = None;
        input.selection_radius = 0.0;
        input.captured = false;
        // Pick mode only means anything while a captured selection exists
        // to click inside; dropping the selection must drop the mode too,
        // or the reticle/hidden-panel state would outlive its reason.
        input.scroll_pick_mode = false;
        // Same reasoning as pick mode, one step stronger: a recognition
        // result is a set of rects positioned against the selection that
        // produced it. Once that selection is gone the lifted lines have
        // nothing to sit on and the OCR button set has nothing to act on,
        // so the mode — and any notice it raised — goes with it.
        input.ocr = OcrState::Idle;
        input.ocr_notice = None;
        input.draw.reset();
        // The marks were drawn on the selection being dropped.
        input.marks.clear();
        input.hittest = Hittest::Outside;
        input.drag_mode = None;
        input.drag_anchor_selection = None;

        InteractionEffects {
            broadcast_ui: true,
            update_cursor_visibility: true,
            set_cursor: Some(CursorIcon::Default),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicksnap_shared::geometry::RectExt;

    fn monitor() -> MonitorInfo {
        MonitorInfo {
            bounds: ScreenRect::from_xy_size(0, 0, 200, 120),
            scale_factor: 1.0,
            is_primary: true,
            refresh_hz: 60.0,
            name: "test".to_string(),
            adapter_id: None,
            #[cfg(target_os = "macos")]
            logical_origin: flicksnap_shared::geometry::LogicalPoint::new(0.0, 0.0),
            #[cfg(target_os = "macos")]
            display_id: 0,
        }
    }

    fn state() -> InteractionState {
        InteractionState {
            virtual_cursor: ScreenPointF::new(30.0, 30.0),
            mouse_down: true,
            mouse_down_pt: Some(ScreenPointF::new(20.0, 20.0)),
            mouse_down_dpi: 1.0,
            dragging: true,
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
            cursor_overlay_visible: true,
            scroll_pick_mode: false,
            ocr: OcrState::Idle,
            ocr_notice: None,
            draw: DrawState::default(),
            marks: Marks::default(),
            modifiers: ModifiersState::empty(),
        }
    }

    /// A result-shaped payload for state assertions. Recognition itself is
    /// never exercised here — this module only cares that the mode field is
    /// carried and cleared correctly.
    fn dummy_outcome() -> Arc<OcrOutcome> {
        Arc::new(OcrOutcome {
            lines: Vec::new(),
            full_text: String::new(),
            text_angle: 0.0,
        })
    }

    #[test]
    fn finalize_selection_ends_the_drag_and_captures() {
        let mut input = state();
        let rect = ScreenRect::from_xy_size(20, 20, 40, 30);

        let effects = InteractionController::finalize_selection(&mut input, rect, 12.0, &[monitor()]);

        assert_eq!(input.selection, Some(rect));
        assert_eq!(input.selection_radius, 12.0);
        assert!(input.captured);
        assert!(!input.mouse_down);
        assert!(effects.broadcast_mouse);
        assert!(effects.broadcast_ui);
    }

    #[test]
    fn reset_clears_capture_and_selection() {
        let mut input = state();
        input.captured = true;
        input.selection = Some(ScreenRect::from_xy_size(1, 1, 10, 10));
        input.selection_radius = 16.0;
        input.scroll_pick_mode = true;
        input.ocr = OcrState::Lifted {
            anchor: Instant::now(),
            req: 1,
            region: ScreenRect::from_xy_size(1, 1, 10, 10),
            dpi_scale: 1.0,
            outcome: dummy_outcome(),
        };
        input.ocr_notice = Some(OcrNotice {
            anchor: Instant::now(),
            kind: OcrNoticeKind::NoText,
        });

        let effects = InteractionController::reset(&mut input);

        assert!(!input.captured);
        assert_eq!(input.selection, None);
        assert_eq!(input.selection_radius, 0.0);
        assert!(!input.scroll_pick_mode);
        // The lifted lines were positioned against the selection that just
        // disappeared; leaving the mode up would draw them over nothing.
        assert!(!input.ocr.active());
        assert!(input.ocr_notice.is_none());
        assert_eq!(input.hittest, Hittest::Outside);
        assert!(effects.broadcast_ui);
        assert!(effects.update_cursor_visibility);
    }

    #[test]
    fn ocr_state_gates_agree_on_each_phase() {
        let region = ScreenRect::from_xy_size(1, 1, 10, 10);

        assert!(!OcrState::Idle.active());
        assert!(!OcrState::Idle.shows_ocr_panel());
        assert!(!OcrState::Idle.hides_panel());

        // Scanning: modal, but NO panel of any kind — the strip used to
        // show here with COPY/SEARCH/UPLOAD lit-but-dead, which read as
        // broken buttons. The strip now appears only once there is text to
        // act on.
        let scanning = OcrState::Scanning {
            anchor: Instant::now(),
            req: 1,
            region,
        };
        assert!(scanning.active());
        assert!(!scanning.shows_ocr_panel());
        assert!(scanning.hides_panel());

        let lifted = OcrState::Lifted {
            anchor: Instant::now(),
            req: 1,
            region,
            dpi_scale: 1.0,
            outcome: dummy_outcome(),
        };
        assert!(lifted.active());
        assert!(lifted.shows_ocr_panel());
        assert!(!lifted.hides_panel());

        // Retracting is still modal (the color fade is playing, the
        // selection stays frozen) but hands the Normal buttons back at once.
        let retracting = OcrState::Retracting {
            anchor: Instant::now(),
        };
        assert!(retracting.active());
        assert!(!retracting.shows_ocr_panel());
        assert!(!retracting.hides_panel());
    }

    #[test]
    fn notice_alpha_is_solid_then_ramps_to_zero() {
        // t=0 and anywhere before the fade window: fully opaque.
        assert_eq!(notice_alpha(0.0), 1.0);
        assert_eq!(notice_alpha(NOTICE_SECS - NOTICE_FADE_SECS), 1.0);
        // Halfway through the fade window.
        assert!((notice_alpha(NOTICE_SECS - NOTICE_FADE_SECS / 2.0) - 0.5).abs() < 1e-4);
        // Just before expiry: nearly gone, but not yet zero.
        let nearly_done = notice_alpha(NOTICE_SECS - 0.001);
        assert!(nearly_done > 0.0 && nearly_done < 0.01, "got {nearly_done}");
        // Past expiry, including absurdly far past it (suspend/resume).
        assert_eq!(notice_alpha(NOTICE_SECS), 0.0);
        assert_eq!(notice_alpha(NOTICE_SECS + 10.0), 0.0);
        assert_eq!(notice_alpha(1.0e9), 0.0);
    }

    #[test]
    fn fresh_notice_is_visible_and_opaque() {
        let n = OcrNotice {
            anchor: Instant::now(),
            kind: OcrNoticeKind::Unavailable,
        };
        assert!(n.visible());
        assert_eq!(notice_alpha(n.anchor.elapsed().as_secs_f32()), 1.0);
        assert!(!n.kind.message().is_empty());
    }
}
