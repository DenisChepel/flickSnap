//! The drawing strip shown beside a captured selection: draw-on-screen's
//! toolbar cut down to BRUSH, ARROW, LINE, RECTANGLE, the colour and UNDO,
//! on the action tray's chassis.
//!
//! It rides with the capture strip — up exactly while the `Normal` set
//! is, on the same host — and reports through the same `PanelOutcome`,
//! so its clicks, its dead chassis and its cursor route exactly as the
//! tray's do. [`place`] puts it beside the selection's right edge and off
//! the tray (pure, physical px); [`show`] lays it out, with the colour
//! popup when that is open. What the user picked lives in
//! [`crate::draw::DrawState`].

pub mod place;
pub mod show;
