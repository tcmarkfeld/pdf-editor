//! Stage 1: glyphs -> words -> line fragments.
//!
//! A *fragment* is a horizontal run of words on one baseline with no large
//! gap inside it. Two fragments sharing a baseline (a job title and its
//! right-aligned date, or lines of neighbouring columns) are kept apart here;
//! later stages decide whether they form a tabbed row, columns or a table.

use document::{DESCENT_RATIO, Rect};

use crate::glyphs::G;

/// Word break when the gap between glyph advances exceeds this many ems.
pub const WORD_GAP_EM: f32 = 0.15;
/// Fragment break (column gutter / tab gap) threshold, in ems.
pub const FRAGMENT_GAP_EM: f32 = 1.2;
/// Glyphs whose baselines differ by less than this many ems share a line.
pub const BASELINE_TOL_EM: f32 = 0.25;

#[derive(Clone, Debug)]
pub struct Word {
    /// Glyph index range within the owning line.
    pub start: usize,
    pub end: usize,
    pub x0: f32,
    pub x1: f32,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub glyphs: Vec<G>,
    pub words: Vec<Word>,
    pub baseline: f32,
    /// Largest font size on the line.
    pub size: f32,
    pub x0: f32,
    pub x1: f32,
    /// Tab stops (absolute x) introduced when fragments of one row are
    /// merged; `word` is the index of the first word after the tab.
    pub tabs: Vec<Tab>,
}

#[derive(Clone, Copy, Debug)]
pub struct Tab {
    pub word: usize,
    pub align: document::TabAlign,
    pub pos: f32,
}

impl Line {
    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn top(&self) -> f32 {
        self.baseline - (1.0 - DESCENT_RATIO) * self.size
    }

    pub fn bottom(&self) -> f32 {
        self.baseline + DESCENT_RATIO * self.size
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x0, self.top(), self.x1, self.bottom())
    }

    pub fn center(&self) -> f32 {
        (self.x0 + self.x1) * 0.5
    }

    pub fn word_text(&self, w: usize) -> String {
        let w = &self.words[w];
        self.glyphs[w.start..w.end].iter().map(|g| g.ch).collect()
    }

    /// Merges `other` (to the right on the same row) into this line behind
    /// a tab stop.
    pub fn append_tabbed(&mut self, mut other: Line, align: document::TabAlign, pos: f32) {
        let offset = self.glyphs.len();
        self.tabs.push(Tab { word: self.words.len(), align, pos });
        for w in &mut other.words {
            w.start += offset;
            w.end += offset;
        }
        for t in &mut other.tabs {
            t.word += self.words.len();
        }
        self.glyphs.append(&mut other.glyphs);
        self.words.append(&mut other.words);
        self.tabs.append(&mut other.tabs);
        self.x1 = self.x1.max(other.x1);
        self.size = self.size.max(other.size);
    }
}

/// Groups glyphs into line fragments. `vrules` are x positions (with their
/// y-extent) of vertical ruling lines, which always separate fragments.
pub fn build_lines(mut glyphs: Vec<G>, vrules: &[(f32, f32, f32)]) -> Vec<Line> {
    glyphs.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x0.total_cmp(&b.x0)));

    // Baseline clustering.
    let mut groups: Vec<Vec<G>> = Vec::new();
    for g in glyphs {
        match groups.last_mut() {
            Some(group) if (g.baseline - group[0].baseline).abs() <= BASELINE_TOL_EM * g.size.min(group[0].size) => {
                group.push(g)
            }
            _ => groups.push(vec![g]),
        }
    }

    let mut lines = Vec::new();
    for mut group in groups {
        group.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        dedupe_overprint(&mut group);
        let mut current: Vec<G> = Vec::new();
        for g in group {
            if let Some(prev) = current.last() {
                let gap = g.x0 - prev.x1;
                let em = prev.size.max(g.size);
                let ruled = vrules.iter().any(|&(x, y0, y1)| {
                    x > prev.x1 - 0.5 && x < g.x0 + 0.5 && g.baseline > y0 && g.baseline - g.size * 0.5 < y1
                });
                if gap > FRAGMENT_GAP_EM * em || ruled {
                    lines.push(make_line(std::mem::take(&mut current)));
                }
            }
            current.push(g);
        }
        if !current.is_empty() {
            lines.push(make_line(current));
        }
    }
    lines.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x0.total_cmp(&b.x0)));
    lines
}

/// Some producers fake bold by drawing a glyph twice with a tiny offset.
fn dedupe_overprint(glyphs: &mut Vec<G>) {
    let mut out: Vec<G> = Vec::with_capacity(glyphs.len());
    for g in glyphs.drain(..) {
        let same = |p: &&mut G| p.ch == g.ch && (p.x0 - g.x0).abs() < 0.4 && (p.baseline - g.baseline).abs() < 0.4;
        if let Some(prev) = out.iter_mut().rev().take(3).find(same) {
            prev.fake_bold = true;
            continue;
        }
        out.push(g);
    }
    *glyphs = out;
}

fn make_line(glyphs: Vec<G>) -> Line {
    let mut words = Vec::new();
    let mut start = 0;
    for i in 1..=glyphs.len() {
        let brk = i == glyphs.len() || {
            let (a, b) = (&glyphs[i - 1], &glyphs[i]);
            let gap = b.x0 - a.x1;
            gap > WORD_GAP_EM * a.size.max(b.size) || (b.space_before && gap > -0.05 * b.size)
        };
        if brk {
            words.push(Word { start, end: i, x0: glyphs[start].x0, x1: glyphs[i - 1].x1 });
            start = i;
        }
    }
    // Median baseline is robust against a stray sub/superscript glyph.
    let mut bl: Vec<f32> = glyphs.iter().map(|g| g.baseline).collect();
    bl.sort_by(f32::total_cmp);
    Line {
        baseline: bl[bl.len() / 2],
        size: glyphs.iter().map(|g| g.size).fold(0.0, f32::max),
        x0: glyphs.first().map_or(0.0, |g| g.x0),
        x1: glyphs.last().map_or(0.0, |g| g.x1),
        glyphs,
        words,
        tabs: Vec::new(),
    }
}
