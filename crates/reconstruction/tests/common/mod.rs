//! Synthetic source pages with exactly known geometry.
//!
//! Advance model: every non-space glyph advances `CHAR_EM * size`, a space
//! glyph `SPACE_EM * size`. Glyph boxes span `baseline - 0.8 * size` to
//! `baseline + 0.2 * size`, matching the reconstruction box model.

use std::sync::Arc;

use document::{Block, Document, Paragraph, PathSeg, Point, Rect, Rgba, Section};
use pdf_source::{FontId, SourceFont, SourceGlyph, SourcePage, SourcePath};

pub const CHAR_EM: f32 = 0.5;
pub const SPACE_EM: f32 = 0.25;

/// Advance width of `text` under the fixed advance model.
pub fn advance(size: f32, text: &str) -> f32 {
    text.chars().map(|c| if c == ' ' { SPACE_EM } else { CHAR_EM } * size).sum()
}

pub struct PageBuilder {
    page: SourcePage,
}

impl PageBuilder {
    pub fn new(width: f32, height: f32) -> Self {
        Self { page: SourcePage { width, height, ..Default::default() } }
    }

    pub fn font(&mut self, name: &str, weight: u16, italic: bool) -> FontId {
        self.page.fonts.push(SourceFont { name: name.into(), family: name.into(), weight, italic, ..Default::default() });
        (self.page.fonts.len() - 1) as FontId
    }

    /// Emits `text` starting at `x`, spaces as real space glyphs. Returns the
    /// x position after the last glyph.
    pub fn text(&mut self, x: f32, baseline: f32, size: f32, font: FontId, text: &str) -> f32 {
        self.emit(x, baseline, size, font, text, true, 0.0)
    }

    /// Like [`Self::text`] but spaces are left as geometric gaps only.
    pub fn text_no_spaces(&mut self, x: f32, baseline: f32, size: f32, font: FontId, text: &str) -> f32 {
        self.emit(x, baseline, size, font, text, false, 0.0)
    }

    /// Like [`Self::text`] with `tracking` points of extra advance after
    /// every glyph (PDF `Tc`).
    pub fn text_tracked(&mut self, x: f32, baseline: f32, size: f32, font: FontId, text: &str, tracking: f32) -> f32 {
        self.emit(x, baseline, size, font, text, true, tracking)
    }

    /// Emits `text` so that its advance ends exactly at `x1`.
    pub fn text_right(&mut self, x1: f32, baseline: f32, size: f32, font: FontId, text: &str) -> f32 {
        self.text(x1 - advance(size, text), baseline, size, font, text)
    }

    /// Greedily word-wraps `text` into lines no wider than `width`, emits
    /// them `pitch` apart and returns the lines.
    #[allow(clippy::too_many_arguments)]
    pub fn wrap_paragraph(
        &mut self,
        x: f32,
        baseline: f32,
        width: f32,
        size: f32,
        pitch: f32,
        font: FontId,
        text: &str,
    ) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for word in text.split_whitespace() {
            match lines.last_mut() {
                Some(line) if advance(size, &format!("{line} {word}")) <= width => {
                    line.push(' ');
                    line.push_str(word);
                }
                _ => lines.push(word.to_string()),
            }
        }
        for (i, line) in lines.iter().enumerate() {
            self.text(x, baseline + i as f32 * pitch, size, font, line);
        }
        lines
    }

    pub fn hrule(&mut self, x0: f32, x1: f32, y: f32, thickness: f32) {
        self.filled_rect(Rect::new(x0, y - thickness * 0.5, x1, y + thickness * 0.5));
    }

    pub fn vrule(&mut self, x: f32, y0: f32, y1: f32, thickness: f32) {
        self.filled_rect(Rect::new(x - thickness * 0.5, y0, x + thickness * 0.5, y1));
    }

    pub fn build(self) -> SourcePage {
        self.page
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(&mut self, x: f32, baseline: f32, size: f32, font: FontId, text: &str, spaces: bool, tracking: f32) -> f32 {
        let mut x = x;
        for ch in text.chars() {
            let adv = advance(size, ch.encode_utf8(&mut [0; 4]));
            if ch != ' ' || spaces {
                let r = Rect::new(x, baseline - 0.8 * size, x + adv, baseline + 0.2 * size);
                self.page.glyphs.push(SourceGlyph {
                    ch,
                    bbox: r,
                    loose: r,
                    origin_x: x,
                    baseline,
                    font,
                    size,
                    color: Rgba::BLACK,
                    angle: 0.0,
                    generated: false,
                    stroked: false,
                });
            }
            x += adv + tracking;
        }
        x
    }

    fn filled_rect(&mut self, r: Rect) {
        let segments = vec![
            PathSeg::MoveTo(Point::new(r.x0, r.y0)),
            PathSeg::LineTo(Point::new(r.x1, r.y0)),
            PathSeg::LineTo(Point::new(r.x1, r.y1)),
            PathSeg::LineTo(Point::new(r.x0, r.y1)),
            PathSeg::Close,
        ];
        self.page.paths.push(SourcePath { bounds: r, segments, fill: Some(Rgba::BLACK), stroke: None });
    }
}

/// Reconstructs `page` into a one-section document.
pub fn reconstruct(page: &SourcePage) -> Document {
    Document { sections: vec![Arc::new(reconstruction::reconstruct(page).section)] }
}

pub fn section(doc: &Document) -> &Section {
    &doc.sections[0]
}

/// Paragraph blocks of `blocks`, panicking (with the outline) on anything else.
pub fn paragraphs<'a>(doc: &Document, blocks: &'a [Block]) -> Vec<&'a Paragraph> {
    blocks
        .iter()
        .map(|b| match b {
            Block::Paragraph(p) => p,
            other => panic!("expected only paragraphs, got {other:?}\n{}", doc.outline()),
        })
        .collect()
}
