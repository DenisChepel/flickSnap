//! The panel's SVG marks, as sources for egui's image loader.
//!
//! Every button mark fills its icon cell the same way: its viewBox is the
//! square around the shape's own bounds, centred — how the C# strips'
//! `TrayGeometryIcon` fits a geometry. A mark drawn with a margin of its
//! own inside a 24-unit canvas would come out smaller than its neighbours
//! by however much margin its artist left, so a new mark needs its viewBox
//! fitted the same way before it goes in.
//!
//! One deliberate exception: EXIT's cross (`delete.svg`) fills 85 % of the
//! cell, its viewBox grown about the same centre. A thin diagonal cross
//! stretched corner to corner reads bigger than the solid marks beside
//! it, the way draw-on-screen draws its own close cross a size down —
//! but a size down there was too small here.
//!
//! Each `include_bytes!` points at a file in `assets/icons/`. The bytes go
//! to the `egui_extras` SVG loader as an [`egui::ImageSource`], which
//! rasterises them synchronously at the pixel size the widget asks for and
//! caches them per context under their uri — so two button defs sharing a
//! mark share one texture on a host, and every host keeps its own set at
//! its own DPI. Fonts live in `ui::fonts`.

use std::borrow::Cow;

use egui::load::Bytes;
use egui::ImageSource;

use super::theme::tokens;

/// One embedded mark: the uri egui caches it under, and its bytes.
///
/// The uri must end in `.svg`: that extension is what routes the bytes to
/// the SVG loader instead of being refused as an unsupported format.
pub struct Svg {
    pub uri: &'static str,
    pub bytes: &'static [u8],
}

impl Svg {
    /// A source for this mark. Cheap — the uri and the bytes are static,
    /// and the loader dedupes by uri.
    pub fn source(&self) -> ImageSource<'static> {
        ImageSource::Bytes {
            uri: Cow::Borrowed(self.uri),
            bytes: Bytes::Static(self.bytes),
        }
    }
}

/// The uri alone: the bytes are a wall of numbers no failure message wants.
impl std::fmt::Debug for Svg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Svg")
            .field(&self.uri)
            .finish()
    }
}

macro_rules! svg {
    ($name:literal) => {
        Svg {
            uri: concat!("bytes://flicksnap/", $name),
            bytes: include_bytes!(concat!("../../../../assets/icons/", $name)),
        }
    };
}

/// File-upload mark for the UPLOAD button (white, viewBox fitted to the ink),
/// matching `assets/icons/screenshot/ico_upload.svg` and the Avalonia `IconUpload`.
pub const UPLOAD: Svg = svg!("upload.svg");
pub const EDIT: Svg = svg!("edit_image.svg");
pub const VIDEO: Svg = svg!("video_camera.svg");
pub const SHARE: Svg = svg!("share.svg");
pub const SCROLL: Svg = svg!("scroll.svg");
pub const OCR: Svg = svg!("ocr.svg");
pub const COPY: Svg = svg!("copy_to_clipboard.svg");
pub const SAVE: Svg = svg!("save.svg");
pub const RESET: Svg = svg!("refresh.svg");
pub const EXIT: Svg = svg!("delete.svg");
pub const SEARCH: Svg = svg!("search.svg");
/// Reverse image search in the capture strip.
pub const IMAGE_SEARCH: Svg = svg!("image_search.svg");
pub const BACK: Svg = svg!("back.svg");

/// The drawing strip's marks: the draw-on-screen toolbar's own glyphs
/// (its pencil, `IconToolLine`, `IconToolArrow` and `IconUndo` from
/// `VectorIcons.axaml`), fitted like every other mark here, so they fill
/// the icon cell exactly as they do on that toolbar. The rectangle is the
/// one exception: that toolbar's mark has rounded corners while the
/// rectangle it draws has square ones, so this one is `IconToolRectangleRounded`'s
/// 24 × 20 outline, two units thick, with its corners squared — the
/// shape the tool actually draws.
pub const DRAW_BRUSH: Svg = svg!("draw_brush.svg");
pub const DRAW_LINE: Svg = svg!("draw_line.svg");
pub const DRAW_ARROW: Svg = svg!("draw_arrow.svg");
pub const DRAW_RECTANGLE: Svg = svg!("draw_rectangle.svg");
pub const DRAW_UNDO: Svg = svg!("draw_undo.svg");

/// The FlickSnap logo drawn as the tray emblem at the head of the strip
/// (16-unit canvas, brand blue baked in). NOT a button icon: it has no
/// button, no hover and no click, and is drawn at its own (larger) size.
pub const FLICKSNAP_LOGO: Svg = svg!("flicksnap-logo.svg");

/// Every mark the tray can draw, button icons and the emblem alike.
pub const ALL: &[&Svg] = &[
    &UPLOAD,
    &EDIT,
    &VIDEO,
    &SHARE,
    &SCROLL,
    &OCR,
    &COPY,
    &SAVE,
    &RESET,
    &EXIT,
    &SEARCH,
    &IMAGE_SEARCH,
    &BACK,
    &DRAW_BRUSH,
    &DRAW_LINE,
    &DRAW_ARROW,
    &DRAW_RECTANGLE,
    &DRAW_UNDO,
    &FLICKSNAP_LOGO,
];

/// Load every mark at the tray's icon size once per pass — a cache hit
/// after the first — so a broken asset is logged instead of painted as
/// egui's "⚠" placeholder, which Inter has no glyph for. Logged once
/// per process: a mark that fails once fails every pass.
pub fn preload(ctx: &egui::Context) {
    static LOGGED: std::sync::Once = std::sync::Once::new();
    for svg in ALL {
        if let Err(err) = egui::Image::new(svg.source())
            .fit_to_exact_size(tokens::ICON_SIZE)
            .load_for_size(ctx, tokens::ICON_SIZE)
        {
            LOGGED.call_once(|| log::error!("panel icon {} failed to load: {err}", svg.uri));
        }
    }
}
