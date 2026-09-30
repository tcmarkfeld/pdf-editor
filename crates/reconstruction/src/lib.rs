//! Deterministic layout reconstruction: `SourcePage` -> editable `Section`.
//!
//! ```text
//! glyphs ──► words ──► line fragments ──► zones (flow / columns / table)
//!                                              │
//!            rules, underlines, bullets ◄──────┤
//!                                              ▼
//!                 paragraphs (lists, tab rows, headings, spacing) ──► Section
//! ```
//!
//! Every decision is a threshold on measured geometry or typography; there is
//! no learned or probabilistic component. See `docs/RECONSTRUCTION.md`.

mod glyphs;
mod graphics;
mod lines;
mod paragraphs;
mod score;
mod zones;

pub use score::ReconstructionScore;

use std::collections::HashMap;
use std::sync::Arc;

use document::{
    Block, Border, Column, Columns, Decoration, ImageBlock, ImageResource, Margins, PathShape, Rect,
    Rule, Section, Size, Table, TableCell, TableRow,
};
use pdf_source::SourcePage;

use crate::glyphs::StyleTable;
use crate::graphics::{Graphics, HRule};
use crate::lines::Line;
use crate::paragraphs::{PageContext, Placed, build_paragraphs, median, round2};
use crate::zones::Zone;

/// Intermediate results kept for debug overlays and diagnostics.
#[derive(Clone, Debug, Default)]
pub struct PageAnalysis {
    pub glyphs: Vec<Rect>,
    pub words: Vec<Rect>,
    pub lines: Vec<Rect>,
    pub blocks: Vec<(Rect, &'static str)>,
    pub zones: Vec<(Rect, &'static str)>,
    /// Confidence for each reconstructed paragraph.
    pub scores: Vec<(Rect, ReconstructionScore)>,
    pub notes: Vec<String>,
}

pub struct Reconstructed {
    pub section: Section,
    pub analysis: PageAnalysis,
    /// Page regions whose content cannot be represented (e.g. rotated text).
    /// The caller should preserve them as raster crops of the source page.
    pub raster_fallbacks: Vec<Rect>,
}

/// Extracts and reconstructs one page of a PDF, preserving unrepresentable
/// regions as raster crops of the source rendering.
pub fn import_page(source: &pdf_source::PdfSource, index: u32) -> Result<(Section, PageAnalysis), String> {
    let r = reconstruct(&source.extract(index)?);
    let mut section = r.section;
    for rect in r.raster_fallbacks {
        let (png, w, h) = source.render_crop(index, rect, 3.0)?;
        section.decorations.push(Decoration::Image { rect, image: Arc::new(ImageResource::png(png, w, h)) });
    }
    Ok((section, r.analysis))
}

pub fn reconstruct(page: &SourcePage) -> Reconstructed {
    let mut analysis = PageAnalysis { glyphs: page.glyphs.iter().filter(|g| !g.generated).map(|g| g.bbox).collect(), ..Default::default() };
    let prepared = glyphs::prepare(page);
    let raster_fallbacks = merge_boxes(&prepared.rotated);
    if !prepared.rotated.is_empty() {
        analysis.notes.push(format!("{} rotated glyphs preserved as raster fallback", prepared.rotated.len()));
    }

    let gfx = graphics::classify(&page.paths);
    let vr: Vec<(f32, f32, f32)> = gfx.vrules.iter().map(|v| (v.x, v.y0, v.y1)).collect();
    let mut lines = lines::build_lines(prepared.glyphs, &vr);
    let mut consumed: Vec<usize> = graphics::apply_underlines(&mut lines, &gfx.hrules);
    consumed.extend(graphics::apply_dot_bullets(&mut lines, &gfx.dots));

    for l in &lines {
        analysis.lines.push(l.rect());
        for w in &l.words {
            analysis.words.push(Rect::new(w.x0, l.top(), w.x1, l.bottom()));
        }
    }

    let styles = StyleTable::new(page);
    let (body_size, body_bold, heading_sizes) = typography(&lines, &styles);
    let mut ctx = PageContext { styles, body_size, body_bold, heading_sizes, next_list_id: page.index * 1000 };

    let mut section = Section {
        page_size: Size::new(page.width, page.height),
        source_page: Some(page.index),
        ..Default::default()
    };

    let mut builder = Builder { lines: &lines, gfx: &gfx, page, consumed, analysis: &mut analysis, claims: Vec::new(), placed_images: Vec::new() };
    if !lines.is_empty() {
        let x0 = lines.iter().map(|l| l.x0).fold(f32::MAX, f32::min);
        let x1 = lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max);
        // Ruled grids first: they locate tables even when cells are empty.
        let mut in_table = vec![false; lines.len()];
        let mut tables = Vec::new();
        for grid in graphics::ruled_grids(&gfx) {
            let (xs, ys) = (&grid.xs, &grid.ys);
            let frame = Rect::new(xs[0], ys[0], xs[xs.len() - 1], ys[ys.len() - 1]);
            let mut rows = vec![vec![Vec::new(); xs.len() - 1]; ys.len() - 1];
            for (i, l) in lines.iter().enumerate() {
                let (cx, cy) = (l.center(), l.baseline - 0.3 * l.size);
                if in_table[i] || !frame.contains(document::Point::new(cx, cy)) {
                    continue;
                }
                let r = ys.windows(2).position(|w| cy >= w[0] && cy < w[1]).unwrap_or(0);
                let c = xs.windows(2).position(|w| cx >= w[0] && cx < w[1]).unwrap_or(0);
                rows[r][c].push(i);
                in_table[i] = true;
            }
            let cols = xs.windows(2).map(|w| (w[0], w[1])).collect();
            tables.push(Zone::Table { cols, rows, frame: Some(frame) });
        }
        let free = (0..lines.len()).filter(|&i| !in_table[i]).collect();
        let mut zones = zones::segment(&lines, free, x0, x1, 0);
        zones::insert_tables(&lines, &mut zones, tables);
        builder.claim_graphics(&zones);
        let placed = builder.zones(&mut ctx, &zones, x0, x1);
        let top = placed.iter().map(|p| p.top).fold(f32::MAX, f32::min);
        let bottom = placed.iter().map(|p| p.bottom).fold(f32::MIN, f32::max);
        section.margins = Margins {
            top: round2(top),
            left: round2(x0),
            right: round2((page.width - x1).max(0.0)),
            bottom: round2((page.height - bottom - 1.0).clamp(0.0, top.max(0.0))),
        };
        section.blocks = finalize(placed, top);
    } else {
        analysis_note(builder.analysis, "no extractable text");
    }
    section.decorations = builder.decorations();
    Reconstructed { section, analysis, raster_fallbacks }
}

/// Unions boxes that touch (within 2pt) into regions.
fn merge_boxes(boxes: &[Rect]) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for b in boxes {
        let b = b.inflate(1.0);
        match out.iter_mut().find(|r| r.inflate(2.0).intersects(&b)) {
            Some(r) => *r = r.union(&b),
            None => out.push(b),
        }
    }
    out
}

fn analysis_note(a: &mut PageAnalysis, s: &str) {
    a.notes.push(s.to_string());
}

/// Body size (character-weighted mode), whether body text is bold, and the
/// distinct sizes used by short, larger-than-body lines (heading candidates).
fn typography(lines: &[Line], styles: &StyleTable) -> (f32, bool, Vec<f32>) {
    let mut hist: HashMap<i32, usize> = HashMap::new();
    let mut bold = 0;
    let mut total = 0;
    for l in lines {
        for g in &l.glyphs {
            *hist.entry((g.size * 2.0).round() as i32).or_default() += 1;
            bold += styles.is_bold(g) as usize;
            total += 1;
        }
    }
    let body = hist.iter().max_by_key(|(k, v)| (**v, -**k)).map_or(11.0, |(k, _)| *k as f32 / 2.0);
    let mut heads: Vec<f32> = lines
        .iter()
        .filter(|l| l.glyphs.len() <= 60 && l.size >= body * 1.15)
        .map(|l| (l.size * 2.0).round() / 2.0)
        .collect();
    heads.sort_by(|a, b| b.total_cmp(a));
    heads.dedup_by(|a, b| (*a - *b).abs() < 0.05 * *b);
    (body, bold * 2 > total.max(1), heads)
}

/// Converts placed drafts to blocks with `space_before` measured from the
/// previous block's bottom (the flow's top for the first block).
pub(crate) fn finalize(mut placed: Vec<Placed>, flow_top: f32) -> Vec<Block> {
    placed.sort_by(|a, b| a.top.total_cmp(&b.top));
    let mut prev = flow_top;
    placed
        .into_iter()
        .map(|p| {
            let mut block = p.block;
            let space = round2(p.top - prev);
            match &mut block {
                Block::Paragraph(x) => x.style.space_before = space,
                Block::Columns(x) => x.space_before = space,
                Block::Table(x) => x.space_before = space,
                Block::Image(x) => x.space_before = space,
                Block::Rule(x) => x.space_before = space,
                Block::Frame(_) => return block, // out of flow: takes no space
            }
            prev = p.bottom;
            block
        })
        .collect()
}

enum Claimed {
    Rule(HRule),
    Image(usize),
}

struct Builder<'a> {
    lines: &'a [Line],
    gfx: &'a Graphics,
    page: &'a SourcePage,
    /// Source path indices already represented in the document.
    consumed: Vec<usize>,
    analysis: &'a mut PageAnalysis,
    /// Graphics assigned to flow zones, in zone walk order.
    claims: Vec<Vec<Claimed>>,
    placed_images: Vec<usize>,
}

impl Builder<'_> {
    fn zone_rect(&self, z: &Zone) -> Rect {
        if let Zone::Table { frame: Some(f), .. } = z {
            return *f;
        }
        let mut ids = Vec::new();
        z.line_ids(&mut ids);
        ids.iter().fold(Rect::EMPTY, |r, &i| r.union(&self.lines[i].rect()))
    }

    fn flow_zones<'z>(zones: &'z [Zone], out: &mut Vec<&'z Zone>) {
        for z in zones {
            match z {
                Zone::Flow { .. } => out.push(z),
                Zone::Columns { cols } => cols.iter().for_each(|c| Self::flow_zones(&c.zones, out)),
                Zone::Table { .. } => {}
            }
        }
    }

    /// Assigns horizontal rules and images to the nearest flow zone that
    /// horizontally contains them; table rules are left to the table.
    fn claim_graphics(&mut self, zones: &[Zone]) {
        let mut flows = Vec::new();
        Self::flow_zones(zones, &mut flows);
        let rects: Vec<(f32, f32, Rect)> = flows
            .iter()
            .map(|z| match z {
                Zone::Flow { x0, x1, .. } => (*x0, *x1, self.zone_rect(z)),
                _ => unreachable!(),
            })
            .collect();
        let mut tables = Vec::new();
        collect_tables(zones, &mut tables);
        let table_rects: Vec<Rect> = tables.iter().map(|z| self.zone_rect(z).inflate(12.0)).collect();
        self.claims = (0..flows.len()).map(|_| Vec::new()).collect();

        let nearest = |x0: f32, x1: f32, y0: f32, y1: f32| {
            rects
                .iter()
                .enumerate()
                .filter(|(_, (fx0, fx1, _))| x0 >= fx0 - 4.0 && x1 <= fx1 + 4.0)
                .map(|(i, (_, _, r))| (i, (r.y0 - y1).max(y0 - r.y1).max(0.0)))
                .filter(|(_, d)| *d < 48.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        };
        for r in &self.gfx.hrules {
            if self.consumed.contains(&r.source) || table_rects.iter().any(|t| t.contains(document::Point::new(r.x0 + 1.0, r.y)) && r.x1 <= t.x1) {
                continue;
            }
            if let Some(i) = nearest(r.x0, r.x1, r.y, r.y) {
                self.claims[i].push(Claimed::Rule(r.clone()));
                self.consumed.push(r.source);
            }
        }
        for (ii, img) in self.page.images.iter().enumerate() {
            let r = img.rect;
            let Some(i) = nearest(r.x0, r.x1, r.y0, r.y1) else { continue };
            let (zx0, zx1, _) = rects[i];
            // Text beside the image would be pushed around by flow placement.
            let beside = self.lines.iter().any(|l| l.rect().y_overlap(&r) > 1.0 && l.x1 > zx0 && l.x0 < zx1);
            if !beside {
                self.claims[i].push(Claimed::Image(ii));
                self.placed_images.push(ii);
            }
        }
    }

    fn zones(&mut self, ctx: &mut PageContext, zones: &[Zone], fx0: f32, fx1: f32) -> Vec<Placed> {
        let mut flow_index = 0;
        self.zones_inner(ctx, zones, fx0, fx1, &mut flow_index)
    }

    fn zones_inner(&mut self, ctx: &mut PageContext, zones: &[Zone], fx0: f32, _fx1: f32, flow_index: &mut usize) -> Vec<Placed> {
        let mut out = Vec::new();
        for z in zones {
            let rect = self.zone_rect(z);
            match z {
                Zone::Flow { lines: ids, x0, x1 } => {
                    self.analysis.zones.push((rect, "flow"));
                    let lines: Vec<Line> = ids.iter().map(|&i| self.lines[i].clone()).collect();
                    let placed = build_paragraphs(ctx, lines, *x0, *x1);
                    for p in &placed {
                        self.analysis.blocks.push((p.rect, p.kind));
                        if let Some(s) = p.score {
                            self.analysis.scores.push((p.rect, s));
                        }
                    }
                    out.extend(placed);
                    let claims = std::mem::take(&mut self.claims[*flow_index]);
                    *flow_index += 1;
                    for c in claims {
                        out.push(self.claimed_block(c, *x0));
                    }
                }
                Zone::Columns { cols } => {
                    self.analysis.zones.push((rect, "columns"));
                    let top = rect.y0;
                    let mut columns = Vec::new();
                    let mut bottom = top;
                    for c in cols {
                        self.analysis.zones.push((Rect::new(c.x0, rect.y0, c.x1, rect.y1), "column"));
                        let placed = self.zones_inner(ctx, &c.zones, c.x0, c.x1, flow_index);
                        bottom = placed.iter().map(|p| p.bottom).fold(bottom, f32::max);
                        columns.push(Column { x: round2(c.x0 - fx0), width: round2(c.x1 - c.x0 + 1.0), blocks: finalize(placed, top) });
                    }
                    let block = Block::Columns(Columns { space_before: 0.0, columns });
                    out.push(Placed { top, bottom, block, rect, kind: "columns", score: None });
                }
                Zone::Table { cols, rows, .. } => {
                    self.analysis.zones.push((rect, "table"));
                    out.push(self.table(ctx, cols, rows, rect, fx0));
                }
            }
        }
        out
    }

    fn claimed_block(&self, c: Claimed, fx0: f32) -> Placed {
        match c {
            Claimed::Rule(r) => {
                let top = r.y - r.thickness * 0.5;
                let rect = Rect::new(r.x0, top, r.x1, top + r.thickness);
                let block = Block::Rule(Rule {
                    space_before: 0.0,
                    x: round2(r.x0 - fx0),
                    width: round2(r.x1 - r.x0),
                    thickness: round2(r.thickness),
                    color: r.color,
                });
                Placed { top, bottom: top + r.thickness, block, rect, kind: "rule", score: None }
            }
            Claimed::Image(i) => {
                let img = &self.page.images[i];
                let block = Block::Image(ImageBlock {
                    space_before: 0.0,
                    x: round2(img.rect.x0 - fx0),
                    width: img.rect.width(),
                    height: img.rect.height(),
                    image: image_resource(img),
                });
                Placed { top: img.rect.y0, bottom: img.rect.y1, block, rect: img.rect, kind: "image", score: None }
            }
        }
    }

    fn table(&mut self, ctx: &mut PageContext, cols: &[(f32, f32)], rows: &[Vec<Vec<usize>>], rect: Rect, fx0: f32) -> Placed {
        // The ruling grid is the set of rules connected to a vertical rule
        // near the text: a wide last column's far border is still found,
        // while an unconnected divider just above the table is not.
        let reach = 1.2 * self.lines[rows[0].iter().flatten().copied().next().unwrap_or(0)].size;
        let area = rect.inflate(reach);
        let (all_v, all_h) = (&self.gfx.vrules, &self.gfx.hrules);
        let mut vsel: Vec<bool> =
            all_v.iter().map(|v| area.contains(document::Point::new(v.x, (v.y0 + v.y1) * 0.5))).collect();
        let mut hsel = vec![false; all_h.len()];
        let touches = |v: &graphics::VRule, h: &HRule| {
            v.x >= h.x0 - 1.0 && v.x <= h.x1 + 1.0 && h.y >= v.y0 - 1.0 && h.y <= v.y1 + 1.0
        };
        loop {
            let mut changed = false;
            for (hi, h) in all_h.iter().enumerate() {
                if !hsel[hi] && all_v.iter().zip(&vsel).any(|(v, &on)| on && touches(v, h)) {
                    hsel[hi] = true;
                    changed = true;
                }
            }
            for (vi, v) in all_v.iter().enumerate() {
                if !vsel[vi] && all_h.iter().zip(&hsel).any(|(h, &on)| on && touches(v, h)) {
                    vsel[vi] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let vr: Vec<_> = all_v.iter().zip(&vsel).filter(|(_, on)| **on).map(|(v, _)| v).collect();
        let mut hr: Vec<&HRule> = all_h.iter().zip(&hsel).filter(|(_, on)| **on).map(|(h, _)| h).collect();
        if vr.is_empty() {
            // Horizontal-only rules (booktabs style) close to the text.
            hr = all_h.iter().filter(|r| r.x0 >= area.x0 && r.x1 <= area.x1 && r.y >= area.y0 && r.y <= area.y1).collect();
        }
        let n = cols.len();

        let mut xs: Vec<f32> = vr.iter().map(|v| v.x).collect();
        xs.sort_by(f32::total_cmp);
        xs.dedup_by(|a, b| (*a - *b).abs() < 2.0);
        let bounds: Vec<f32> = if xs.len() == n + 1 {
            xs
        } else {
            let mut b = vec![cols[0].0];
            for w in cols.windows(2) {
                b.push((w[0].1 + w[1].0) * 0.5);
            }
            b.push(cols[n - 1].1);
            b
        };
        let row_top = |r: &Vec<Vec<usize>>| r.iter().flatten().map(|&i| self.lines[i].top()).fold(f32::MAX, f32::min);
        let row_bottom = |r: &Vec<Vec<usize>>| r.iter().flatten().map(|&i| self.lines[i].bottom()).fold(f32::MIN, f32::max);
        let mut ys: Vec<f32> = hr.iter().map(|r| r.y).collect();
        ys.sort_by(f32::total_cmp);
        ys.dedup_by(|a, b| (*a - *b).abs() < 2.0);
        let tops: Vec<f32> = if ys.len() == rows.len() + 1 {
            ys
        } else {
            let mut t: Vec<f32> = rows.iter().map(row_top).collect();
            t.push(rows.last().map_or(rect.y1, row_bottom));
            t
        };

        let ruled = !hr.is_empty() && !vr.is_empty();
        let mut trows = Vec::new();
        for (ri, row) in rows.iter().enumerate() {
            let mut cells = Vec::new();
            for (ci, ids) in row.iter().enumerate() {
                let (cx0, cx1) = (bounds[ci], bounds[ci + 1]);
                let lines: Vec<Line> = ids.iter().map(|&i| self.lines[i].clone()).collect();
                let pad = lines.iter().map(|l| l.x0 - cx0).fold(f32::MAX, f32::min).max(0.0);
                let right = if ruled { cx1 - pad } else { cx1 };
                let placed = build_paragraphs(ctx, lines, cx0, right.max(cx0 + 1.0));
                let blocks = if placed.is_empty() { vec![self.empty_cell_paragraph(ctx)] } else { finalize(placed, tops[ri]) };
                cells.push(TableCell::new(blocks));
            }
            trows.push(TableRow { min_height: round2(tops[ri + 1] - tops[ri]), cells });
        }
        for r in &hr {
            self.consumed.push(r.source);
        }
        for v in &vr {
            self.consumed.push(v.source);
        }
        let thickness = median(hr.iter().map(|r| r.thickness).collect());
        let borders = ruled.then(|| Border { width: round2(thickness.max(0.25)), color: hr[0].color });
        let block = Block::Table(Table {
            space_before: 0.0,
            x: round2(bounds[0] - fx0),
            col_widths: bounds.windows(2).map(|w| round2(w[1] - w[0])).collect(),
            rows: trows,
            borders,
        });
        Placed { top: tops[0], bottom: *tops.last().expect("tops"), block, rect, kind: "table", score: None }
    }

    /// An empty, editable paragraph for a cell with no text, in body style.
    fn empty_cell_paragraph(&self, ctx: &PageContext) -> Block {
        let mut style = self.lines.iter().find_map(|l| l.glyphs.first()).map(|g| ctx.styles.style(g, 0.0)).unwrap_or_default();
        style.size = ctx.body_size;
        style.font.weight = 400;
        style.font.italic = false;
        style.link = None;
        style.underline = false;
        let pstyle = document::ParagraphStyle {
            indent_left: 5.0,
            indent_right: 5.0,
            space_before: 3.0,
            line_spacing: document::LineSpacing::Multiple(1.2),
            ..Default::default()
        };
        Block::Paragraph(document::Paragraph::new("", style, pstyle))
    }

    /// Everything not represented in the flow becomes an absolute decoration.
    fn decorations(&self) -> Vec<Decoration> {
        let mut out = Vec::new();
        for (i, p) in self.page.paths.iter().enumerate() {
            if self.consumed.contains(&i) {
                continue;
            }
            out.push(Decoration::Path(PathShape { segments: p.segments.clone(), fill: p.fill, stroke: p.stroke }));
        }
        for (i, img) in self.page.images.iter().enumerate() {
            if !self.placed_images.contains(&i) {
                out.push(Decoration::Image { rect: img.rect, image: image_resource(img) });
            }
        }
        out
    }
}

fn collect_tables<'z>(zones: &'z [Zone], out: &mut Vec<&'z Zone>) {
    for z in zones {
        match z {
            Zone::Table { .. } => out.push(z),
            Zone::Columns { cols } => cols.iter().for_each(|c| collect_tables(&c.zones, out)),
            Zone::Flow { .. } => {}
        }
    }
}

fn image_resource(img: &pdf_source::SourceImage) -> Arc<ImageResource> {
    Arc::new(ImageResource::png(img.png.clone(), img.width_px, img.height_px))
}
