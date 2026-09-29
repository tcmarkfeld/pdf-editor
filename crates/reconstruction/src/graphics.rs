//! Vector graphics classification: horizontal/vertical rules, underlines,
//! vector-drawn bullets, and everything else (page decorations).

use document::{PathSeg, Rect, Rgba};
use pdf_source::SourcePath;

use crate::glyphs::G;
use crate::lines::{Line, Word};

#[derive(Clone, Debug)]
pub struct HRule {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub thickness: f32,
    pub color: Rgba,
    pub source: usize,
}

#[derive(Clone, Debug)]
pub struct VRule {
    pub x: f32,
    pub y0: f32,
    pub y1: f32,
    pub source: usize,
}

#[derive(Default)]
pub struct Graphics {
    pub hrules: Vec<HRule>,
    pub vrules: Vec<VRule>,
    pub dots: Vec<(Rect, Rgba, usize)>,
    /// Indices of paths that are neither rules nor bullets.
    pub other: Vec<usize>,
}

/// Axis-aligned rectangle a path traces, if it is one (possibly degenerate:
/// a single stroked line).
fn as_rect(p: &SourcePath) -> Option<Rect> {
    let pts: Vec<_> = p
        .segments
        .iter()
        .filter_map(|s| match s {
            PathSeg::MoveTo(p) | PathSeg::LineTo(p) => Some(*p),
            PathSeg::CurveTo(..) => None,
            PathSeg::Close => None,
        })
        .collect();
    if pts.len() < 2 || p.segments.iter().any(|s| matches!(s, PathSeg::CurveTo(..))) || pts.len() > 6 {
        return None;
    }
    let axis = pts.windows(2).all(|w| (w[0].x - w[1].x).abs() < 0.1 || (w[0].y - w[1].y).abs() < 0.1);
    axis.then_some(p.bounds)
}

pub fn classify(paths: &[SourcePath]) -> Graphics {
    let mut g = Graphics::default();
    for (i, p) in paths.iter().enumerate() {
        let b = p.bounds;
        let color = p.fill.or(p.stroke.map(|s| s.0)).unwrap_or(Rgba::BLACK);
        let stroke_w = p.stroke.map_or(0.0, |s| s.1);
        if let Some(r) = as_rect(p) {
            let (w, h) = (r.width(), r.height());
            if h + stroke_w <= 3.0 && w >= 6.0 {
                g.hrules.push(HRule { x0: r.x0, x1: r.x1, y: r.center_y(), thickness: (h.max(stroke_w)).max(0.25), color, source: i });
                continue;
            }
            if w + stroke_w <= 3.0 && h >= 6.0 {
                g.vrules.push(VRule { x: r.center_x(), y0: r.y0, y1: r.y1, source: i });
                continue;
            }
            // A stroked rectangle outline contributes four rules.
            if p.fill.is_none() && stroke_w > 0.0 && w > 6.0 && h > 6.0 {
                for (y, x0, x1) in [(r.y0, r.x0, r.x1), (r.y1, r.x0, r.x1)] {
                    g.hrules.push(HRule { x0, x1, y, thickness: stroke_w, color, source: i });
                }
                for x in [r.x0, r.x1] {
                    g.vrules.push(VRule { x, y0: r.y0, y1: r.y1, source: i });
                }
                continue;
            }
        }
        let small = b.width() <= 7.0 && b.height() <= 7.0 && b.width() > 0.8;
        let squarish = (b.width() / b.height().max(0.01) - 1.0).abs() < 0.4;
        if p.fill.is_some() && small && squarish {
            g.dots.push((b, color, i));
            continue;
        }
        g.other.push(i);
    }
    g
}

/// Marks glyphs sitting just above a thin rule as underlined; returns the
/// source indices of rules consumed as underlines.
pub fn apply_underlines(lines: &mut [Line], hrules: &[HRule]) -> Vec<usize> {
    let mut used = Vec::new();
    for r in hrules {
        if r.thickness > 1.5 {
            continue;
        }
        for line in lines.iter_mut() {
            let dy = r.y - line.baseline;
            if dy < -0.05 * line.size || dy > 0.3 * line.size || r.x0 < line.x0 - 1.0 || r.x1 > line.x1 + 1.0 {
                continue;
            }
            let mut hit = false;
            for gl in &mut line.glyphs {
                let c = (gl.x0 + gl.x1) * 0.5;
                if c >= r.x0 && c <= r.x1 {
                    gl.underline = true;
                    hit = true;
                }
            }
            if hit {
                used.push(r.source);
                break;
            }
        }
    }
    used
}

/// Vector dots immediately left of a line become bullet glyphs.
pub fn apply_dot_bullets(lines: &mut [Line], dots: &[(Rect, Rgba, usize)]) -> Vec<usize> {
    let mut used = Vec::new();
    for &(d, color, source) in dots {
        let cy = d.center_y();
        let Some(line) = lines.iter_mut().find(|l| {
            cy <= l.baseline && cy >= l.baseline - 0.75 * l.size && l.x0 - d.x1 > 0.0 && l.x0 - d.x1 < 2.5 * l.size
        }) else {
            continue;
        };
        // Size the substitute "•" so its disc matches the drawn one, and shift
        // it to the same height. A bullet's disc is ~0.26em wide centred
        // ~0.36em above the baseline in common text faces.
        let proto = line.glyphs[0].clone();
        let size = (d.width() / 0.26).clamp(0.6 * line.size, 1.8 * line.size);
        let shift = (line.baseline - cy) - 0.36 * size;
        let bullet = G { ch: '•', x0: d.x0, x1: d.x1, color, size, shift, underline: false, link: None, space_before: false, ..proto };
        line.glyphs.insert(0, bullet);
        for w in &mut line.words {
            w.start += 1;
            w.end += 1;
        }
        line.words.insert(0, Word { start: 0, end: 1, x0: d.x0, x1: d.x1 });
        for t in &mut line.tabs {
            t.word += 1;
        }
        line.x0 = d.x0;
        used.push(source);
    }
    used
}

/// A table lattice formed by ruling lines: column and row boundaries.
pub struct Grid {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

/// Finds clean ruling grids: connected sets of rules whose distinct
/// horizontal rules each span the full width and whose vertical rules each
/// span the full height. Tables drawn this way are recognised even when
/// their cells are empty.
pub fn ruled_grids(g: &Graphics) -> Vec<Grid> {
    let touches = |v: &VRule, h: &HRule| v.x >= h.x0 - 1.0 && v.x <= h.x1 + 1.0 && h.y >= v.y0 - 1.0 && h.y <= v.y1 + 1.0;
    let (nh, nv) = (g.hrules.len(), g.vrules.len());
    // Union-find over rules: indices 0..nh are horizontal, nh.. vertical.
    let mut parent: Vec<usize> = (0..nh + nv).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for (hi, h) in g.hrules.iter().enumerate() {
        for (vi, v) in g.vrules.iter().enumerate() {
            if touches(v, h) {
                let (a, b) = (find(&mut parent, hi), find(&mut parent, nh + vi));
                parent[a] = b;
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, (Vec<usize>, Vec<usize>)> = Default::default();
    for i in 0..nh + nv {
        let root = find(&mut parent, i);
        let e = groups.entry(root).or_default();
        if i < nh { e.0.push(i) } else { e.1.push(i - nh) }
    }
    let dedup = |mut v: Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 2.0);
        v
    };
    let mut out = Vec::new();
    for (hs, vs) in groups.into_values() {
        let ys = dedup(hs.iter().map(|&i| g.hrules[i].y).collect());
        let xs = dedup(vs.iter().map(|&i| g.vrules[i].x).collect());
        if xs.len() < 2 || ys.len() < 2 {
            continue;
        }
        let (x0, x1, y0, y1) = (xs[0], xs[xs.len() - 1], ys[0], ys[ys.len() - 1]);
        // Coverage of each boundary by the union of its segments.
        let covered = |segs: Vec<(f32, f32)>, lo: f32, hi: f32| {
            let mut segs = segs;
            segs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (mut total, mut cur) = (0.0, lo);
            for (a, b) in segs {
                let (a, b) = (a.max(cur), b.min(hi));
                if b > a {
                    total += b - a;
                    cur = b;
                }
            }
            total >= 0.9 * (hi - lo)
        };
        let rows_ok = ys.iter().all(|&y| {
            covered(hs.iter().map(|&i| &g.hrules[i]).filter(|h| (h.y - y).abs() < 2.0).map(|h| (h.x0, h.x1)).collect(), x0, x1)
        });
        let cols_ok = xs.iter().all(|&x| {
            covered(vs.iter().map(|&i| &g.vrules[i]).filter(|v| (v.x - x).abs() < 2.0).map(|v| (v.y0, v.y1)).collect(), y0, y1)
        });
        if rows_ok && cols_ok {
            out.push(Grid { xs, ys });
        }
    }
    out.sort_by(|a, b| a.ys[0].total_cmp(&b.ys[0]));
    out
}
