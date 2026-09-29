//! Deterministic reconstruction confidence. Each component is the fraction
//! of measurable evidence that agrees with the chosen structure; nothing is
//! learned or probabilistic.

use document::Align;

use crate::lines::Line;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReconstructionScore {
    /// Lines consistent with the paragraph's alignment model.
    pub geometry: f32,
    /// Characters in the paragraph's dominant size/weight.
    pub typography: f32,
    /// Line pitches within 5% of the paragraph's pitch.
    pub spacing: f32,
}

impl ReconstructionScore {
    pub fn min(&self) -> f32 {
        self.geometry.min(self.typography).min(self.spacing)
    }
}

pub fn paragraph(lines: &[Line], align: Align, x0: f32, x1: f32, is_bold: impl Fn(&crate::glyphs::G) -> bool) -> ReconstructionScore {
    const TOL: f32 = 1.5;
    let n = lines.len();
    let cont_left = if n >= 2 { lines[1].x0 } else { lines[0].x0 };
    let max_right = lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max);
    let fits = |l: &Line, i: usize| match align {
        Align::Left => i == 0 || (l.x0 - cont_left).abs() <= TOL,
        Align::Justify => i == 0 || ((l.x0 - cont_left).abs() <= TOL && (i + 1 == n || (l.x1 - max_right).abs() <= TOL)),
        Align::Center => (l.center() - (x0 + x1) * 0.5).abs() <= 2.0,
        Align::Right => (l.x1 - max_right).abs() <= TOL,
    };
    let geometry = lines.iter().enumerate().filter(|(i, l)| fits(l, *i)).count() as f32 / n as f32;

    let mut counts: Vec<((i32, bool), usize)> = Vec::new();
    let mut total = 0;
    for l in lines {
        for g in l.words.iter().flat_map(|w| &l.glyphs[w.start..w.end]) {
            let key = ((g.size * 10.0).round() as i32, is_bold(g));
            match counts.iter_mut().find(|c| c.0 == key) {
                Some(c) => c.1 += 1,
                None => counts.push((key, 1)),
            }
            total += 1;
        }
    }
    let typography = counts.iter().map(|c| c.1).max().unwrap_or(0) as f32 / total.max(1) as f32;

    let pitches: Vec<f32> = lines.windows(2).map(|w| w[1].baseline - w[0].baseline).collect();
    let spacing = if pitches.is_empty() {
        1.0
    } else {
        let m = crate::paragraphs::median(pitches.clone());
        pitches.iter().filter(|p| (*p - m).abs() <= 0.05 * m).count() as f32 / pitches.len() as f32
    };
    ReconstructionScore { geometry, typography, spacing }
}
