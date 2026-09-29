//! The editable document tree.
//!
//! The model is deliberately "Word-like": lists and headings are paragraph
//! properties rather than containers, and right-aligned resume dates are tab
//! stops. Containers exist only where independent flows are unavoidable
//! (columns, table cells, absolutely positioned frames).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::geom::{Point, Rect, Rgba, Size};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Document {
    /// One section per source page. Sections never merge or split during
    /// editing, so section indices are stable identifiers.
    pub sections: Vec<Arc<Section>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Section {
    pub page_size: Size,
    pub margins: Margins,
    /// Flow content in reading order.
    pub blocks: Vec<Block>,
    /// Non-editable page-anchored graphics drawn beneath the flow
    /// (backgrounds, sidebars, logos that could not be placed in flow).
    pub decorations: Vec<Decoration>,
    pub source_page: Option<u32>,
    /// True while the page is still being reconstructed in the background.
    #[serde(default)]
    pub pending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Margins {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Block {
    Paragraph(Paragraph),
    Columns(Columns),
    Table(Table),
    Image(ImageBlock),
    Rule(Rule),
    /// Out-of-flow content at a fixed page position (the escape hatch for
    /// content whose reconstruction evidence is weak). Still editable.
    Frame(Frame),
}

impl Block {
    /// Child flows of container blocks, in reading order.
    pub fn children(&self) -> Vec<&Vec<Block>> {
        match self {
            Block::Columns(c) => c.columns.iter().map(|c| &c.blocks).collect(),
            Block::Table(t) => t.rows.iter().flat_map(|r| r.cells.iter().map(|c| &c.blocks)).collect(),
            Block::Frame(f) => vec![&f.blocks],
            _ => Vec::new(),
        }
    }

    pub fn child_mut(&mut self, i: usize) -> Option<&mut Vec<Block>> {
        match self {
            Block::Columns(c) => c.columns.get_mut(i).map(|c| &mut c.blocks),
            Block::Table(t) => {
                let cols = t.col_widths.len().max(1);
                t.rows.get_mut(i / cols).and_then(|r| r.cells.get_mut(i % cols)).map(|c| &mut c.blocks)
            }
            Block::Frame(f) if i == 0 => Some(&mut f.blocks),
            _ => None,
        }
    }

    pub fn space_before(&self) -> f32 {
        match self {
            Block::Paragraph(p) => p.style.space_before,
            Block::Columns(c) => c.space_before,
            Block::Table(t) => t.space_before,
            Block::Image(i) => i.space_before,
            Block::Rule(r) => r.space_before,
            Block::Frame(_) => 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Paragraph {
    /// Styled runs. Invariant: never empty (an empty paragraph keeps one
    /// empty run so it still carries a style); adjacent runs differ in style.
    pub runs: Vec<TextRun>,
    pub style: ParagraphStyle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    pub style: TextStyle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub font: FontSpec,
    pub size: f32,
    pub color: Rgba,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub strike: bool,
    #[serde(default)]
    pub link: Option<Arc<str>>,
    /// Positive raises text (superscript), in points.
    #[serde(default)]
    pub baseline_shift: f32,
    /// Extra advance per character, in points.
    #[serde(default)]
    pub letter_spacing: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: FontSpec::default(),
            size: 11.0,
            color: Rgba::BLACK,
            underline: false,
            strike: false,
            link: None,
            baseline_shift: 0.0,
            letter_spacing: 0.0,
        }
    }
}

/// A font request as recovered from the source document. Resolution to an
/// installed font happens in the `fonts` crate at layout time.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FontSpec {
    /// Clean family name, e.g. "Times New Roman".
    pub family: Arc<str>,
    /// Original PDF BaseFont name, e.g. "ABCDEF+TimesNewRomanPS-BoldMT".
    #[serde(default)]
    pub source_name: Option<Arc<str>>,
    pub weight: u16,
    pub italic: bool,
    pub generic: GenericFamily,
}

impl Default for FontSpec {
    fn default() -> Self {
        Self {
            family: Arc::from("Helvetica"),
            source_name: None,
            weight: 400,
            italic: false,
            generic: GenericFamily::SansSerif,
        }
    }
}

impl FontSpec {
    pub fn is_bold(&self) -> bool {
        self.weight >= 600
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GenericFamily {
    Serif,
    #[default]
    SansSerif,
    Monospace,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParagraphStyle {
    pub align: Align,
    /// Offsets from the containing flow's content box.
    pub indent_left: f32,
    pub indent_right: f32,
    /// Added to `indent_left` for the first line (negative = hanging).
    pub first_line_indent: f32,
    /// Gap between the previous block's bottom and this paragraph's top.
    pub space_before: f32,
    pub line_spacing: LineSpacing,
    #[serde(default)]
    pub tab_stops: Vec<TabStop>,
    #[serde(default)]
    pub list: Option<ListInfo>,
    #[serde(default)]
    pub role: Role,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            align: Align::Left,
            indent_left: 0.0,
            indent_right: 0.0,
            first_line_indent: 0.0,
            space_before: 0.0,
            line_spacing: LineSpacing::Multiple(1.2),
            tab_stops: Vec::new(),
            list: None,
            role: Role::Body,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// Vertical box model shared by reconstruction and layout.
///
/// A line with pitch `P` (from [`LineSpacing`]) whose largest font size is
/// `S` occupies `P` points: its baseline sits `DESCENT_RATIO * S` above the
/// line box bottom. Paragraph boxes are the union of their line boxes and
/// `space_before` separates consecutive block boxes. Reconstruction measures
/// `space_before` with the same rule, so imported pages lay out with every
/// baseline exactly where the PDF had it — independent of font metrics.
pub const DESCENT_RATIO: f32 = 0.2;

/// Baseline-to-baseline distance between consecutive lines.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum LineSpacing {
    /// Exact distance in points (what reconstruction measures).
    Exact(f32),
    /// Multiple of the line's largest font size.
    Multiple(f32),
}

impl LineSpacing {
    pub fn pitch(&self, font_size: f32) -> f32 {
        match *self {
            LineSpacing::Exact(v) => v,
            LineSpacing::Multiple(m) => m * font_size,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabStop {
    /// Position relative to the paragraph's left content edge.
    pub pos: f32,
    pub align: TabAlign,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListInfo {
    /// Consecutive paragraphs with the same id and level number together.
    pub id: u32,
    pub level: u8,
    pub kind: ListKind,
    /// Marker x-position relative to the paragraph's left content edge
    /// (typically negative: the marker hangs in the indent).
    pub marker_offset: f32,
    pub marker_style: TextStyle,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ListKind {
    Bullet(String),
    Ordered { start: u32, format: NumberFormat, suffix: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberFormat {
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

impl NumberFormat {
    pub fn format(&self, n: u32) -> String {
        fn alpha(mut n: u32, base: u8) -> String {
            let mut s = Vec::new();
            while n > 0 {
                n -= 1;
                s.push(base + (n % 26) as u8);
                n /= 26;
            }
            s.reverse();
            String::from_utf8(s).unwrap_or_default()
        }
        fn roman(mut n: u32) -> String {
            const T: [(u32, &str); 13] = [
                (1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"),
                (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i"),
            ];
            let mut s = String::new();
            for (v, r) in T {
                while n >= v {
                    s.push_str(r);
                    n -= v;
                }
            }
            s
        }
        match self {
            NumberFormat::Decimal => n.to_string(),
            NumberFormat::LowerAlpha => alpha(n, b'a'),
            NumberFormat::UpperAlpha => alpha(n, b'A'),
            NumberFormat::LowerRoman => roman(n),
            NumberFormat::UpperRoman => roman(n).to_uppercase(),
        }
    }
}

/// Structural role derived from typography. Only affects editing behaviour
/// (e.g. the paragraph created after a heading), never appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[default]
    Body,
    Heading(u8),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Columns {
    pub space_before: f32,
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Column {
    /// Relative to the containing flow's left content edge.
    pub x: f32,
    pub width: f32,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub space_before: f32,
    /// Offset of the table from the flow's left content edge.
    pub x: f32,
    pub col_widths: Vec<f32>,
    pub rows: Vec<TableRow>,
    pub borders: Option<Border>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableRow {
    pub min_height: f32,
    pub cells: Vec<TableCell>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TableCell {
    pub blocks: Vec<Block>,
    /// Background fill.
    #[serde(default)]
    pub shading: Option<Rgba>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Border {
    pub width: f32,
    pub color: Rgba,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageBlock {
    pub space_before: f32,
    pub x: f32,
    pub width: f32,
    pub height: f32,
    pub image: Arc<ImageResource>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub space_before: f32,
    pub x: f32,
    /// Zero or negative: extend to the flow's right edge.
    pub width: f32,
    pub thickness: f32,
    pub color: Rgba,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    /// Page coordinates.
    pub rect: Rect,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Decoration {
    Path(PathShape),
    Image { rect: Rect, image: Arc<ImageResource> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathShape {
    pub segments: Vec<PathSeg>,
    pub fill: Option<Rgba>,
    pub stroke: Option<(Rgba, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PathSeg {
    MoveTo(Point),
    LineTo(Point),
    CurveTo(Point, Point, Point),
    Close,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageResource {
    /// Content hash; stable identity for texture/export caches.
    pub id: u64,
    pub width_px: u32,
    pub height_px: u32,
    pub format: ImageFormat,
    #[serde(with = "crate::b64")]
    pub bytes: Vec<u8>,
}

impl ImageResource {
    /// Wraps PNG bytes, deriving the content-hash id.
    pub fn png(bytes: Vec<u8>, width_px: u32, height_px: u32) -> Self {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        bytes.hash(&mut h);
        Self { id: h.finish(), width_px, height_px, format: ImageFormat::Png, bytes }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
}
