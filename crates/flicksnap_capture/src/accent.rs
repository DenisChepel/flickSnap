//! The accent colour maths the overlay paints with: `flicksnap_shared::color`, the one
//! implementation FlickSnap.Ui's `AccentColors` and `AccentTheme` also call (through flicksnap_interop), so
//! the overlay and the C# windows beside it derive the same colours from the same pick.
//!
//! The shell passes the accent exactly as the user chose it (`--accent-color`); the panel's fill is
//! [`ensure_contrast_with_white`] of it unless `--no-accent-contrast` is passed, and the hint comet
//! is [`comet_colors`] of the pick itself.

pub use flicksnap_shared::color::*;
