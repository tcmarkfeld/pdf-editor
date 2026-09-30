//! Table creation and structural editing.

use document::{
    Block, Border, LineSpacing, Paragraph, ParagraphStyle, Rgba, Table, TableCell, TableRow, TextStyle,
};

use crate::{EditKind, Editor, Pos, Selection};

/// Where the caret sits inside a table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCursor {
    pub section: usize,
    /// Path of the table block.
    pub path: Vec<u32>,
    pub row: usize,
    pub col: usize,
    pub rows: usize,
    pub cols: usize,
}

const CELL_PAD: f32 = 5.0;
const DEFAULT_BORDER: Border = Border { width: 0.75, color: Rgba([120, 120, 120, 255]) };

fn cell_paragraph(style: &TextStyle) -> Paragraph {
    let pstyle = ParagraphStyle {
        indent_left: CELL_PAD,
        indent_right: CELL_PAD,
        space_before: 3.0,
        line_spacing: LineSpacing::Multiple(1.2),
        ..Default::default()
    };
    let mut s = style.clone();
    s.link = None;
    Paragraph::new("", s, pstyle)
}

fn new_cell(style: &TextStyle) -> TableCell {
    TableCell::new(vec![Block::Paragraph(cell_paragraph(style))])
}

fn row_height(style: &TextStyle) -> f32 {
    style.size * 1.2 + 6.0
}

/// Style of the first text in a cell, used for cells created next to it.
fn cell_style(cell: &TableCell) -> TextStyle {
    cell.blocks
        .iter()
        .find_map(|b| if let Block::Paragraph(p) = b { Some(p.style_at(0).clone()) } else { None })
        .unwrap_or_default()
}

impl Editor {
    pub fn table_at_caret(&self) -> Option<TableCursor> {
        self.table_cursor(self.sel.focus.para)
    }

    /// The innermost table containing paragraph `para`, and its cell.
    fn table_cursor(&self, para: usize) -> Option<TableCursor> {
        let r = self.paras.get(para)?;
        // The innermost table enclosing the caret paragraph.
        let mut k = r.path.len().checked_sub(3)?;
        loop {
            if k % 2 == 0
                && let Some(Block::Table(t)) = self.doc.block(r.section, &r.path[..=k])
            {
                let cols = t.col_widths.len().max(1);
                let cell = r.path[k + 1] as usize;
                return Some(TableCursor { section: r.section, path: r.path[..=k].to_vec(), row: cell / cols, col: cell % cols, rows: t.rows.len(), cols });
            }
            k = k.checked_sub(1)?;
        }
    }

    fn table_mut(&mut self, c: &TableCursor) -> Option<&mut Table> {
        match self.doc.block_mut(c.section, &c.path)? {
            Block::Table(t) => Some(t),
            _ => None,
        }
    }

    /// Flat paragraph index of the first paragraph in a cell.
    fn cell_para(&self, c: &TableCursor, row: usize, col: usize) -> Option<usize> {
        let mut path = c.path.clone();
        path.extend([(row * c.cols + col) as u32, 0]);
        self.paras.iter().position(|r| r.section == c.section && r.path.starts_with(&path))
    }

    fn caret_to_cell(&mut self, c: &TableCursor, row: usize, col: usize) {
        self.paras = self.doc.paragraphs();
        let fresh = TableCursor { cols: c.cols.max(1), ..c.clone() };
        if let Some(i) = self.cell_para(&fresh, row, col) {
            self.sel = Selection::caret(Pos::new(i, 0));
        }
    }

    /// Inserts a `rows` × `cols` table next to the caret's paragraph,
    /// spanning `width`.
    pub fn insert_table(&mut self, rows: usize, cols: usize, width: f32) {
        if self.paras.is_empty() || rows == 0 || cols == 0 {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.delete_selection_inner();
        let p = self.sel.focus;
        let para = self.paragraph(p.para);
        let len = para.len();
        let style = para.style_at(p.offset).clone();
        let body = { let mut s = style.clone(); s.link = None; s.underline = false; s };
        // Insert below the caret's paragraph (above it when the caret is at
        // its very start); never split a paragraph mid-sentence.
        let r = self.paras[self.sel.focus.para].clone();
        let at_end = p.offset > 0 && len > 0;
        let table = Table {
            space_before: 6.0,
            x: 0.0,
            col_widths: vec![width / cols as f32; cols],
            rows: (0..rows)
                .map(|_| TableRow { min_height: row_height(&body), cells: (0..cols).map(|_| new_cell(&body)).collect() })
                .collect(),
            borders: Some(DEFAULT_BORDER),
        };
        let Some((flow, idx)) = self.doc.flow_mut(&r) else { return };
        let at = if at_end { idx + 1 } else { idx };
        flow.insert(at, Block::Table(table));
        // Always leave a paragraph after the table to continue typing in.
        if !matches!(flow.get(at + 1), Some(Block::Paragraph(_))) {
            let mut after = Paragraph::new("", body, ParagraphStyle::default());
            after.style.space_before = 6.0;
            flow.insert(at + 1, Block::Paragraph(after));
        }
        let mut path = r.path.clone();
        *path.last_mut().expect("path") = at as u32;
        let c = TableCursor { section: r.section, path, row: 0, col: 0, rows, cols };
        self.caret_to_cell(&c, 0, 0);
        self.refresh();
    }

    pub fn table_insert_row(&mut self, below: bool) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        let at = if below { c.row + 1 } else { c.row };
        // Merged cells crossing the new row's position are split first.
        self.split_spans(&c, |r0, r1, _, _| r0 < at && at <= r1);
        if let Some(t) = self.table_mut(&c) {
            let src = &t.rows[c.row];
            let row = TableRow { min_height: src.min_height, cells: src.cells.iter().map(|cell| new_cell(&cell_style(cell))).collect() };
            t.rows.insert(at, row);
        }
        self.caret_to_cell(&TableCursor { rows: c.rows + 1, ..c.clone() }, at, c.col);
        self.refresh();
    }

    pub fn table_insert_col(&mut self, right: bool) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        let at = if right { c.col + 1 } else { c.col };
        self.split_spans(&c, |_, _, c0, c1| c0 < at && at <= c1);
        if let Some(t) = self.table_mut(&c) {
            for row in &mut t.rows {
                let style = cell_style(&row.cells[c.col]);
                row.cells.insert(at, new_cell(&style));
            }
            // Keep the table width: the new column takes half of the current one.
            let half = t.col_widths[c.col] * 0.5;
            t.col_widths[c.col] = half;
            t.col_widths.insert(at, half);
        }
        self.caret_to_cell(&TableCursor { cols: c.cols + 1, ..c.clone() }, c.row, at);
        self.refresh();
    }

    pub fn table_delete_row(&mut self) {
        let Some(c) = self.table_at_caret() else { return };
        if c.rows <= 1 {
            return self.table_delete();
        }
        self.checkpoint(EditKind::Other);
        self.split_spans(&c, |r0, r1, _, _| r0 <= c.row && c.row <= r1);
        if let Some(t) = self.table_mut(&c) {
            t.rows.remove(c.row);
        }
        self.caret_to_cell(&TableCursor { rows: c.rows - 1, ..c.clone() }, c.row.min(c.rows - 2), c.col);
        self.refresh();
    }

    pub fn table_delete_col(&mut self) {
        let Some(c) = self.table_at_caret() else { return };
        if c.cols <= 1 {
            return self.table_delete();
        }
        self.checkpoint(EditKind::Other);
        self.split_spans(&c, |_, _, c0, c1| c0 <= c.col && c.col <= c1);
        if let Some(t) = self.table_mut(&c) {
            for row in &mut t.rows {
                row.cells.remove(c.col);
            }
            let w = t.col_widths.remove(c.col);
            let neighbour = c.col.saturating_sub(1).min(t.col_widths.len() - 1);
            t.col_widths[neighbour] += w;
        }
        self.caret_to_cell(&TableCursor { cols: c.cols - 1, ..c.clone() }, c.row, c.col.min(c.cols - 2));
        self.refresh();
    }

    pub fn table_delete(&mut self) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        let idx = *c.path.last().expect("path") as usize;
        let body = self.body_style();
        if let Some(flow) = self.doc.flow_of_mut(c.section, &c.path) {
            flow.remove(idx);
            if !matches!(flow.get(idx), Some(Block::Paragraph(_))) {
                flow.insert(idx, Block::Paragraph(Paragraph::new("", body, ParagraphStyle::default())));
            }
        }
        self.paras = self.doc.paragraphs();
        let mut path = c.path.clone();
        *path.last_mut().expect("path") = idx as u32;
        if let Some(i) = self.paras.iter().position(|r| r.section == c.section && r.path == path) {
            self.sel = Selection::caret(Pos::new(i, 0));
        }
        self.refresh();
    }

    pub fn table_toggle_borders(&mut self) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        if let Some(t) = self.table_mut(&c) {
            t.borders = if t.borders.is_some() { None } else { Some(DEFAULT_BORDER) };
        }
        self.refresh();
    }

    pub fn table_borders(&self) -> Option<Border> {
        let c = self.table_at_caret()?;
        match self.doc.block(c.section, &c.path)? {
            Block::Table(t) => t.borders,
            _ => None,
        }
    }

    pub fn table_set_border_color(&mut self, color: Rgba) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        if let Some(t) = self.table_mut(&c) {
            let b = t.borders.get_or_insert(DEFAULT_BORDER);
            b.color = color;
        }
        self.refresh();
    }

    /// Shades the caret's cell, or its whole row when `row` is set.
    pub fn table_set_shading(&mut self, color: Option<Rgba>, row: bool) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        if let Some(t) = self.table_mut(&c) {
            let cells = &mut t.rows[c.row].cells;
            if row {
                cells.iter_mut().for_each(|cell| cell.shading = color);
            } else {
                cells[c.col].shading = color;
            }
        }
        self.refresh();
    }

    pub fn table_distribute_columns(&mut self) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        if let Some(t) = self.table_mut(&c) {
            let total: f32 = t.col_widths.iter().sum();
            let n = t.col_widths.len() as f32;
            t.col_widths.iter_mut().for_each(|w| *w = total / n);
        }
        self.refresh();
    }

    /// Tab / Shift+Tab inside a table: select the next/previous cell's
    /// text; Tab in the last cell adds a row. Returns false outside tables.
    pub fn table_next_cell(&mut self, backwards: bool) -> bool {
        let Some(c) = self.table_at_caret() else { return false };
        let flat = c.row * c.cols + c.col;
        let target = if backwards {
            match flat.checked_sub(1) {
                Some(t) => t,
                None => return true,
            }
        } else if flat + 1 == c.rows * c.cols {
            self.table_insert_row(true);
            let c = TableCursor { rows: c.rows + 1, ..c };
            self.caret_to_cell(&c, c.rows - 1, 0);
            return true;
        } else {
            flat + 1
        };
        // Covered (merged) cells hold no paragraphs: skip over them.
        let total = c.rows * c.cols;
        let mut t = target;
        loop {
            if let Some(i) = self.cell_para(&c, t / c.cols, t % c.cols) {
                self.break_coalescing();
                let len = self.paragraph(i).len();
                self.sel = Selection { anchor: Pos::new(i, 0), focus: Pos::new(i, len) };
                return true;
            }
            t = if backwards { match t.checked_sub(1) { Some(x) => x, None => return true } } else { t + 1 };
            if t >= total {
                return true;
            }
        }
    }

    /// Cell rectangle (rows, cols; inclusive) covered by the selection when
    /// both ends are in the same table, grown to include whole merged cells.
    #[allow(clippy::type_complexity)]
    fn selected_cell_rect(&self) -> Option<(TableCursor, (usize, usize), (usize, usize))> {
        let (a, b) = self.sel.ordered();
        let (ca, cb) = (self.table_cursor(a.para)?, self.table_cursor(b.para)?);
        if ca.section != cb.section || ca.path != cb.path {
            return None;
        }
        let Some(Block::Table(t)) = self.doc.block(ca.section, &ca.path) else { return None };
        let (mut r0, mut r1) = (ca.row.min(cb.row), ca.row.max(cb.row));
        let (mut c0, mut c1) = (ca.col.min(cb.col), ca.col.max(cb.col));
        loop {
            let before = (r0, r1, c0, c1);
            for (ri, row) in t.rows.iter().enumerate() {
                for (ci, cell) in row.cells.iter().enumerate() {
                    if cell.merged {
                        continue;
                    }
                    let (er, ec) = (ri + cell.row_span.max(1) as usize - 1, ci + cell.col_span.max(1) as usize - 1);
                    let overlaps = ri <= r1 && er >= r0 && ci <= c1 && ec >= c0;
                    if overlaps {
                        (r0, r1, c0, c1) = (r0.min(ri), r1.max(er), c0.min(ci), c1.max(ec));
                    }
                }
            }
            if before == (r0, r1, c0, c1) {
                return Some((ca, (r0, r1), (c0, c1)));
            }
        }
    }

    /// Whether the selection spans several cells that could be merged.
    pub fn can_merge_cells(&self) -> bool {
        self.selected_cell_rect().is_some_and(|(_, (r0, r1), (c0, c1))| r0 != r1 || c0 != c1)
    }

    /// Whether the caret's cell is a merged cell that can be split.
    pub fn can_split_cell(&self) -> bool {
        self.table_at_caret().is_some_and(|c| {
            matches!(self.doc.block(c.section, &c.path), Some(Block::Table(t)) if {
                let cell = &t.rows[c.row].cells[c.col];
                cell.col_span > 1 || cell.row_span > 1
            })
        })
    }

    /// Merges the selected cells into the top-left one, keeping all text.
    pub fn table_merge_cells(&mut self) {
        let Some((c, (r0, r1), (c0, c1))) = self.selected_cell_rect() else { return };
        if r0 == r1 && c0 == c1 {
            return;
        }
        self.checkpoint(EditKind::Other);
        if let Some(t) = self.table_mut(&c) {
            let mut moved = Vec::new();
            for r in r0..=r1 {
                for col in c0..=c1 {
                    if (r, col) == (r0, c0) {
                        continue;
                    }
                    let cell = &mut t.rows[r].cells[col];
                    let has_text = cell.blocks.iter().any(|b| !matches!(b, Block::Paragraph(p) if p.is_empty()));
                    let blocks = std::mem::take(&mut cell.blocks);
                    if has_text {
                        moved.extend(blocks);
                    }
                    *cell = TableCell { merged: true, ..TableCell::new(Vec::new()) };
                }
            }
            let owner = &mut t.rows[r0].cells[c0];
            owner.blocks.extend(moved);
            owner.col_span = (c1 - c0 + 1) as u16;
            owner.row_span = (r1 - r0 + 1) as u16;
        }
        self.caret_to_cell(&c, r0, c0);
        self.refresh();
    }

    /// Splits the caret's merged cell back into single cells.
    pub fn table_split_cell(&mut self) {
        let Some(c) = self.table_at_caret() else { return };
        self.checkpoint(EditKind::Other);
        let (r, col) = (c.row, c.col);
        self.split_spans(&c, |r0, _, c0, _| (r0, c0) == (r, col));
        self.caret_to_cell(&c, r, col);
        self.refresh();
    }

    /// Splits every merged cell whose extent `(r0, r1, c0, c1)` matches.
    fn split_spans(&mut self, c: &TableCursor, matches: impl Fn(usize, usize, usize, usize) -> bool) {
        let Some(t) = self.table_mut(c) else { return };
        let mut owners = Vec::new();
        for (ri, row) in t.rows.iter().enumerate() {
            for (ci, cell) in row.cells.iter().enumerate() {
                let (rs, cs) = (cell.row_span.max(1) as usize, cell.col_span.max(1) as usize);
                if !cell.merged && (rs > 1 || cs > 1) && matches(ri, ri + rs - 1, ci, ci + cs - 1) {
                    owners.push((ri, ci, rs, cs));
                }
            }
        }
        for (ri, ci, rs, cs) in owners {
            let style = cell_style(&t.rows[ri].cells[ci]);
            let shading = t.rows[ri].cells[ci].shading;
            for r in ri..ri + rs {
                for col in ci..ci + cs {
                    let cell = &mut t.rows[r].cells[col];
                    if (r, col) == (ri, ci) {
                        cell.row_span = 1;
                        cell.col_span = 1;
                    } else {
                        *cell = TableCell { shading, ..new_cell(&style) };
                    }
                }
            }
        }
    }

    /// Sets column widths of the table at `path` (e.g. after dragging a
    /// column border).
    pub fn table_set_col_widths(&mut self, section: usize, path: &[u32], widths: Vec<f32>) {
        self.checkpoint(EditKind::Other);
        if let Some(Block::Table(t)) = self.doc.block_mut(section, path)
            && t.col_widths.len() == widths.len()
        {
            t.col_widths = widths;
        }
        self.refresh();
    }
}
