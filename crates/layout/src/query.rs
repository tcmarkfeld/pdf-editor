//! Geometry queries used by the editor: hit testing, caret and selection
//! rectangles, and visual-line navigation.

use std::ops::Range;

use document::Rect;
use parley::{Affinity, Cursor, Selection};

use crate::{DocLayout, ParaLayout, VisualLine};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub para: usize,
    pub offset: usize,
}

impl DocLayout {
    fn global_para(&self, section: usize, local: usize) -> usize {
        self.paras.partition_point(|&(s, _)| s < section) + local
    }

    /// The caret position nearest to a point on a page.
    pub fn hit(&self, page: usize, x: f32, y: f32) -> Option<Hit> {
        let (si, lp) = self.pages[page];
        let mut best: Option<(f32, usize, usize)> = None;
        for (pi, pl) in self.sections[si].paras.iter().enumerate() {
            for (li, line) in pl.lines.iter().enumerate() {
                if line.page != lp {
                    continue;
                }
                let dy = (line.top - y).max(y - line.bottom).max(0.0);
                let dx = (line.x0 - x).max(x - line.x1).max(0.0);
                let score = dy * 2.0 + dx;
                if best.is_none_or(|b| score < b.0) {
                    best = Some((score, pi, li));
                }
            }
        }
        let (_, pi, li) = best?;
        let para = self.global_para(si, pi);
        Some(Hit { para, offset: self.offset_at(para, li, x) })
    }

    /// Caret offset on visual line `line` of `para` closest to page x.
    pub fn offset_at(&self, para: usize, line: usize, x: f32) -> usize {
        let pl = self.para(para);
        let Some(vl) = pl.lines.get(line) else { return 0 };
        let best = vl.parts.iter().min_by(|a, b| part_distance(pl, a, x).total_cmp(&part_distance(pl, b, x)));
        let Some(part) = best else { return vl.text.start };
        let seg = &pl.segments[part.seg];
        let m = seg.layout.get(part.line).map(|l| *l.metrics()).expect("line");
        let cursor = Cursor::from_point(&seg.layout, x - part.dx, (m.block_min_coord + m.block_max_coord) * 0.5);
        seg.start + cursor.index()
    }

    /// Visual line of `para` containing the caret at `offset`.
    pub fn line_of(&self, para: usize, offset: usize) -> usize {
        self.locate(para, offset).map_or(0, |(li, _)| li)
    }

    /// Caret rectangle (global page, page-space rect).
    pub fn caret(&self, para: usize, offset: usize) -> Option<(usize, Rect)> {
        let pl = self.para(para);
        let (li, x) = self.locate(para, offset)?;
        let vl = &pl.lines[li];
        let size = (vl.bottom - vl.top).min(pl_line_size(vl) * 1.25);
        Some((self.line_page(para, vl), Rect::new(x, vl.baseline - size * 0.8, x, vl.baseline + size * 0.25)))
    }

    /// Visual line index and page x of the caret at `offset`.
    fn locate(&self, para: usize, offset: usize) -> Option<(usize, f32)> {
        let pl = self.para(para);
        let si = pl.segments.iter().position(|s| offset <= s.end).unwrap_or(pl.segments.len().saturating_sub(1));
        let seg = pl.segments.get(si)?;
        if seg.layout.is_empty() {
            let li = pl.lines.iter().position(|l| l.parts.iter().all(|p| p.seg != si)).unwrap_or(0);
            return Some((li, pl.lines.get(li)?.x0));
        }
        let local = offset.clamp(seg.start, seg.end) - seg.start;
        let cursor = Cursor::from_byte_index(&seg.layout, local, Affinity::Downstream);
        let bb = cursor.geometry(&seg.layout, 1.0);
        let cy = (bb.y0 + bb.y1) as f32 * 0.5;
        let k = seg
            .layout
            .lines()
            .position(|l| cy >= l.metrics().block_min_coord && cy <= l.metrics().block_max_coord)
            .unwrap_or(0);
        pl.lines
            .iter()
            .enumerate()
            .find_map(|(li, vl)| vl.parts.iter().find(|p| p.seg == si && p.line == k).map(|p| (li, bb.x0 as f32 + p.dx)))
    }

    /// Selection highlight rectangles for `range` of `para`.
    pub fn selection_rects(&self, para: usize, range: Range<usize>) -> Vec<(usize, Rect)> {
        let pl = self.para(para);
        let mut out = Vec::new();
        for (si, seg) in pl.segments.iter().enumerate() {
            let a = range.start.max(seg.start);
            let b = range.end.min(seg.end);
            if a >= b {
                continue;
            }
            let sel = Selection::new(
                Cursor::from_byte_index(&seg.layout, a - seg.start, Affinity::Downstream),
                Cursor::from_byte_index(&seg.layout, b - seg.start, Affinity::Upstream),
            );
            for (bb, k) in sel.geometry(&seg.layout) {
                if let Some((vl, p)) = pl.lines.iter().find_map(|vl| vl.parts.iter().find(|p| p.seg == si && p.line == k).map(|p| (vl, p))) {
                    let r = Rect::new(bb.x0 as f32 + p.dx, vl.top, bb.x1 as f32 + p.dx, vl.bottom);
                    out.push((self.line_page(para, vl), r));
                }
            }
        }
        // Tabs between segments and empty paragraphs still show as selected.
        for (i, vl) in pl.lines.iter().enumerate() {
            let covers_break = range.start <= vl.text.end && range.end > vl.text.end && i + 1 == pl.lines.len();
            if covers_break || (vl.text.is_empty() && range.start <= vl.text.start && range.end > vl.text.start) {
                out.push((self.line_page(para, vl), Rect::new(vl.x1, vl.top, vl.x1 + 4.0, vl.bottom)));
            }
        }
        out
    }

    pub fn line_count(&self, para: usize) -> usize {
        self.para(para).lines.len()
    }

    /// Offset at the start / end of visual line `line` (end excludes a
    /// trailing soft-wrap space).
    pub fn line_bounds(&self, para: usize, line: usize, text: &str) -> (usize, usize) {
        let pl = self.para(para);
        let Some(vl) = pl.lines.get(line) else { return (0, 0) };
        let mut end = vl.text.end.min(text.len());
        if line + 1 < pl.lines.len() {
            while end > vl.text.start && text[..end].ends_with(' ') {
                end -= 1;
            }
        }
        (vl.text.start.min(end), end)
    }
}

fn part_distance(pl: &ParaLayout, p: &crate::Part, x: f32) -> f32 {
    let Some(line) = pl.segments[p.seg].layout.get(p.line) else { return f32::MAX };
    let m = line.metrics();
    let x0 = p.dx + m.offset;
    let x1 = x0 + m.advance;
    (x0 - x).max(x - x1).max(0.0)
}

fn pl_line_size(vl: &VisualLine) -> f32 {
    (vl.bottom - vl.top).max(1.0)
}
