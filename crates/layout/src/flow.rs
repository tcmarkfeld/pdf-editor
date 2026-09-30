//! Vertical flow: block stacking, columns, tables, frames and pagination.

use std::collections::HashMap;

use document::{Block, DESCENT_RATIO, Decoration, Paragraph, Rect, Section};
use fonts::FontSystem;
use parley::LayoutContext;

use crate::para::{self, Shaped};
use crate::{Item, ObjectGeom, PageLayout, ParaLayout, Part, RunIndex, SectionLayout, TableGeom, VisualLine};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Pos {
    page: usize,
    y: f32,
}

impl Pos {
    fn max(self, o: Pos) -> Pos {
        if (o.page, o.y) > (self.page, self.y) { o } else { self }
    }
}

/// Horizontal extent and vertical limits of a flow.
#[derive(Clone, Copy)]
struct Area {
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    paginate: bool,
}

struct Ctx<'a> {
    section: &'a Section,
    si: usize,
    pages: Vec<PageLayout>,
    paras: Vec<ParaLayout>,
    fonts: &'a mut FontSystem,
    lcx: &'a mut LayoutContext<RunIndex>,
    /// List counters per list id, one slot per level.
    counters: HashMap<u32, Vec<Option<u32>>>,
    /// Path of the block being laid out (see `document::ParaRef`).
    path: Vec<u32>,
    objects: Vec<ObjectGeom>,
    tables: Vec<TableGeom>,
}

/// `start_y`: where content begins on the first page when the previous
/// section's overflow already occupies its top.
pub fn layout_section(
    section: &Section,
    si: usize,
    start_y: Option<f32>,
    fonts: &mut FontSystem,
    lcx: &mut LayoutContext<RunIndex>,
) -> SectionLayout {
    let mut ctx = Ctx {
        section,
        si,
        pages: Vec::new(),
        paras: Vec::new(),
        fonts,
        lcx,
        counters: HashMap::new(),
        path: Vec::new(),
        objects: Vec::new(),
        tables: Vec::new(),
    };
    ctx.new_page(false);
    for d in &section.decorations {
        let item = match d {
            Decoration::Path(p) => Item::Path(p.clone()),
            Decoration::Image { rect, image } => Item::Image { rect: *rect, image: image.clone() },
        };
        ctx.pages[0].items.push(item);
    }
    ctx.pages[0].decorations = ctx.pages[0].items.len();
    let m = section.margins;
    let size = section.page_size;
    let area = Area {
        x: m.left,
        w: (size.w - m.left - m.right).max(1.0),
        top: m.top,
        bottom: (size.h - m.bottom).max(m.top + 1.0),
        paginate: true,
    };
    let mut pos = Pos { page: 0, y: start_y.map_or(m.top, |y| y.max(m.top)) };
    ctx.blocks(&section.blocks, area, &mut pos);
    let end = (pos.page, pos.y);
    SectionLayout { pages: ctx.pages, paras: ctx.paras, objects: ctx.objects, tables: ctx.tables, end }
}

impl Ctx<'_> {
    fn new_page(&mut self, continuation: bool) -> usize {
        self.pages.push(PageLayout { section: self.si, decorations: 0, size: self.section.page_size, items: Vec::new(), continuation });
        self.pages.len() - 1
    }

    fn next_page(&mut self, page: usize) -> usize {
        if page + 1 < self.pages.len() { page + 1 } else { self.new_page(true) }
    }

    /// Moves to the next page if `h` points do not fit below `pos`.
    fn ensure(&mut self, area: Area, pos: &mut Pos, h: f32) {
        if area.paginate && pos.y + h > area.bottom + 0.5 && pos.y > area.top + 0.5 {
            pos.page = self.next_page(pos.page);
            pos.y = area.top;
        }
    }

    fn blocks(&mut self, blocks: &[Block], area: Area, pos: &mut Pos) {
        for (bi, b) in blocks.iter().enumerate() {
            self.path.push(bi as u32);
            match b {
                Block::Paragraph(p) => self.paragraph(p, area, pos),
                Block::Rule(r) => {
                    pos.y += r.space_before;
                    self.ensure(area, pos, r.thickness);
                    let w = if r.width > 0.0 { r.width } else { area.w - r.x };
                    let rect = Rect::from_xywh(area.x + r.x, pos.y, w, r.thickness);
                    self.pages[pos.page].items.push(Item::Rect { rect, color: r.color });
                    pos.y += r.thickness;
                }
                Block::Image(img) => {
                    pos.y += img.space_before;
                    self.ensure(area, pos, img.height);
                    let rect = Rect::from_xywh(area.x + img.x, pos.y, img.width, img.height);
                    self.pages[pos.page].items.push(Item::Image { rect, image: img.image.clone() });
                    self.objects.push(ObjectGeom { page: pos.page, rect, path: self.path.clone() });
                    pos.y += img.height;
                }
                Block::Columns(c) => {
                    pos.y += c.space_before;
                    let start = *pos;
                    let mut end = start;
                    for (ci, col) in c.columns.iter().enumerate() {
                        let mut p = start;
                        self.path.push(ci as u32);
                        self.blocks(&col.blocks, Area { x: area.x + col.x, w: col.width, ..area }, &mut p);
                        self.path.pop();
                        end = end.max(p);
                    }
                    *pos = end;
                }
                Block::Table(t) => self.table(t, area, pos),
                Block::Frame(f) => {
                    let mut p = Pos { page: 0, y: f.rect.y0 };
                    let sub = Area { x: f.rect.x0, w: f.rect.width(), top: f.rect.y0, bottom: f32::MAX, paginate: false };
                    self.path.push(0);
                    self.blocks(&f.blocks, sub, &mut p);
                    self.path.pop();
                }
            }
            self.path.pop();
        }
    }

    /// Rows are laid out in order; a cell spanning rows contributes its
    /// height to the last row it covers. Shading and borders are drawn once
    /// every row's extent is known.
    fn table(&mut self, t: &document::Table, area: Area, pos: &mut Pos) {
        pos.y += t.space_before;
        let ncols = t.col_widths.len();
        let mut xs = vec![area.x + t.x];
        for w in &t.col_widths {
            xs.push(xs.last().copied().unwrap_or(0.0) + w);
        }
        // (page, top, bottom) per row, and item index for shading per row.
        let mut rows: Vec<(usize, f32, f32)> = Vec::with_capacity(t.rows.len());
        let mut shade_at: Vec<usize> = Vec::with_capacity(t.rows.len());
        // Content end of cells spanning rows, keyed by their last row.
        let mut span_ends: Vec<(usize, Pos)> = Vec::new();
        for (ri, row) in t.rows.iter().enumerate() {
            self.ensure(area, pos, row.min_height);
            let start = *pos;
            shade_at.push(self.pages[start.page].items.len());
            let mut end = Pos { page: start.page, y: start.y + row.min_height };
            for (ci, cell) in row.cells.iter().enumerate().take(ncols) {
                if cell.merged {
                    continue;
                }
                let cs = (cell.col_span.max(1) as usize).min(ncols - ci);
                let rs = (cell.row_span.max(1) as usize).min(t.rows.len() - ri);
                let mut p = start;
                self.path.push((ri * ncols + ci) as u32);
                self.blocks(&cell.blocks, Area { x: xs[ci], w: xs[ci + cs] - xs[ci], ..area }, &mut p);
                self.path.pop();
                if rs == 1 { end = end.max(p) } else { span_ends.push((ri + rs - 1, p)) }
            }
            for (_, p) in span_ends.iter().filter(|(last, _)| *last == ri) {
                end = end.max(*p);
            }
            rows.push((start.page, start.y, end.y));
            *pos = end;
        }

        self.tables.push(TableGeom { path: self.path.clone(), cols: xs.clone(), rows: rows.clone() });
        for (ri, row) in t.rows.iter().enumerate() {
            for (ci, cell) in row.cells.iter().enumerate().take(ncols) {
                if cell.merged {
                    continue;
                }
                let cs = (cell.col_span.max(1) as usize).min(ncols - ci);
                let rs = (cell.row_span.max(1) as usize).min(t.rows.len() - ri);
                let (page, top, _) = rows[ri];
                let (last_page, _, bottom) = rows[ri + rs - 1];
                if page != last_page {
                    continue; // split across pages: skip decoration
                }
                let r = Rect::new(xs[ci], top, xs[ci + cs], bottom);
                if let Some(color) = cell.shading {
                    self.pages[page].items.insert(shade_at[ri], Item::Rect { rect: r, color });
                    for rj in ri + 1..rows.len() {
                        if rows[rj].0 == page {
                            shade_at[rj] += 1;
                        }
                    }
                }
                if let Some(b) = t.borders {
                    let h = b.width * 0.5;
                    for edge in [
                        Rect::new(r.x0 - h, r.y0 - h, r.x1 + h, r.y0 + h),
                        Rect::new(r.x0 - h, r.y1 - h, r.x1 + h, r.y1 + h),
                        Rect::new(r.x0 - h, r.y0 - h, r.x0 + h, r.y1 + h),
                        Rect::new(r.x1 - h, r.y0 - h, r.x1 + h, r.y1 + h),
                    ] {
                        self.pages[page].items.push(Item::Rect { rect: edge, color: b.color });
                    }
                }
            }
        }
    }

    fn paragraph(&mut self, p: &Paragraph, area: Area, pos: &mut Pos) {
        let shaped = para::shape(p, area.x, area.w, self.fonts, self.lcx);
        pos.y += p.style.space_before;
        let marker = p.style.list.as_ref().map(|l| {
            let slots = self.counters.entry(l.id).or_default();
            let lv = l.level as usize;
            slots.truncate(lv + 1);
            slots.resize(lv + 1, None);
            let start = match &l.kind {
                document::ListKind::Ordered { start, .. } => *start,
                _ => 1,
            };
            let n = slots[lv].map_or(start, |v| v + 1);
            slots[lv] = Some(n);
            (para::marker_text(&l.kind, n), l)
        });

        let mut lines = Vec::with_capacity(shaped.line_count);
        for k in 0..shaped.line_count {
            let s = shaped.line_sizes[k];
            let pitch = p.style.line_spacing.pitch(s);
            self.ensure(area, pos, pitch);
            let top = pos.y;
            let baseline = top + pitch - DESCENT_RATIO * s;
            pos.y += pitch;
            let items = &mut self.pages[pos.page].items;
            para::emit_line(p, &shaped, k, |lb| baseline - lb, items);
            if k == 0
                && let Some((text, info)) = &marker
            {
                let x = shaped.content_x + p.style.first_line_indent.max(0.0) + info.marker_offset;
                para::emit_marker(text, &info.marker_style, x, baseline, self.fonts, self.lcx, &mut self.pages[pos.page].items);
            }
            lines.push(visual_line(&shaped, k, pos.page, top, pos.y, baseline, p.style.align));
        }
        self.paras.push(ParaLayout {
            segments: shaped.segments,
            lines,
            content_x: shaped.content_x,
            width: shaped.width,
            align: p.style.align,
        });
    }
}

fn visual_line(s: &Shaped, k: usize, page: usize, top: f32, bottom: f32, baseline: f32, align: document::Align) -> VisualLine {
    let mut parts = Vec::new();
    let (mut x0, mut x1) = (f32::MAX, f32::MIN);
    let (mut t0, mut t1) = (usize::MAX, 0);
    for (si, seg) in s.segments.iter().enumerate() {
        let Some(line) = seg.layout.get(k) else { continue };
        let m = line.metrics();
        let dx = s.content_x + s.seg_x[si];
        parts.push(Part { seg: si, line: k, dx, dy: baseline - m.baseline });
        x0 = x0.min(dx + m.offset);
        x1 = x1.max(dx + m.offset + m.advance - m.trailing_whitespace);
        let r = line.text_range();
        t0 = t0.min(seg.start + r.start);
        t1 = t1.max(seg.start + r.end);
    }
    if parts.is_empty() {
        let x = match align {
            document::Align::Center => s.content_x + s.width * 0.5,
            document::Align::Right => s.content_x + s.width,
            _ => s.content_x,
        };
        (x0, x1) = (x, x);
        (t0, t1) = (s.text.len(), s.text.len());
    }
    VisualLine { page, top, bottom, baseline, x0, x1, parts, text: t0..t1 }
}
