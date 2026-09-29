//! Stage 0: normalise raw source glyphs into the working representation.

use std::sync::Arc;

use document::{FontSpec, GenericFamily, Rect, Rgba, TextStyle};
use pdf_source::{SourceFont, SourceGlyph, SourcePage};

/// A drawn glyph in working form. `x0..x1` is the advance extent.
#[derive(Clone, Debug)]
pub struct G {
    pub ch: char,
    pub x0: f32,
    pub x1: f32,
    pub baseline: f32,
    pub size: f32,
    pub font: usize,
    pub color: Rgba,
    /// Index into `SourcePage::links`.
    pub link: Option<usize>,
    pub underline: bool,
    pub fake_bold: bool,
    /// A real space glyph preceded this glyph in the content.
    pub space_before: bool,
    /// Baseline shift in points (positive raises), for synthesized glyphs.
    pub shift: f32,
}

pub struct Prepared {
    pub glyphs: Vec<G>,
    /// Ink boxes of rotated glyphs (not representable in flow).
    pub rotated: Vec<Rect>,
}

fn expand_ligature(ch: char) -> Option<&'static str> {
    Some(match ch {
        'ﬀ' => "ff",
        'ﬁ' => "fi",
        'ﬂ' => "fl",
        'ﬃ' => "ffi",
        'ﬄ' => "ffl",
        'ﬅ' | 'ﬆ' => "st",
        _ => return None,
    })
}

/// Drops generated/rotated glyphs, expands ligatures, and converts real
/// spaces into `space_before` flags (spaces carry no geometry we need).
pub fn prepare(page: &SourcePage) -> Prepared {
    let mut glyphs = Vec::with_capacity(page.glyphs.len());
    let mut rotated = Vec::new();
    let mut pending_space = false;
    for g in &page.glyphs {
        if g.generated {
            // PDFium's inferred separators: only keep them as weak word hints.
            pending_space |= g.ch == ' ';
            continue;
        }
        if g.angle.abs() > 1.0 && (g.angle - 360.0).abs() > 1.0 {
            rotated.push(g.bbox);
            continue;
        }
        if g.ch.is_whitespace() || g.ch == '\u{a0}' {
            pending_space = true;
            continue;
        }
        let size = (g.size * 20.0).round() / 20.0;
        if size <= 0.5 {
            continue;
        }
        let link = page.links.iter().position(|l| l.rect.inflate(1.0).contains(document::Point::new(center_x(g), g.baseline - size * 0.3)));
        let mut push = |ch: char, x0: f32, x1: f32, space_before: bool| {
            glyphs.push(G {
                ch,
                x0,
                x1,
                baseline: g.baseline,
                size,
                font: g.font as usize,
                color: g.color,
                link,
                underline: false,
                fake_bold: g.stroked,
                space_before,
                shift: 0.0,
            });
        };
        match expand_ligature(g.ch) {
            Some(s) => {
                let n = s.chars().count() as f32;
                let w = (g.loose.x1 - g.loose.x0) / n;
                for (i, ch) in s.chars().enumerate() {
                    let x = g.loose.x0 + w * i as f32;
                    push(ch, x, x + w, pending_space && i == 0);
                }
            }
            None => push(g.ch, g.loose.x0, g.loose.x1, pending_space),
        }
        pending_space = false;
    }
    Prepared { glyphs, rotated }
}

fn center_x(g: &SourceGlyph) -> f32 {
    (g.loose.x0 + g.loose.x1) * 0.5
}

/// Deterministic generic-family classification from font flags and name.
pub fn generic_family(f: &SourceFont) -> GenericFamily {
    let n = f.family.to_ascii_lowercase();
    const MONO: [&str; 6] = ["courier", "mono", "consolas", "menlo", "code", "typewriter"];
    const SERIF: [&str; 10] = ["times", "georgia", "garamond", "serif", "cambria", "minion", "palatino", "baskerville", "caslon", "book antiqua"];
    if f.fixed_pitch || MONO.iter().any(|m| n.contains(m)) {
        GenericFamily::Monospace
    } else if n.contains("sans") {
        GenericFamily::SansSerif
    } else if f.serif || SERIF.iter().any(|s| n.contains(s)) {
        GenericFamily::Serif
    } else {
        GenericFamily::SansSerif
    }
}

/// Builds text styles for glyphs, memoising per distinct source style.
pub struct StyleTable<'a> {
    page: &'a SourcePage,
    fonts: Vec<FontSpec>,
}

impl<'a> StyleTable<'a> {
    pub fn new(page: &'a SourcePage) -> Self {
        let fonts = page
            .fonts
            .iter()
            .map(|f| FontSpec {
                family: Arc::from(if f.family.is_empty() { "Helvetica" } else { f.family.as_str() }),
                source_name: (!f.name.is_empty()).then(|| Arc::from(f.name.as_str())),
                weight: f.weight,
                italic: f.italic,
                generic: generic_family(f),
            })
            .collect();
        Self { page, fonts }
    }

    pub fn font(&self, id: usize) -> FontSpec {
        self.fonts.get(id).cloned().unwrap_or_default()
    }

    pub fn is_bold(&self, g: &G) -> bool {
        g.fake_bold || self.fonts.get(g.font).is_some_and(|f| f.is_bold())
    }

    pub fn style(&self, g: &G, letter_spacing: f32) -> TextStyle {
        let mut font = self.font(g.font);
        if g.fake_bold {
            font.weight = font.weight.max(700);
        }
        TextStyle {
            font,
            size: g.size,
            color: g.color,
            underline: g.underline,
            strike: false,
            link: g.link.map(|i| Arc::from(self.page.links[i].uri.as_str())),
            baseline_shift: g.shift,
            letter_spacing,
        }
    }
}
