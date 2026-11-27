//! The button panel shown once a selection is captured: one floating tray
//! (the design shared with the C# recording / share-region / scrolling
//! strips) holding the FlickSnap emblem and the action
//! buttons in their groups (the accent group first, then the grey ones),
//! anchored beneath or beside the selection.
//!
//! One chassis for every set: the emblem, the set's body (the OCR word
//! count, the scroll-picker's instruction, or nothing) and the buttons in
//! one run along the chosen axis. The capture strip is one flat toolbar
//! of icon-only tiles straight on the chassis — the optional hand-offs,
//! then the core actions, then a hairline rule and the two ways out —
//! with each button's phrase and accelerator in its hover chip. Stood on
//! its end it is the same run top to bottom, the rule lying flat.
//! `--panel-buttons` picks the presentation for every other strip.
//!
//! The tray lives in [`place`] (which side of the selection, pure and in
//! physical pixels), [`theme`] (the design tokens as egui `Style`, fonts
//! and frames), [`assets`] (the SVG marks and the sources egui's image
//! loader rasterises them from), [`widgets`] (the two button styles, the readout, the emblem) and
//! [`show`] (the analytic strip size, the placement and the `Area`). It is
//! run by `ui::egui_host` on the app thread and shipped to the workers as
//! tessellated primitives, so the run that answers a click is the run that
//! drew the strip.
//!
//!   * [`model`]: the static truth. The two button sets ([`model::PanelButtonSet`]:
//!     `Normal` for a fresh capture, `Ocr` while lifted text is on screen),
//!     the per-button command / label / accelerator / icon, the two
//!     button styles ([`model::ButtonStyle`]), the shell's opt-out switches
//!     ([`model::PanelFeatures`]), and the accelerator lookup. No geometry.
//!
//! Each strip is placed with its own width, its row ending at the
//! selection's right edge, so a set swap moves the buttons under the
//! cursor; the re-click hazard that creates is `PanelSwapGuard`'s job
//! (app.rs), not this module's.

pub mod assets;
pub mod model;
pub mod place;
pub mod show;
pub mod theme;
pub mod widgets;

pub use model::lookup_command_by_key;
