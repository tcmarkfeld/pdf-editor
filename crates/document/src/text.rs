//! Run-level text editing on a single paragraph. Offsets are UTF-8 byte
//! offsets into the paragraph's concatenated text.

use std::ops::Range;

use crate::model::{Paragraph, ParagraphStyle, TextRun, TextStyle};

impl Paragraph {
    pub fn new(text: impl Into<String>, style: TextStyle, pstyle: ParagraphStyle) -> Self {
        Self { runs: vec![TextRun { text: text.into(), style }], style: pstyle }
    }

    pub fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.runs.iter().map(|r| r.text.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Style that newly typed text at `offset` inherits: the character
    /// before the caret, or the first character when at the start.
    pub fn style_at(&self, offset: usize) -> &TextStyle {
        let mut start = 0;
        let mut first = None;
        for run in &self.runs {
            let end = start + run.text.len();
            if !run.text.is_empty() {
                first.get_or_insert(&run.style);
                if offset > start && offset <= end {
                    return &run.style;
                }
            }
            start = end;
        }
        first.unwrap_or(&self.runs[0].style)
    }

    /// Largest font size used on any run (including empty paragraphs).
    pub fn max_font_size(&self) -> f32 {
        self.runs.iter().map(|r| r.style.size).fold(0.0, f32::max)
    }

    pub fn insert(&mut self, offset: usize, text: &str, style: Option<&TextStyle>) {
        if text.is_empty() {
            return;
        }
        let style = style.cloned().unwrap_or_else(|| self.style_at(offset).clone());
        let idx = self.split_runs_at(offset);
        self.runs.insert(idx, TextRun { text: text.to_string(), style });
        self.normalize();
    }

    pub fn delete(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }
        let keep_style = self.style_at(range.start).clone();
        let a = self.split_runs_at(range.start);
        let b = self.split_runs_at(range.end);
        self.runs.drain(a..b);
        if self.runs.is_empty() {
            self.runs.push(TextRun { text: String::new(), style: keep_style });
        }
        self.normalize();
    }

    /// Splits this paragraph at `offset`, returning the tail as a new
    /// paragraph with the same paragraph style (minus space before).
    pub fn split_off(&mut self, offset: usize) -> Paragraph {
        let tail_style = self.style_at(offset).clone();
        let idx = self.split_runs_at(offset);
        let mut tail: Vec<TextRun> = self.runs.drain(idx..).collect();
        if self.runs.is_empty() {
            self.runs.push(TextRun { text: String::new(), style: tail_style.clone() });
        }
        if tail.is_empty() {
            tail.push(TextRun { text: String::new(), style: tail_style });
        }
        let mut style = self.style.clone();
        style.space_before = 0.0;
        let mut p = Paragraph { runs: tail, style };
        self.normalize();
        p.normalize();
        p
    }

    /// Appends `other`'s runs to the end of this paragraph.
    pub fn append(&mut self, other: Paragraph) {
        if self.is_empty() {
            self.runs = other.runs;
        } else {
            self.runs.extend(other.runs.into_iter().filter(|r| !r.text.is_empty()));
        }
        self.normalize();
    }

    /// Copies the runs covering `range` (used for clipboard and undo-free
    /// structural edits).
    pub fn slice(&self, range: Range<usize>) -> Vec<TextRun> {
        let mut out = Vec::new();
        let mut start = 0;
        for run in &self.runs {
            let end = start + run.text.len();
            let a = range.start.max(start);
            let b = range.end.min(end);
            if a < b {
                out.push(TextRun { text: run.text[a - start..b - start].to_string(), style: run.style.clone() });
            }
            start = end;
        }
        out
    }

    /// Applies `f` to the style of every run intersecting `range`.
    pub fn restyle(&mut self, range: Range<usize>, f: impl Fn(&mut TextStyle)) {
        if range.is_empty() {
            return;
        }
        let a = self.split_runs_at(range.start);
        let b = self.split_runs_at(range.end);
        for run in &mut self.runs[a..b] {
            f(&mut run.style);
        }
        self.normalize();
    }

    /// Ensures a run boundary exists at `offset`; returns the index of the
    /// run starting there (or `runs.len()`).
    fn split_runs_at(&mut self, offset: usize) -> usize {
        let mut start = 0;
        for i in 0..self.runs.len() {
            let len = self.runs[i].text.len();
            if offset == start {
                return i;
            }
            if offset < start + len {
                let tail = self.runs[i].text.split_off(offset - start);
                let style = self.runs[i].style.clone();
                self.runs.insert(i + 1, TextRun { text: tail, style });
                return i + 1;
            }
            start += len;
        }
        self.runs.len()
    }

    /// Merges adjacent runs with equal styles and removes empty runs,
    /// keeping at least one.
    pub fn normalize(&mut self) {
        let mut out: Vec<TextRun> = Vec::with_capacity(self.runs.len());
        let fallback = self.runs.first().map(|r| r.style.clone());
        for run in self.runs.drain(..) {
            if run.text.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.style == run.style => last.text.push_str(&run.text),
                _ => out.push(run),
            }
        }
        if out.is_empty() {
            out.push(TextRun { text: String::new(), style: fallback.unwrap_or_default() });
        }
        self.runs = out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> TextStyle {
        let mut s = TextStyle::default();
        s.font.weight = 700;
        s
    }

    fn para() -> Paragraph {
        let mut p = Paragraph::new("Hello ", TextStyle::default(), ParagraphStyle::default());
        p.runs.push(TextRun { text: "world".into(), style: bold() });
        p
    }

    #[test]
    fn insert_inherits_preceding_style() {
        let mut p = para();
        p.insert(11, "!", None);
        assert_eq!(p.text(), "Hello world!");
        assert_eq!(p.runs.len(), 2);
        assert!(p.runs[1].style.font.is_bold());
        p.insert(0, ">", None);
        assert!(!p.runs[0].style.font.is_bold());
    }

    #[test]
    fn delete_across_runs_and_to_empty() {
        let mut p = para();
        p.delete(3..8);
        assert_eq!(p.text(), "Helrld");
        p.delete(0..p.len());
        assert_eq!(p.text(), "");
        assert_eq!(p.runs.len(), 1);
    }

    #[test]
    fn split_and_append_roundtrip() {
        let mut p = para();
        let tail = p.split_off(8);
        assert_eq!(p.text(), "Hello wo");
        assert_eq!(tail.text(), "rld");
        assert!(tail.runs[0].style.font.is_bold());
        p.append(tail);
        assert_eq!(p.text(), "Hello world");
        assert_eq!(p.runs.len(), 2);
    }

    #[test]
    fn split_at_end_keeps_style_for_empty_tail() {
        let mut p = para();
        let tail = p.split_off(11);
        assert!(tail.is_empty());
        assert!(tail.runs[0].style.font.is_bold());
    }
}
