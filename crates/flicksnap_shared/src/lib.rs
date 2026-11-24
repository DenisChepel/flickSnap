//! Pure logic shared by FlickSnap's Rust binaries (the capture overlay
//! `flicksnap_capture`, the scrolling-capture driver `flicksnap_scroll_driver`, the AI
//! inference binary `flicksnap_ai` and the recorder stack) and by FlickSnap.Ui and
//! FlickSnap.VideoRender through `flicksnap_interop`.
//!
//! The bar for living here is that **two processes must agree**, so a change
//! made in one place and not the other would be a bug:
//!
//! - [`geometry`] — the virtual-desktop coordinate space every capture
//!   coordinate, CLI flag and Win32 call is expressed in.
//! - [`session`] — the `session.json` contract, which `FlickSnap.Ui` reads
//!   (`SessionInfo`) and both binaries write.
//! - [`ocr`] — the recognition request/response contract the overlay and
//!   `flicksnap_ai ocr` speak across their process boundary.
//! - [`color`], [`share_region`], [`encoding`]: pure logic that a Rust
//!   helper and FlickSnap.Ui (through flicksnap_interop) must compute identically.
//! - [`contract`]: typed constant tables both sides read; flicksnap_codegen
//!   renders them into C# (`cargo codegen`).
//!
//! Anything that only one binary needs stays in that binary. What the binaries
//! share but that reaches the OS or Sentry has a crate of its own: `flicksnap_desktop`
//! and `flicksnap_telemetry`.

pub mod color;
pub mod contract;
pub mod encoding;
pub mod geometry;
pub mod ocr;
pub mod session;
pub mod share_region;
