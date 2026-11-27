//! Static content for the Tips & Hotkeys panel.
//!
//! The rows are static apart from the name of the monitor under the cursor.
//! Layout math reads this table to measure the longest row and size the
//! panel accordingly.

/// The accent-colored title bar text.
pub const TITLE: &str = "Tips & Hotkeys";

/// A single body row: a one-character hotkey label followed by the
/// description text, each in its own column (`DxScreenCapture.cpp:746-752`).
pub struct TipRow {
    pub hotkey: &'static str,
    /// `{monitor}` is substituted with the name of the monitor under the cursor.
    pub description_template: &'static str,
}

impl TipRow {
    const fn new(hotkey: &'static str, description_template: &'static str) -> Self {
        Self {
            hotkey,
            description_template,
        }
    }
}

/// The panel's rows, top to bottom.
pub const TIPS: &[TipRow] = &[
    TipRow::new("F", "Select monitor {monitor}"),
    TipRow::new("A", "Select all monitors"),
    TipRow::new("M", "Toggle cursor in screenshot"),
    TipRow::new("X", "Toggle magnifier"),
    TipRow::new("D", "Toggle debug stats"),
    TipRow::new("T", "Show or hide this panel"),
    TipRow::new("Q", "Hide or show the overlay"),
];

/// Fallback when no monitor is under the cursor.
pub const FALLBACK: &str = "n/a";

/// Max characters (inclusive) before the monitor name is truncated.
pub(crate) const MONITOR_NAME_MAX: usize = 28;

pub(crate) fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
    let mut iter = s.chars();
    let head: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        let mut out: String = head
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect();
        out.push('…');
        out
    } else {
        head
    }
}

/// Render the body of a single tip row (description only, without hotkey
/// or gap) with runtime substitutions applied.
pub fn render_description(template: &str, hovered_monitor: Option<&str>) -> String {
    let monitor = hovered_monitor.map(|s| truncate_ellipsis(s, MONITOR_NAME_MAX));
    template.replace("{monitor}", monitor.as_deref().unwrap_or(FALLBACK))
}
