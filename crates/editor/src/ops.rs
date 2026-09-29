//! Document-mutating editor commands.

use document::{Block, Role, TextRun, TextStyle};

use crate::{EditKind, Editor, Pos, Selection, next_grapheme, prev_grapheme, prev_word, next_word};

impl Editor {
    /// Types or pastes plain text at the selection. '\n' splits paragraphs.
    pub fn insert_text(&mut self, text: &str) {
        if text.is_empty() || self.paras.is_empty() {
            return;
        }
        let kind = if text.contains('\n') || !self.sel.is_collapsed() { EditKind::Other } else { EditKind::Typing };
        self.checkpoint(kind);
        self.delete_selection_inner();
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.split_inner();
            }
            let line = line.trim_end_matches('\r');
            let p = self.sel.focus;
            self.paragraph_mut(p.para).insert(p.offset, line, None);
            self.sel = Selection::caret(Pos::new(p.para, p.offset + line.len()));
        }
        self.refresh();
        if kind == EditKind::Typing {
            self.last_edit = Some(EditKind::Typing);
        }
    }

    /// Pastes, preserving styles when the system clipboard still holds what
    /// was copied from this editor.
    pub fn paste(&mut self, system_text: &str) {
        let Some(clip) = self.clipboard.clone().filter(|c| c.plain == system_text.replace("\r\n", "\n")) else {
            return self.insert_text(system_text);
        };
        self.checkpoint(EditKind::Other);
        self.delete_selection_inner();
        for (i, runs) in clip.paragraphs.iter().enumerate() {
            if i > 0 {
                self.split_inner();
            }
            let mut p = self.sel.focus;
            for run in runs {
                self.paragraph_mut(p.para).insert(p.offset, &run.text, Some(&run.style));
                p.offset += run.text.len();
            }
            self.sel = Selection::caret(p);
        }
        self.refresh();
    }

    /// Enter: split the paragraph at the caret.
    pub fn enter(&mut self) {
        if self.paras.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        self.delete_selection_inner();
        let p = self.sel.focus;
        let para = self.paragraph(p.para);
        if para.is_empty() && para.style.list.is_some() {
            // Enter on an empty list item leaves the list (word-processor behaviour).
            self.paragraph_mut(p.para).style.list = None;
        } else {
            self.split_inner();
        }
        self.refresh();
    }

    fn split_inner(&mut self) {
        let p = self.sel.focus;
        let r = self.paras[p.para].clone();
        let para = self.paragraph_mut(p.para);
        let after_heading = p.offset == para.len() && matches!(para.style.role, Role::Heading(_));
        let mut tail = para.split_off(p.offset);
        if let Some((flow, idx)) = self.doc.flow_mut(&r) {
            if after_heading {
                tail.style.role = Role::Body;
                // Continue in the style of the body text below the heading.
                if let Some(Block::Paragraph(next)) = flow.get(idx + 1)
                    && next.style.role == Role::Body
                {
                    tail.runs = vec![TextRun { text: String::new(), style: next.runs[0].style.clone() }];
                    tail.style = next.style.clone();
                    tail.style.space_before = 0.0;
                }
            }
            flow.insert(idx + 1, Block::Paragraph(tail));
        }
        self.paras = self.doc.paragraphs();
        self.sel = Selection::caret(Pos::new(p.para + 1, 0));
    }

    pub fn backspace(&mut self, word: bool) {
        if self.paras.is_empty() {
            return;
        }
        if !self.sel.is_collapsed() {
            self.checkpoint(EditKind::Other);
            self.delete_selection_inner();
            return self.refresh();
        }
        let p = self.sel.focus;
        if p.offset > 0 {
            let text = self.text(p.para);
            let from = if word { prev_word(&text, p.offset) } else { prev_grapheme(&text, p.offset) }.unwrap_or(0);
            self.checkpoint(EditKind::Deleting);
            self.paragraph_mut(p.para).delete(from..p.offset);
            self.sel = Selection::caret(Pos::new(p.para, from));
            self.last_edit = Some(EditKind::Deleting);
            return self.refresh();
        }
        if self.paragraph(p.para).style.list.is_some() {
            // First backspace at the start of a list item removes its marker.
            self.checkpoint(EditKind::Other);
            self.paragraph_mut(p.para).style.list = None;
            return self.refresh();
        }
        if p.para == 0 {
            return;
        }
        self.checkpoint(EditKind::Other);
        let cur = self.paras[p.para].clone();
        let prev = self.paras[p.para - 1].clone();
        if prev.is_next_sibling(&cur) {
            let len = self.paragraph(p.para - 1).len();
            self.merge_with_next(p.para - 1);
            self.sel = Selection::caret(Pos::new(p.para - 1, len));
        } else if let Some((flow, idx)) = self.doc.flow_mut(&cur)
            && idx > 0
            && matches!(flow[idx - 1], Block::Rule(_) | Block::Image(_))
        {
            flow.remove(idx - 1);
            self.sel = Selection::caret(Pos::new(p.para, 0));
        } else {
            self.sel = Selection::caret(Pos::new(p.para - 1, self.paragraph(p.para - 1).len()));
        }
        self.refresh();
    }

    pub fn delete_forward(&mut self, word: bool) {
        if self.paras.is_empty() {
            return;
        }
        if !self.sel.is_collapsed() {
            self.checkpoint(EditKind::Other);
            self.delete_selection_inner();
            return self.refresh();
        }
        let p = self.sel.focus;
        let text = self.text(p.para);
        if p.offset < text.len() {
            let to = if word { next_word(&text, p.offset) } else { next_grapheme(&text, p.offset) }.unwrap_or(text.len());
            self.checkpoint(EditKind::Deleting);
            self.paragraph_mut(p.para).delete(p.offset..to);
            self.last_edit = Some(EditKind::Deleting);
            return self.refresh();
        }
        if p.para + 1 >= self.paras.len() {
            return;
        }
        self.checkpoint(EditKind::Other);
        let cur = self.paras[p.para].clone();
        if cur.is_next_sibling(&self.paras[p.para + 1]) {
            self.merge_with_next(p.para);
        } else if let Some((flow, idx)) = self.doc.flow_mut(&cur)
            && matches!(flow.get(idx + 1), Some(Block::Rule(_) | Block::Image(_)))
        {
            flow.remove(idx + 1);
        }
        self.refresh();
    }

    /// Appends paragraph `i + 1` (its next sibling) to paragraph `i`.
    fn merge_with_next(&mut self, i: usize) {
        let next = self.paras[i + 1].clone();
        let Some((flow, idx)) = self.doc.flow_mut(&next) else { return };
        let Block::Paragraph(tail) = flow.remove(idx) else { unreachable!("paragraph ref points at a paragraph") };
        self.paragraph_mut(i).append(tail);
        self.paras = self.doc.paragraphs();
    }

    /// Removes the selected content, leaving a collapsed caret. Paragraphs
    /// in the same flow merge; across flows (columns, cells) only text goes.
    pub(crate) fn delete_selection_inner(&mut self) {
        let (a, b) = self.sel.ordered();
        self.sel = Selection::caret(a);
        if a == b {
            return;
        }
        if a.para == b.para {
            self.paragraph_mut(a.para).delete(a.offset..b.offset);
            return;
        }
        let ra = self.paras[a.para].clone();
        let rb = self.paras[b.para].clone();
        if ra.same_flow(&rb) {
            self.paragraph_mut(b.para).delete(0..b.offset);
            let len_a = self.paragraph(a.para).len();
            self.paragraph_mut(a.para).delete(a.offset..len_a);
            let (ia, ib) = (*ra.path.last().expect("path") as usize, *rb.path.last().expect("path") as usize);
            if let Some((flow, _)) = self.doc.flow_mut(&ra) {
                let tail = match flow.remove(ib) {
                    Block::Paragraph(p) => p,
                    _ => unreachable!("paragraph ref points at a paragraph"),
                };
                flow.drain(ia + 1..ib);
                if let Block::Paragraph(pa) = &mut flow[ia] {
                    pa.append(tail);
                }
            }
            self.paras = self.doc.paragraphs();
        } else {
            for i in a.para..=b.para {
                let len = self.paragraph(i).len();
                let from = if i == a.para { a.offset } else { 0 };
                let to = if i == b.para { b.offset } else { len };
                self.paragraph_mut(i).delete(from..to);
            }
        }
    }

    /// Toggles a character style over the selection (Cmd+B / I / U).
    pub fn toggle_style(&mut self, get: fn(&TextStyle) -> bool, set: fn(&mut TextStyle, bool)) {
        if self.sel.is_collapsed() {
            return;
        }
        let (a, b) = self.sel.ordered();
        let all_on = (a.para..=b.para).all(|i| {
            let p = self.paragraph(i);
            let from = if i == a.para { a.offset } else { 0 };
            let to = if i == b.para { b.offset } else { p.len() };
            p.slice(from..to).iter().all(|r| get(&r.style))
        });
        self.checkpoint(EditKind::Other);
        for i in a.para..=b.para {
            let len = self.paragraph(i).len();
            let from = if i == a.para { a.offset } else { 0 };
            let to = if i == b.para { b.offset } else { len };
            self.paragraph_mut(i).restyle(from..to, |s| set(s, !all_on));
        }
        self.refresh();
    }
}
