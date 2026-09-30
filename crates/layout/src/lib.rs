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

use std::collections::HashMap;
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
    /// Leading items that are page decorations (drawn beneath everything).
    pub decorations: usize,
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
    pub objects: Vec<ObjectGeom>,
    pub tables: Vec<TableGeom>,
    /// Where the flow ended: (local page, y). A local page above 0 means the
    /// content overflowed its source page.
    pub end: (usize, f32),
}

/// Where an in-flow image block landed (for selecting and resizing it).
#[derive(Clone, Debug)]
pub struct ObjectGeom {
    /// Section-local page.
    pub page: usize,
    pub rect: Rect,
    /// Block path within the section.
    pub path: Vec<u32>,
}

/// A laid-out table's grid (for column resizing).
#[derive(Clone, Debug)]
pub struct TableGeom {
    pub path: Vec<u32>,
    /// Column boundaries (page x), `col_widths.len() + 1` of them.
    pub cols: Vec<f32>,
    /// (section-local page, top, bottom) per row.
    pub rows: Vec<(usize, f32, f32)>,
}

/// Laid-out document. Cheap to rebuild from cached sections.
///
/// A section that overflows its page continues onto the next section's
/// first page (which then starts below the carried-over content), so a
/// global page can show content from two sections.
pub struct DocLayout {
    pub sections: Vec<Arc<SectionLayout>>,
    /// Sources of each global page: (section, local page).
    pages: Vec<Vec<(usize, usize)>>,
    /// Combined items for pages with more than one source.
    merged: HashMap<usize, PageLayout>,
    /// Global paragraph -> (section, local paragraph).
    paras: Vec<(usize, usize)>,
    /// Section -> local page -> global page.
    page_map: Vec<Vec<usize>>,
}

impl DocLayout {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, i: usize) -> &PageLayout {
        if let Some(m) = self.merged.get(&i) {
            return m;
        }
        let (s, p) = self.pages[i][0];
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
        self.page_map[self.paras[para].0][line.page]
    }

    pub fn first_page_of_section(&self, section: usize) -> usize {
        self.page_map[section][0]
    }

    /// (section, local page) sources shown on a global page.
    pub fn page_sources(&self, page: usize) -> &[(usize, usize)] {
        &self.pages[page]
    }

    /// The image under a point, as (section, geometry).
    pub fn object_at(&self, page: usize, x: f32, y: f32) -> Option<(usize, &ObjectGeom)> {
        let p = document::Point::new(x, y);
        self.pages[page]
            .iter()
            .find_map(|&(si, lp)| self.sections[si].objects.iter().find(|o| o.page == lp && o.rect.contains(p)).map(|o| (si, o)))
    }

    /// Global page and rect of the image block at `path`.
    pub fn object(&self, section: usize, path: &[u32]) -> Option<(usize, Rect)> {
        let o = self.sections.get(section)?.objects.iter().find(|o| o.path == path)?;
        Some((self.page_map[section][o.page], o.rect))
    }

    /// Tables with rows on a global page, as (section, geometry, local page).
    pub fn tables_on(&self, page: usize) -> impl Iterator<Item = (usize, &TableGeom, usize)> {
        self.pages[page].iter().flat_map(move |&(si, lp)| {
            self.sections[si].tables.iter().filter(move |t| t.rows.iter().any(|r| r.0 == lp)).map(move |t| (si, t, lp))
        })
    }
}

pub struct Layouter {
    pub fonts: FontSystem,
    lcx: LayoutContext<RunIndex>,
    /// Per section: the laid-out section, keyed by its content and the y
    /// its content started at (after carried-over overflow).
    cache: Vec<Option<CacheEntry>>,
}

type CacheEntry = (Arc<Section>, Option<u32>, Arc<SectionLayout>);

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
        let mut out = DocLayout { sections: Vec::new(), pages: Vec::new(), merged: HashMap::new(), paras: Vec::new(), page_map: Vec::new() };
        // Overflow carried from the previous section: (global page, y).
        let mut carry: Option<(usize, f32)> = None;
        for (si, section) in doc.sections.iter().enumerate() {
            let start = carry.filter(|&(g, _)| out.page(g).size == section.page_size).map(|(_, y)| y);
            let key = start.map(f32::to_bits);
            let laid = match &self.cache[si] {
                Some((s, k, l)) if Arc::ptr_eq(s, section) && *k == key => l.clone(),
                _ => {
                    let l = Arc::new(flow::layout_section(section, si, start, &mut self.fonts, &mut self.lcx));
                    self.cache[si] = Some((section.clone(), key, l.clone()));
                    l
                }
            };
            let mut map = Vec::with_capacity(laid.pages.len());
            for lp in 0..laid.pages.len() {
                match (lp, start, carry) {
                    (0, Some(_), Some((g, _))) => {
                        // Share the page the previous section overflowed onto:
                        // this page's backgrounds, then the carried content,
                        // then this section's own content.
                        let own = &laid.pages[0];
                        let mut page = own.clone();
                        page.items.truncate(own.decorations);
                        page.items.extend(out.page(g).items.iter().cloned());
                        page.items.extend(own.items[own.decorations..].iter().cloned());
                        out.pages[g].push((si, 0));
                        out.merged.insert(g, page);
                        map.push(g);
                    }
                    _ => {
                        map.push(out.pages.len());
                        out.pages.push(vec![(si, lp)]);
                    }
                }
            }
            carry = (laid.end.0 > 0).then(|| (map[laid.end.0], laid.end.1));
            out.page_map.push(map);
            out.paras.extend((0..laid.paras.len()).map(|p| (si, p)));
            out.sections.push(laid);
        }
        out
    }
}
