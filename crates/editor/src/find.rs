//! Find and replace over all paragraphs (including table cells and columns).

use std::ops::Range;

use crate::{EditKind, Editor, Pos, Selection};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub para: usize,
    pub range: Range<usize>,
}

/// Byte length of `query` matched at the start of `hay`, if it matches.
fn match_at(hay: &str, query: &str, case_sensitive: bool) -> Option<usize> {
    let mut h = hay.char_indices();
    for q in query.chars() {
        let (_, c) = h.next()?;
        let same = if case_sensitive { c == q } else { c == q || c.to_lowercase().eq(q.to_lowercase()) };
        if !same {
            return None;
        }
    }
    Some(h.next().map_or(hay.len(), |(i, _)| i))
}

impl Editor {
    pub fn find_all(&self, query: &str, case_sensitive: bool) -> Vec<Match> {
        let mut out = Vec::new();
        if query.is_empty() {
            return out;
        }
        for para in 0..self.para_count() {
            let text = self.paragraph(para).text();
            let mut i = 0;
            while i < text.len() {
                if let Some(len) = match_at(&text[i..], query, case_sensitive) {
                    out.push(Match { para, range: i..i + len });
                    i += len.max(1);
                } else {
                    i += text[i..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
        out
    }

    /// Selects a match (the UI then scrolls to the caret).
    pub fn select_match(&mut self, m: &Match) {
        self.sel = Selection { anchor: Pos::new(m.para, m.range.start), focus: Pos::new(m.para, m.range.end) };
        self.break_coalescing();
    }

    /// Replaces one match, keeping the style of the text it replaces.
    pub fn replace_match(&mut self, m: &Match, with: &str) {
        self.checkpoint(EditKind::Other);
        self.replace_inner(m, with);
        self.sel = Selection::caret(Pos::new(m.para, m.range.start + with.len()));
        self.refresh();
    }

    /// Replaces every match as a single undo step; returns how many.
    pub fn replace_all(&mut self, query: &str, with: &str, case_sensitive: bool) -> usize {
        let matches = self.find_all(query, case_sensitive);
        if matches.is_empty() {
            return 0;
        }
        self.checkpoint(EditKind::Other);
        // Back to front so earlier offsets stay valid.
        for m in matches.iter().rev() {
            self.replace_inner(m, with);
        }
        self.refresh();
        matches.len()
    }

    fn replace_inner(&mut self, m: &Match, with: &str) {
        let p = self.paragraph_mut(m.para);
        let first = p.text()[m.range.start..].chars().next().map_or(0, char::len_utf8);
        let style = p.style_at(m.range.start + first).clone();
        p.delete(m.range.clone());
        p.insert(m.range.start, with, Some(&style));
    }
}
