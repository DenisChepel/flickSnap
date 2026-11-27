//! Drawing over a captured selection: the tool the user picked, the
//! colour it inks in ([`DrawState`]), the marks drawn ([`marks`]) and what
//! they look like ([`geometry`]).
//!
//! A deliberately small take on draw-on-screen (the shell's
//! `DrawOnScreen`): four of its tools, its eight colours, its thinnest
//! size and nothing else — no select tool, no sizes to choose, no hotkeys.
//! The state lives on [`crate::interaction::InteractionState`] beside the
//! selection it annotates, and a reset drops it with the selection.

pub mod burn;
pub mod geometry;
pub mod marks;

pub use marks::{Ink, Mark, Marks};

use egui::Color32;

/// One drawing tool, by draw-on-screen's name for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawTool {
    Brush,
    Arrow,
    Line,
    Rectangle,
}

impl DrawTool {
    /// Every tool, in draw-on-screen's toolbar order (`DrawPalette.Tools`
    /// with the ones this strip leaves out taken away).
    pub const ALL: [DrawTool; 4] = [DrawTool::Brush, DrawTool::Arrow, DrawTool::Line, DrawTool::Rectangle];

    /// The shape this tool drags out from one point to another — every
    /// tool's but the freehand brush's.
    pub const fn shape(self) -> Option<Shape> {
        match self {
            DrawTool::Brush => None,
            DrawTool::Arrow => Some(Shape::Arrow),
            DrawTool::Line => Some(Shape::Line),
            DrawTool::Rectangle => Some(Shape::Rectangle),
        }
    }
}

/// A mark dragged out from one point to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Arrow,
    Line,
    Rectangle,
}

/// Draw-on-screen's eight colours (`DrawPalette.Colors`), in its order:
/// red, orange, yellow, green, blue, purple, white, black. Mirrored by
/// hand — keep the two lists identical.
pub const PALETTE: [Color32; 8] = [
    Color32::from_rgb(0xDF, 0x28, 0x28),
    Color32::from_rgb(0xE2, 0x7F, 0x38),
    Color32::from_rgb(0xE2, 0xCF, 0x38),
    Color32::from_rgb(0x86, 0xE2, 0x38),
    Color32::from_rgb(0x35, 0x9E, 0xC9),
    Color32::from_rgb(0x78, 0x55, 0xD4),
    Color32::from_rgb(0xF4, 0xF4, 0xF4),
    Color32::from_rgb(0x1E, 0x1E, 0x1E),
];

/// The colours' names, index for index with [`PALETTE`], as the hover
/// chips say them (draw-on-screen's `Draw_Color_*` strings).
pub const PALETTE_NAMES: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Blue", "Purple", "White", "Black"];

/// What the user has picked on the drawing strip.
///
/// The default is the one every capture starts from: no tool, so the
/// selection still moves and resizes exactly as it always has until a
/// tool is picked, and red, draw-on-screen's default colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrawState {
    /// The tool a drag on the overlay draws with, if any.
    pub tool: Option<DrawTool>,
    /// Index into [`PALETTE`]. Always in range.
    pub color: usize,
    /// The colour popup is open beside the strip.
    pub palette_open: bool,
}

impl DrawState {
    /// Pick `tool`, or put it down when it is the one already picked —
    /// which is how the user gets the move-and-resize gestures back.
    pub fn toggle_tool(&mut self, tool: DrawTool) {
        self.tool = if self.tool == Some(tool) { None } else { Some(tool) };
    }

    /// Open the colour popup, or close it when it is open.
    pub fn toggle_palette(&mut self) {
        self.palette_open = !self.palette_open;
    }

    /// Shut the colour popup. Returns whether it was open, so the caller
    /// knows whether there is a change to show.
    pub fn close_palette(&mut self) -> bool {
        std::mem::replace(&mut self.palette_open, false)
    }

    /// Ink in `PALETTE[index]` from now on and close the popup it was
    /// picked from. An index past the palette is ignored.
    pub fn pick_color(&mut self, index: usize) {
        if index < PALETTE.len() {
            self.color = index;
        }
        self.palette_open = false;
    }

    /// What a RESET leaves: the tool put down — the next selection moves
    /// and resizes as a fresh one does — and the popup shut. The colour is
    /// the user's preference, not the selection's, so it stays.
    pub fn reset(&mut self) {
        *self = Self {
            color: self.color,
            ..Self::default()
        };
    }

    /// The colour the next mark is inked in.
    pub fn ink(&self) -> Color32 {
        PALETTE[self.color]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_starts_with_no_tool_in_red_and_the_popup_shut() {
        let s = DrawState::default();
        assert_eq!(s.tool, None);
        assert_eq!(s.ink(), Color32::from_rgb(0xDF, 0x28, 0x28));
        assert!(!s.palette_open);
    }

    #[test]
    fn picking_the_picked_tool_puts_it_down() {
        let mut s = DrawState::default();
        s.toggle_tool(DrawTool::Arrow);
        assert_eq!(s.tool, Some(DrawTool::Arrow));
        s.toggle_tool(DrawTool::Line);
        assert_eq!(s.tool, Some(DrawTool::Line));
        s.toggle_tool(DrawTool::Line);
        assert_eq!(s.tool, None);
    }

    #[test]
    fn a_reset_puts_the_tool_down_and_shuts_the_popup_but_keeps_the_colour() {
        let mut s = DrawState::default();
        s.toggle_tool(DrawTool::Rectangle);
        s.pick_color(3);
        s.toggle_palette();
        s.reset();
        assert_eq!(
            s,
            DrawState {
                color: 3,
                ..DrawState::default()
            }
        );
    }

    #[test]
    fn picking_a_colour_closes_the_popup_and_ignores_an_index_past_the_palette() {
        let mut s = DrawState::default();
        s.toggle_palette();
        assert!(s.palette_open);
        s.pick_color(4);
        assert_eq!((s.color, s.palette_open), (4, false));

        s.toggle_palette();
        s.pick_color(PALETTE.len());
        assert_eq!((s.color, s.palette_open), (4, false));
    }
}
