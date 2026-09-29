//! Formatting commands: character styles, paragraph styles, block types,
//! lists, indentation, links and inserted rules.

use std::collections::HashMap;
use std::sync::Arc;

use document::{
    Align, Block, LineSpacing, ListInfo, ListKind, NumberFormat, ParagraphStyle, Rgba, Role, Rule, TextStyle,
};

use crate::{EditKind, Editor};

/// Extra left indent per list level, in points.
pub const LIST_INDENT: f32 = 18.0;
/// Indent step for ordinary paragraphs (half an inch).
pub const INDENT_STEP: f32 = 36.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockType {
    Normal,
    Heading(u8),
}

impl Editor {
    /// Paragraph indices touched by the selection.
    pub fn selected_paras(&self) -> std::ops::RangeInclusive<usize> {
        let (a, b) = self.sel.ordered();
        a.para..=b.para
    }

    /// Style at the caret (or the start of the selection): what the toolbar
    /// shows and what typed text will use.
    pub fn current_style(&self) -> TextStyle {
        if let Some(s) = &self.pending_style {
            return s.clone();
        }
        if self.paras.is_empty() {
            return TextStyle::default();
        }
        let (a, b) = self.sel.ordered();
        let p = self.paragraph(a.para);
        // Inside a selection, report the first selected character.
        let at = if a != b && a.offset < p.len() { next_char(&p.text(), a.offset) } else { a.offset };
        p.style_at(at).clone()
    }

    pub fn current_paragraph_style(&self) -> ParagraphStyle {
        if self.paras.is_empty() {
            return ParagraphStyle::default();
        }
        self.paragraph(self.sel.focus.para).style.clone()
    }

    /// Applies `f` to the selected text, or to the text about to be typed
    /// when the selection is collapsed.
    pub fn set_text_style(&mut self, f: impl Fn(&mut TextStyle)) {
        if self.paras.is_empty() {
            return;
        }
        if self.sel.is_collapsed() {
            let mut s = self.current_style();
            f(&mut s);
            self.pending_style = Some(s);
            return;
        }
        self.checkpoint(EditKind::Other);
        let (a, b) = self.sel.ordered();
        for i in a.para..=b.para {
            let len = self.paragraph(i).len();
            let from = if i == a.para { a.offset } else { 0 };
            let to = if i == b.para { b.offset } else { len };
            let p = self.paragraph_mut(i);
            let before = p.max_font_size();
            p.restyle(from..to, &f);
            let after = p.max_font_size();
            // Keep measured line spacing proportional when text grows/shrinks.
            if let LineSpacing::Exact(v) = p.style.line_spacing
                && before > 0.0
                && (after - before).abs() > 0.01
            {
                p.style.line_spacing = LineSpacing::Exact(v * after / before);
            }
        }
        self.refresh();
    }

    /// Toggles a boolean character style over the selection (Cmd+B / I / U).
    pub fn toggle_style(&mut self, get: fn(&TextStyle) -> bool, set: fn(&mut TextStyle, bool)) {
        let on = if self.sel.is_collapsed() {
            get(&self.current_style())
        } else {
            let (a, b) = self.sel.ordered();
            (a.para..=b.para).all(|i| {
                let p = self.paragraph(i);
                let from = if i == a.para { a.offset } else { 0 };
                let to = if i == b.para { b.offset } else { p.len() };
                p.slice(from..to).iter().filter(|r| !r.text.trim().is_empty()).all(|r| get(&r.style))
            })
        };
        self.set_text_style(|s| set(s, !on));
    }

    pub fn set_paragraph_style(&mut self, f: impl Fn(&mut ParagraphStyle)) {
        if self.paras.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        for i in self.selected_paras() {
            f(&mut self.paragraph_mut(i).style);
        }
        self.refresh();
    }

    pub fn set_align(&mut self, align: Align) {
        self.set_paragraph_style(|s| s.align = align);
    }

    /// Line spacing as a multiple of single spacing (1.2 × font size).
    pub fn set_line_spacing(&mut self, multiple: f32) {
        self.set_paragraph_style(|s| s.line_spacing = LineSpacing::Multiple(1.2 * multiple));
    }

    /// Body text style: the most common run style by character count.
    pub fn body_style(&self) -> TextStyle {
        type Key = (Arc<str>, u32, u16, bool);
        let mut counts: HashMap<Key, (usize, TextStyle)> = HashMap::new();
        for i in 0..self.paras.len() {
            let p = self.paragraph(i);
            if p.style.role != Role::Body {
                continue;
            }
            for r in &p.runs {
                let s = &r.style;
                let key = (s.font.family.clone(), (s.size * 10.0) as u32, s.font.weight, s.font.italic);
                counts.entry(key).or_insert((0, s.clone())).0 += r.text.len();
            }
        }
        let mut best = counts.into_values().max_by_key(|(n, _)| *n).map(|(_, s)| s).unwrap_or_default();
        best.link = None;
        best.underline = false;
        best.strike = false;
        best.font.weight = 400;
        best.font.italic = false;
        best
    }

    /// Word/Docs paragraph styles: Normal text or Heading 1–3.
    pub fn set_block_type(&mut self, t: BlockType) {
        if self.paras.is_empty() {
            return;
        }
        let body = self.body_style();
        self.checkpoint(EditKind::Other);
        for i in self.selected_paras() {
            let p = self.paragraph_mut(i);
            let (role, size, weight) = match t {
                BlockType::Normal => (Role::Body, body.size, 400),
                BlockType::Heading(1) => (Role::Heading(1), (body.size * 1.8).round(), 700),
                BlockType::Heading(2) => (Role::Heading(2), (body.size * 1.45).round(), 700),
                BlockType::Heading(l) => (Role::Heading(l), (body.size * 1.2).round(), 700),
            };
            let before = p.max_font_size();
            p.style.role = role;
            p.style.list = None;
            let len = p.len();
            p.restyle(0..len, |s| {
                s.size = size;
                s.font.weight = weight;
                if t == BlockType::Normal {
                    s.font.family = body.font.family.clone();
                    s.font.generic = body.font.generic;
                    s.color = body.color;
                }
            });
            if p.is_empty() {
                let s = &mut p.runs[0].style;
                s.size = size;
                s.font.weight = weight;
            }
            if let LineSpacing::Exact(v) = p.style.line_spacing
                && before > 0.0
            {
                p.style.line_spacing = LineSpacing::Exact(v * size / before);
            }
        }
        self.refresh();
    }

    pub fn block_type(&self) -> BlockType {
        match self.current_paragraph_style().role {
            Role::Heading(l) => BlockType::Heading(l),
            Role::Body => BlockType::Normal,
        }
    }

    /// Toggles bullets (`ordered == false`) or numbering on the selected
    /// paragraphs; switching kind converts in place.
    pub fn toggle_list(&mut self, ordered: bool) {
        if self.paras.is_empty() {
            return;
        }
        let range = self.selected_paras();
        let is_kind = |l: &ListInfo| matches!(l.kind, ListKind::Ordered { .. }) == ordered;
        let all = range.clone().all(|i| self.paragraph(i).style.list.as_ref().is_some_and(is_kind));
        // Join a directly preceding list of the same kind, else start one.
        let first = *range.start();
        let join = first
            .checked_sub(1)
            .filter(|&p| self.para_ref(p).is_next_sibling(self.para_ref(first)))
            .and_then(|p| self.paragraph(p).style.list.clone())
            .filter(is_kind);
        let id = join.as_ref().map_or_else(|| self.max_list_id() + 1, |l| l.id);
        self.checkpoint(EditKind::Other);
        for i in range {
            let marker_style = {
                let mut s = self.paragraph(i).style_at(0).clone();
                s.link = None;
                s.underline = false;
                s
            };
            let st = &mut self.paragraph_mut(i).style;
            if all {
                if let Some(l) = st.list.take() {
                    st.indent_left = (st.indent_left + l.marker_offset).max(0.0);
                }
                continue;
            }
            match &mut st.list {
                Some(l) => {
                    l.kind = list_kind(ordered, l.level);
                    l.id = id;
                }
                None => {
                    st.first_line_indent = 0.0;
                    st.indent_left += LIST_INDENT;
                    st.list = Some(ListInfo {
                        id,
                        level: 0,
                        kind: list_kind(ordered, 0),
                        marker_offset: -LIST_INDENT + 4.0,
                        marker_style,
                    });
                }
            }
        }
        self.refresh();
    }

    fn max_list_id(&self) -> u32 {
        (0..self.paras.len()).filter_map(|i| self.paragraph(i).style.list.as_ref().map(|l| l.id)).max().unwrap_or(0)
    }

    /// Increase/decrease indent: list items change level, other paragraphs
    /// move by half an inch.
    pub fn change_indent(&mut self, increase: bool) {
        if self.paras.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        for i in self.selected_paras() {
            let st = &mut self.paragraph_mut(i).style;
            match &mut st.list {
                Some(l) => {
                    let ordered = matches!(l.kind, ListKind::Ordered { .. });
                    if increase && l.level < 8 {
                        l.level += 1;
                        st.indent_left += LIST_INDENT;
                    } else if !increase && l.level > 0 {
                        l.level -= 1;
                        st.indent_left = (st.indent_left - LIST_INDENT).max(0.0);
                    } else if !increase {
                        let off = l.marker_offset;
                        st.list = None;
                        st.indent_left = (st.indent_left + off).max(0.0);
                        continue;
                    }
                    let level = l.level;
                    l.kind = list_kind(ordered, level);
                }
                None => {
                    let step = if increase { INDENT_STEP } else { -INDENT_STEP };
                    st.indent_left = (st.indent_left + step).clamp(0.0, 360.0);
                }
            }
        }
        self.refresh();
    }

    /// Adds (or with `None`, removes) a hyperlink on the selection.
    pub fn set_link(&mut self, url: Option<&str>) {
        if self.sel.is_collapsed() {
            return;
        }
        let link: Option<Arc<str>> = url.map(Arc::from);
        let body = self.body_style();
        self.set_text_style(|s| {
            s.link = link.clone();
            s.underline = link.is_some();
            s.color = if link.is_some() { Rgba::rgb(17, 85, 204) } else { body.color };
        });
    }

    pub fn current_link(&self) -> Option<Arc<str>> {
        self.current_style().link
    }

    /// Resets selected text to the body style and paragraphs to Normal.
    pub fn clear_formatting(&mut self) {
        if self.paras.is_empty() {
            return;
        }
        let body = self.body_style();
        if self.sel.is_collapsed() {
            self.pending_style = Some(body);
            return;
        }
        self.checkpoint(EditKind::Other);
        let (a, b) = self.sel.ordered();
        for i in a.para..=b.para {
            let p = self.paragraph_mut(i);
            let from = if i == a.para { a.offset } else { 0 };
            let to = if i == b.para { b.offset } else { p.len() };
            p.restyle(from..to, |s| *s = body.clone());
            p.style.role = Role::Body;
        }
        self.refresh();
    }

    /// Inserts a full-width horizontal line after the caret's paragraph.
    pub fn insert_rule(&mut self) {
        if self.paras.is_empty() {
            return;
        }
        self.checkpoint(EditKind::Other);
        let r = self.paras[self.sel.focus.para].clone();
        let color = self.paragraph(self.sel.focus.para).style_at(0).color;
        if let Some((flow, idx)) = self.doc.flow_mut(&r) {
            let rule = Rule { space_before: 4.0, x: 0.0, width: 0.0, thickness: 0.75, color };
            flow.insert(idx + 1, Block::Rule(rule));
        }
        self.refresh();
    }
}

/// Bullet glyphs and number formats cycle by nesting level, as in Docs.
fn list_kind(ordered: bool, level: u8) -> ListKind {
    if ordered {
        let format = [NumberFormat::Decimal, NumberFormat::LowerAlpha, NumberFormat::LowerRoman][level as usize % 3];
        ListKind::Ordered { start: 1, format, suffix: ".".into() }
    } else {
        ListKind::Bullet(["•", "◦", "▪"][level as usize % 3].into())
    }
}

fn next_char(s: &str, o: usize) -> usize {
    s[o..].chars().next().map_or(o, |c| o + c.len_utf8())
}
