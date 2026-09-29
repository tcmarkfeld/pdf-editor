//! Layout engine: document model -> positioned pages.
//!
//! Parley does shaping (ligatures, kerning, fallback, bidi) and line
//! breaking; this crate owns everything vertical — the shared box model from
//! `document::DESCENT_RATIO`, block flow, columns, tables and pagination — so
//! imported pages reproduce source baselines exactly and edits reflow.
//!
//! Results are cached per section by `Arc` identity: an edit re-lays out only
//! the section it touched.

mod flow;
mod para;
mod query;

use std::ops::Range;
use std::sync::Arc;

use document::{Document, ImageResource, PathShape, Rect, Rgba, Section, Size};
use fonts::FontSystem;
use parley::{FontData, LayoutContext};

pub use query::Hit;

/// Brush payload: index of the paragraph run a glyph belongs to.
pub type RunIndex = u32;

#[derive(Clone, Debug)]
pub struct PositionedGlyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    /// Byte range within [`GlyphRun::text`].
    pub text: Range<u32>,
}

#[derive(Clone)]
pub struct GlyphRun {
    pub font: FontData,
    pub size: f32,
    pub color: Rgba,
    pub embolden: bool,
    pub skew: Option<f32>,
    pub glyphs: Vec<PositionedGlyph>,
    pub text: String,
    pub link: Option<Arc<str>>,
}

#[derive(Clone)]
pub enum Item {
    Glyphs(GlyphRun),
    Rect { rect: Rect, color: Rgba },
    Path(PathShape),
    Image { rect: Rect, image: Arc<ImageResource> },
}

#[derive(Clone)]
pub struct PageLayout {
    pub section: usize,
    pub size: Size,
    pub items: Vec<Item>,
    /// Page added because the section's content overflowed its source page.
    pub continuation: bool,
}

/// One shaped piece of a paragraph between tab characters.
pub struct Segment {
    pub layout: parley::Layout<RunIndex>,
    /// Byte range of this segment within the paragraph text.
    pub start: usize,
    pub end: usize,
}

/// A piece of a visual line: parley line `line` of segment `seg`, whose
/// layout coordinates map to page coordinates by adding `(dx, dy)`.
#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub seg: usize,
    pub line: usize,
    pub dx: f32,
    pub dy: f32,
}

#[derive(Clone, Debug)]
pub struct VisualLine {
    /// Section-local page index.
    pub page: usize,
    pub top: f32,
    pub bottom: f32,
    pub baseline: f32,
    pub x0: f32,
    pub x1: f32,
    pub parts: Vec<Part>,
    /// Paragraph byte range covered by the line.
    pub text: Range<usize>,
}

pub struct ParaLayout {
    pub segments: Vec<Segment>,
    pub lines: Vec<VisualLine>,
    /// Page x of the paragraph's left content edge / its available width.
    pub content_x: f32,
    pub width: f32,
    pub align: document::Align,
}

pub struct SectionLayout {
    pub pages: Vec<PageLayout>,
    /// In the section's paragraph reading order.
    pub paras: Vec<ParaLayout>,
}

/// Laid-out document. Cheap to rebuild from cached sections.
pub struct DocLayout {
    pub sections: Vec<Arc<SectionLayout>>,
    /// Global page -> (section, local page).
    pages: Vec<(usize, usize)>,
    /// Global paragraph -> (section, local paragraph).
    paras: Vec<(usize, usize)>,
    first_page: Vec<usize>,
}

impl DocLayout {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, i: usize) -> &PageLayout {
        let (s, p) = self.pages[i];
        &self.sections[s].pages[p]
    }

    pub fn para_count(&self) -> usize {
        self.paras.len()
    }

    pub fn para(&self, i: usize) -> &ParaLayout {
        let (s, p) = self.paras[i];
        &self.sections[s].paras[p]
    }

    /// Global page index of a paragraph line.
    pub fn line_page(&self, para: usize, line: &VisualLine) -> usize {
        self.first_page[self.paras[para].0] + line.page
    }

    pub fn first_page_of_section(&self, section: usize) -> usize {
        self.first_page[section]
    }
}

pub struct Layouter {
    pub fonts: FontSystem,
    lcx: LayoutContext<RunIndex>,
    cache: Vec<Option<(Arc<Section>, Arc<SectionLayout>)>>,
}

impl Default for Layouter {
    fn default() -> Self {
        Self::new()
    }
}

impl Layouter {
    pub fn new() -> Self {
        Self { fonts: FontSystem::new(), lcx: LayoutContext::new(), cache: Vec::new() }
    }

    pub fn layout(&mut self, doc: &Document) -> DocLayout {
        self.cache.resize_with(doc.sections.len(), || None);
        let mut out = DocLayout { sections: Vec::new(), pages: Vec::new(), paras: Vec::new(), first_page: Vec::new() };
        for (si, section) in doc.sections.iter().enumerate() {
            let cached = match &self.cache[si] {
                Some((s, l)) if Arc::ptr_eq(s, section) => l.clone(),
                _ => {
                    let l = Arc::new(flow::layout_section(section, si, &mut self.fonts, &mut self.lcx));
                    self.cache[si] = Some((section.clone(), l.clone()));
                    l
                }
            };
            out.first_page.push(out.pages.len());
            out.pages.extend((0..cached.pages.len()).map(|p| (si, p)));
            out.paras.extend((0..cached.paras.len()).map(|p| (si, p)));
            out.sections.push(cached);
        }
        out
    }
}
