//! Editing engine: selections, text operations, navigation and undo/redo on
//! the document model. Rendering-agnostic; navigation that depends on line
//! geometry takes a `DocLayout` for the current document.
//!
//! Positions are `(flat paragraph index, byte offset)` over the reading-order
//! paragraph list, so ordering and ranges are trivial comparisons.
//!
//! Undo is snapshot based: a snapshot is a `Vec<Arc<Section>>` clone, and
//! edits copy only the section they touch (`Arc::make_mut`), so history is
//! cheap even for long documents. Consecutive typing coalesces into one step.

mod ops;

use std::sync::Arc;

use document::{Document, ParaRef, Paragraph, Section, TextRun};
use layout::DocLayout;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub para: usize,
    pub offset: usize,
}

impl Pos {
    pub const fn new(para: usize, offset: usize) -> Self {
        Self { para, offset }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Pos,
    pub focus: Pos,
}

impl Selection {
    pub fn caret(p: Pos) -> Self {
        Self { anchor: p, focus: p }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.focus { (self.anchor, self.focus) } else { (self.focus, self.anchor) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    LineStart,
    LineEnd,
    ParaStart,
    ParaEnd,
    DocStart,
    DocEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Clone)]
struct Snapshot {
    sections: Vec<Arc<Section>>,
    sel: Selection,
}

/// Styled clipboard content kept alongside the plain-text system clipboard.
#[derive(Clone, Debug)]
pub struct RichClip {
    pub plain: String,
    pub paragraphs: Vec<Vec<TextRun>>,
}

pub struct Editor {
    pub doc: Document,
    pub sel: Selection,
    paras: Vec<ParaRef>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: Option<EditKind>,
    goal_x: Option<f32>,
    pub clipboard: Option<RichClip>,
    /// Incremented on every document change.
    pub revision: u64,
}

impl Editor {
    pub fn new(doc: Document) -> Self {
        let paras = doc.paragraphs();
        Self {
            doc,
            sel: Selection::default(),
            paras,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            goal_x: None,
            clipboard: None,
            revision: 0,
        }
    }

    pub fn para_count(&self) -> usize {
        self.paras.len()
    }

    pub fn para_ref(&self, i: usize) -> &ParaRef {
        &self.paras[i]
    }

    pub fn paragraph(&self, i: usize) -> &Paragraph {
        self.doc.paragraph(&self.paras[i]).expect("valid paragraph index")
    }

    fn paragraph_mut(&mut self, i: usize) -> &mut Paragraph {
        self.doc.paragraph_mut(&self.paras[i].clone()).expect("valid paragraph index")
    }

    fn text(&self, i: usize) -> String {
        self.paragraph(i).text()
    }

    fn refresh(&mut self) {
        self.paras = self.doc.paragraphs();
        self.revision += 1;
        let clamp = |p: Pos, paras: &[ParaRef], doc: &Document| -> Pos {
            if paras.is_empty() {
                return Pos::default();
            }
            let para = p.para.min(paras.len() - 1);
            let len = doc.paragraph(&paras[para]).map_or(0, |x| x.len());
            Pos::new(para, p.offset.min(len))
        };
        self.sel.anchor = clamp(self.sel.anchor, &self.paras, &self.doc);
        self.sel.focus = clamp(self.sel.focus, &self.paras, &self.doc);
    }

    /// Records an undo step unless this edit continues the previous one.
    fn checkpoint(&mut self, kind: EditKind) {
        let coalesce = kind != EditKind::Other && self.last_edit == Some(kind);
        if !coalesce {
            self.undo.push(Snapshot { sections: self.doc.sections.clone(), sel: self.sel });
            if self.undo.len() > 500 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = Some(kind);
        self.goal_x = None;
    }

    /// Ends typing coalescing (e.g. after a caret move).
    fn break_coalescing(&mut self) {
        self.last_edit = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(Snapshot { sections: std::mem::replace(&mut self.doc.sections, s.sections), sel: self.sel });
            self.sel = s.sel;
            self.last_edit = None;
            self.refresh();
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(Snapshot { sections: std::mem::replace(&mut self.doc.sections, s.sections), sel: self.sel });
            self.sel = s.sel;
            self.last_edit = None;
            self.refresh();
        }
    }

    /// Installs a section that finished reconstructing in the background,
    /// in the live document and throughout history (it is not an edit).
    pub fn replace_section(&mut self, index: usize, section: Arc<Section>) {
        let before = self.paras.iter().filter(|p| p.section < index).count();
        let old_count = self.paras.iter().filter(|p| p.section == index).count();
        if index >= self.doc.sections.len() {
            return;
        }
        self.doc.sections[index] = section.clone();
        for s in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            if index < s.sections.len() {
                s.sections[index] = section.clone();
            }
        }
        self.paras = self.doc.paragraphs();
        let new_count = self.paras.iter().filter(|p| p.section == index).count();
        let shift = |p: &mut Pos| {
            if p.para >= before + old_count {
                p.para = p.para + new_count - old_count;
            }
        };
        shift(&mut self.sel.anchor);
        shift(&mut self.sel.focus);
        self.revision += 1;
    }

    // ---------------------------------------------------------------------
    // Selection and navigation

    pub fn set_caret(&mut self, p: Pos) {
        self.sel = Selection::caret(p);
        self.goal_x = None;
        self.break_coalescing();
    }

    pub fn extend_to(&mut self, p: Pos) {
        self.sel.focus = p;
        self.goal_x = None;
        self.break_coalescing();
    }

    pub fn select_all(&mut self) {
        let last = self.paras.len().saturating_sub(1);
        let end = if self.paras.is_empty() { 0 } else { self.paragraph(last).len() };
        self.sel = Selection { anchor: Pos::new(0, 0), focus: Pos::new(last, end) };
    }

    pub fn select_word_at(&mut self, p: Pos) {
        let text = self.text(p.para);
        let (mut a, mut b) = (p.offset, p.offset);
        for (i, w) in text.split_word_bound_indices() {
            if i <= p.offset && p.offset <= i + w.len() && !w.trim().is_empty() {
                (a, b) = (i, i + w.len());
                break;
            }
        }
        self.sel = Selection { anchor: Pos::new(p.para, a), focus: Pos::new(p.para, b) };
    }

    pub fn select_paragraph(&mut self, para: usize) {
        let len = self.paragraph(para).len();
        self.sel = Selection { anchor: Pos::new(para, 0), focus: Pos::new(para, len) };
    }

    pub fn move_caret(&mut self, m: Move, extend: bool, layout: &DocLayout) {
        self.break_coalescing();
        if !extend && !self.sel.is_collapsed() && matches!(m, Move::Left | Move::Right) {
            let (a, b) = self.sel.ordered();
            self.sel = Selection::caret(if m == Move::Left { a } else { b });
            return;
        }
        let keep_goal = matches!(m, Move::Up | Move::Down);
        let target = self.target(self.sel.focus, m, layout);
        if !keep_goal {
            self.goal_x = None;
        }
        if extend { self.sel.focus = target } else { self.sel = Selection::caret(target) }
    }

    fn target(&mut self, p: Pos, m: Move, layout: &DocLayout) -> Pos {
        let n = self.paras.len();
        if n == 0 {
            return Pos::default();
        }
        let text = self.text(p.para);
        match m {
            Move::Left => match prev_grapheme(&text, p.offset) {
                Some(o) => Pos::new(p.para, o),
                None if p.para > 0 => Pos::new(p.para - 1, self.paragraph(p.para - 1).len()),
                None => p,
            },
            Move::Right => match next_grapheme(&text, p.offset) {
                Some(o) => Pos::new(p.para, o),
                None if p.para + 1 < n => Pos::new(p.para + 1, 0),
                None => p,
            },
            Move::WordLeft => match prev_word(&text, p.offset) {
                Some(o) => Pos::new(p.para, o),
                None if p.para > 0 => Pos::new(p.para - 1, self.paragraph(p.para - 1).len()),
                None => p,
            },
            Move::WordRight => match next_word(&text, p.offset) {
                Some(o) => Pos::new(p.para, o),
                None if p.para + 1 < n => Pos::new(p.para + 1, 0),
                None => p,
            },
            Move::Up | Move::Down => {
                if p.para >= layout.para_count() {
                    return p;
                }
                let x = *self.goal_x.get_or_insert_with(|| layout.caret(p.para, p.offset).map_or(0.0, |(_, r)| r.x0));
                let line = layout.line_of(p.para, p.offset);
                if m == Move::Up {
                    if line > 0 {
                        Pos::new(p.para, layout.offset_at(p.para, line - 1, x))
                    } else if p.para > 0 {
                        let q = p.para - 1;
                        Pos::new(q, layout.offset_at(q, layout.line_count(q).saturating_sub(1), x))
                    } else {
                        Pos::new(0, 0)
                    }
                } else if line + 1 < layout.line_count(p.para) {
                    Pos::new(p.para, layout.offset_at(p.para, line + 1, x))
                } else if p.para + 1 < n.min(layout.para_count()) {
                    Pos::new(p.para + 1, layout.offset_at(p.para + 1, 0, x))
                } else {
                    Pos::new(p.para, text.len())
                }
            }
            Move::LineStart | Move::LineEnd => {
                if p.para >= layout.para_count() {
                    return p;
                }
                let line = layout.line_of(p.para, p.offset);
                let (a, b) = layout.line_bounds(p.para, line, &text);
                Pos::new(p.para, if m == Move::LineStart { a } else { b })
            }
            Move::ParaStart => Pos::new(p.para, 0),
            Move::ParaEnd => Pos::new(p.para, text.len()),
            Move::DocStart => Pos::new(0, 0),
            Move::DocEnd => Pos::new(n - 1, self.paragraph(n - 1).len()),
        }
    }

    /// Plain text of the selection; paragraphs separated by '\n'.
    pub fn selected_text(&self) -> String {
        self.selected_runs().iter().map(|p| p.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn selected_runs(&self) -> Vec<Vec<TextRun>> {
        let (a, b) = self.sel.ordered();
        (a.para..=b.para.min(self.paras.len().saturating_sub(1)))
            .map(|i| {
                let p = self.paragraph(i);
                let start = if i == a.para { a.offset } else { 0 };
                let end = if i == b.para { b.offset } else { p.len() };
                p.slice(start..end)
            })
            .collect()
    }

    pub fn copy(&mut self) -> String {
        let plain = self.selected_text();
        self.clipboard = Some(RichClip { plain: plain.clone(), paragraphs: self.selected_runs() });
        plain
    }

    pub fn cut(&mut self) -> String {
        let s = self.copy();
        if !self.sel.is_collapsed() {
            self.checkpoint(EditKind::Other);
            self.delete_selection_inner();
            self.refresh();
        }
        s
    }
}

fn prev_grapheme(s: &str, o: usize) -> Option<usize> {
    s[..o].grapheme_indices(true).next_back().map(|(i, _)| i)
}

fn next_grapheme(s: &str, o: usize) -> Option<usize> {
    s[o..].graphemes(true).next().map(|g| o + g.len())
}

fn prev_word(s: &str, o: usize) -> Option<usize> {
    if o == 0 {
        return None;
    }
    let mut last = 0;
    for (i, w) in s.split_word_bound_indices() {
        if i >= o {
            break;
        }
        if !w.trim().is_empty() {
            last = i;
        }
    }
    Some(last)
}

fn next_word(s: &str, o: usize) -> Option<usize> {
    if o >= s.len() {
        return None;
    }
    s.split_word_bound_indices()
        .map(|(i, w)| (i + w.len(), w))
        .find(|(end, w)| *end > o && !w.trim().is_empty())
        .map(|(end, _)| end)
        .or(Some(s.len()))
}
