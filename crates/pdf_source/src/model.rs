//! Plain-data description of what a PDF page contains. Everything the
//! reconstruction layer needs, nothing PDFium-specific, so tests can build
//! pages synthetically with exactly known geometry.

use document::{PathSeg, Rect, Rgba};

#[derive(Clone, Debug, Default)]
pub struct SourcePage {
    pub index: u32,
    pub width: f32,
    pub height: f32,
    pub glyphs: Vec<SourceGlyph>,
    pub fonts: Vec<SourceFont>,
    pub images: Vec<SourceImage>,
    pub paths: Vec<SourcePath>,
    pub links: Vec<SourceLink>,
}

pub type FontId = u16;

#[derive(Clone, Debug)]
pub struct SourceGlyph {
    pub ch: char,
    /// Ink bounds.
    pub bbox: Rect,
    /// Advance-width x-extent with font ascent/descent y-extent. Use this for
    /// spacing decisions: its width is the glyph's advance.
    pub loose: Rect,
    pub origin_x: f32,
    pub baseline: f32,
    pub font: FontId,
    /// Effective size in points after the text matrix.
    pub size: f32,
    pub color: Rgba,
    /// Rotation of the text baseline in degrees (0 = horizontal).
    pub angle: f32,
    /// Inserted by PDFium's text extraction rather than drawn by the PDF.
    pub generated: bool,
    /// Rendered with a stroke as well as a fill (a common fake-bold trick).
    pub stroked: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceFont {
    /// BaseFont name as written in the PDF, subset prefix included.
    pub name: String,
    /// Family recovered from the name (see [`crate::fontname`]).
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    pub serif: bool,
    pub fixed_pitch: bool,
    pub symbolic: bool,
}

#[derive(Clone, Debug)]
pub struct SourceImage {
    pub rect: Rect,
    pub width_px: u32,
    pub height_px: u32,
    /// PNG-encoded pixels (masks already applied).
    pub png: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SourcePath {
    pub bounds: Rect,
    pub segments: Vec<PathSeg>,
    pub fill: Option<Rgba>,
    pub stroke: Option<(Rgba, f32)>,
}

#[derive(Clone, Debug)]
pub struct SourceLink {
    pub rect: Rect,
    pub uri: String,
}
