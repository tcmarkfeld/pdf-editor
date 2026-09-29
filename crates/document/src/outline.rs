//! Human-readable structural dump of a document, used by debug tooling and
//! reconstruction tests (asserting structure rather than internals).

use std::fmt::Write;

use crate::model::*;

impl Document {
    pub fn outline(&self) -> String {
        let mut s = String::new();
        for (i, sec) in self.sections.iter().enumerate() {
            let m = sec.margins;
            let _ = writeln!(
                s,
                "section {i} {}x{} margins t{} l{} r{} b{} decorations={}",
                sec.page_size.w, sec.page_size.h, m.top, m.left, m.right, m.bottom, sec.decorations.len()
            );
            blocks(&mut s, &sec.blocks, 1);
        }
        s
    }
}

fn blocks(s: &mut String, blocks: &[Block], depth: usize) {
    let pad = "  ".repeat(depth);
    for b in blocks {
        match b {
            Block::Paragraph(p) => {
                let st = &p.style;
                let mut tags = vec![format!("{:?}", st.align).to_lowercase()];
                if let Role::Heading(l) = st.role {
                    tags.push(format!("h{l}"));
                }
                if let Some(l) = &st.list {
                    let marker = match &l.kind {
                        ListKind::Bullet(b) => b.clone(),
                        ListKind::Ordered { start, format, suffix } => format!("{}{suffix}", format.format(*start)),
                    };
                    tags.push(format!("list{}:{}:L{}", l.id, marker, l.level));
                }
                for t in &st.tab_stops {
                    tags.push(format!("tab{:?}@{}", t.align, t.pos));
                }
                if st.indent_left != 0.0 {
                    tags.push(format!("ind{}", st.indent_left));
                }
                let _ = writeln!(s, "{pad}P[{}] sb{} {:?}", tags.join(" "), st.space_before, p.text());
            }
            Block::Columns(c) => {
                let _ = writeln!(s, "{pad}COLUMNS sb{}", c.space_before);
                for col in &c.columns {
                    let _ = writeln!(s, "{pad}  COL x{} w{}", col.x, col.width);
                    self::blocks(s, &col.blocks, depth + 2);
                }
            }
            Block::Table(t) => {
                let _ = writeln!(s, "{pad}TABLE {}x{} borders={} sb{}", t.col_widths.len(), t.rows.len(), t.borders.is_some(), t.space_before);
                for (ri, r) in t.rows.iter().enumerate() {
                    for (ci, c) in r.cells.iter().enumerate() {
                        let _ = writeln!(s, "{pad}  CELL {ri},{ci}");
                        self::blocks(s, &c.blocks, depth + 2);
                    }
                }
            }
            Block::Image(i) => {
                let _ = writeln!(s, "{pad}IMAGE {}x{} sb{}", i.width, i.height, i.space_before);
            }
            Block::Rule(r) => {
                let _ = writeln!(s, "{pad}RULE x{} w{} t{} sb{}", r.x, r.width, r.thickness, r.space_before);
            }
            Block::Frame(f) => {
                let _ = writeln!(s, "{pad}FRAME {:?}", f.rect);
                self::blocks(s, &f.blocks, depth + 1);
            }
        }
    }
}
