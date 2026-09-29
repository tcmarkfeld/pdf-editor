//! Horizontal layout of one paragraph: shaping, tab segments, wrapping,
//! alignment and list markers. Vertical placement is done by `flow`.

use document::{Align, ListKind, Paragraph, TabAlign, TextStyle};
use fonts::FontSystem;
use parley::{
    Alignment, AlignmentOptions, FontStyle, FontWeight, IndentOptions, LayoutContext, PositionedLayoutItem, StyleProperty,
};

use crate::{GlyphRun, Item, PositionedGlyph, RunIndex, Segment};

/// Default tab stop interval (half an inch), as in word processors.
const DEFAULT_TAB: f32 = 36.0;

pub struct Shaped {
    pub text: String,
    pub segments: Vec<Segment>,
    /// x offset of each segment relative to the content edge.
    pub seg_x: Vec<f32>,
    /// Visual line k = line k of every segment that has one.
    pub line_count: usize,
    /// Largest font size on each visual line.
    pub line_sizes: Vec<f32>,
    pub content_x: f32,
    pub width: f32,
}

pub fn shape(
    p: &Paragraph,
    flow_x: f32,
    flow_w: f32,
    fonts: &mut FontSystem,
    lcx: &mut LayoutContext<RunIndex>,
) -> Shaped {
    let st = &p.style;
    let text = p.text();
    let mut content_x = flow_x + st.indent_left;
    let mut width = (flow_w - st.indent_left - st.indent_right).max(1.0);

    let mut ranges = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        if ch == '\t' {
            ranges.push(start..i);
            start = i + 1;
        }
    }
    ranges.push(start..text.len());
    let tabbed = ranges.len() > 1;

    let stacks: Vec<_> = p.runs.iter().map(|r| fonts.family_stack(&r.style.font)).collect();
    let mut segments: Vec<Segment> = ranges
        .iter()
        .map(|r| {
            let sub = &text[r.clone()];
            let mut b = lcx.ranged_builder(&mut fonts.fcx, sub, 1.0, false);
            let first = p.style_at(r.start);
            b.push_default(StyleProperty::FontSize(first.size));
            b.push_default(StyleProperty::FontFamily(stacks[0].clone()));
            let mut run_start = 0;
            for (ri, run) in p.runs.iter().enumerate() {
                let (a, z) = (run_start.max(r.start), (run_start + run.text.len()).min(r.end));
                run_start += run.text.len();
                if a >= z {
                    continue;
                }
                let range = a - r.start..z - r.start;
                let s = &run.style;
                b.push(StyleProperty::FontFamily(stacks[ri].clone()), range.clone());
                b.push(StyleProperty::FontSize(s.size), range.clone());
                b.push(StyleProperty::FontWeight(FontWeight::new(s.font.weight as f32)), range.clone());
                if s.font.italic {
                    b.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
                }
                if s.letter_spacing != 0.0 {
                    b.push(StyleProperty::LetterSpacing(s.letter_spacing), range.clone());
                }
                b.push(StyleProperty::Brush(ri as RunIndex), range);
            }
            Segment { layout: b.build(sub), start: r.start, end: r.end }
        })
        .collect();

    let mut seg_x = vec![0.0; segments.len()];
    if !tabbed {
        let layout = &mut segments[0].layout;
        let fli = st.first_line_indent;
        if fli >= 0.0 {
            layout.set_text_indent(fli, IndentOptions::default());
        } else {
            content_x += fli;
            width -= fli;
            layout.set_text_indent(-fli, IndentOptions { hanging: true, ..Default::default() });
        }
        // A hair of slack so lines that exactly fit in the source still fit.
        layout.break_all_lines(Some(width + 0.5));
        let align = match st.align {
            Align::Left => Alignment::Left,
            Align::Center => Alignment::Center,
            Align::Right => Alignment::Right,
            Align::Justify => Alignment::Justify,
        };
        layout.align(align, AlignmentOptions::default());
    } else {
        place_tabs(p, &mut segments, &mut seg_x, width);
    }

    let line_count = segments.iter().map(|s| s.layout.len()).max().unwrap_or(0).max(1);
    let fallback_size = p.max_font_size();
    let line_sizes = (0..line_count)
        .map(|k| {
            let mut size: f32 = 0.0;
            for s in &segments {
                if let Some(line) = s.layout.get(k) {
                    for run in line.runs() {
                        size = size.max(run.font_size());
                    }
                }
            }
            if size > 0.0 { size } else { fallback_size }
        })
        .collect();
    Shaped { text, segments, seg_x, line_count, line_sizes, content_x, width }
}

/// Word-processor tab semantics: each tab advances to the next stop after
/// the current pen position; right/centre stops align the following text
/// segment's end/centre. A segment that would collide wraps instead.
fn place_tabs(p: &Paragraph, segments: &mut [Segment], seg_x: &mut [f32], width: f32) {
    let mut stops: Vec<(f32, TabAlign)> = p.style.tab_stops.iter().map(|t| (t.pos, t.align)).collect();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    let last_explicit = stops.last().copied();
    let mut natural = Vec::with_capacity(segments.len());
    for s in segments.iter_mut() {
        s.layout.break_all_lines(None);
        s.layout.align(Alignment::Left, AlignmentOptions::default());
        natural.push(s.layout.width());
    }
    seg_x[0] = p.style.first_line_indent.max(0.0);
    let mut cur = seg_x[0] + natural[0];
    for i in 1..segments.len() {
        let (pos, align) = stops
            .iter()
            .copied()
            .find(|s| s.0 > cur + 0.01)
            .or(last_explicit.filter(|s| s.1 != TabAlign::Left && i == segments.len() - 1))
            .unwrap_or(((cur / DEFAULT_TAB).floor() * DEFAULT_TAB + DEFAULT_TAB, TabAlign::Left));
        let w = natural[i];
        let x = match align {
            TabAlign::Left => pos,
            TabAlign::Right => pos - w,
            TabAlign::Center => pos - w * 0.5,
        };
        let gap = 0.3 * p.style_at(segments[i].start).size;
        if seg_x[i - 1] + natural[i - 1] > x - gap {
            // Previous segment collides: wrap it before this segment.
            let limit = (x - gap - seg_x[i - 1]).max(24.0);
            segments[i - 1].layout.break_all_lines(Some(limit));
            segments[i - 1].layout.align(Alignment::Left, AlignmentOptions::default());
        }
        seg_x[i] = x.max(seg_x[i - 1] + 1.0);
        cur = seg_x[i] + w;
    }
    // A last segment running past the margin wraps within what is left.
    let n = segments.len();
    if cur > width + 0.5 && width - seg_x[n - 1] > 24.0 {
        segments[n - 1].layout.break_all_lines(Some(width - seg_x[n - 1]));
    }
}

/// Emits glyph and decoration items for visual line `k` of a shaped
/// paragraph whose segment `s` maps by `(dx[s], dy)`.
pub fn emit_line(p: &Paragraph, shaped: &Shaped, k: usize, dy: impl Fn(f32) -> f32, items: &mut Vec<Item>) {
    for (si, seg) in shaped.segments.iter().enumerate() {
        let Some(line) = seg.layout.get(k) else { continue };
        let dx = shaped.content_x + shaped.seg_x[si];
        let ddy = dy(line.metrics().baseline);
        let sub = &shaped.text[seg.start..seg.end];
        let mut prev_run = usize::MAX;
        let mut glyph_start = 0;
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(gr) = item else { continue };
            let run = gr.run();
            if run.index() != prev_run {
                prev_run = run.index();
                glyph_start = 0;
            }
            let count = gr.glyphs().count();
            let ranges: Vec<std::ops::Range<usize>> = run
                .visual_clusters()
                .flat_map(|c| {
                    let r = c.text_range();
                    c.glyphs().map(move |_| r.clone())
                })
                .skip(glyph_start)
                .take(count)
                .collect();
            glyph_start += count;
            let style = run_style(p, gr.style().brush);
            let lo = ranges.iter().map(|r| r.start).min().unwrap_or(0);
            let hi = ranges.iter().map(|r| r.end).max().unwrap_or(0);
            let glyphs: Vec<PositionedGlyph> = gr
                .positioned_glyphs()
                .zip(ranges.iter())
                .map(|(g, r)| PositionedGlyph {
                    id: g.id,
                    x: g.x + dx,
                    y: g.y + ddy,
                    advance: g.advance,
                    text: (r.start - lo) as u32..(r.end - lo) as u32,
                })
                .collect();
            let synthesis = run.synthesis();
            if style.underline || style.strike {
                let m = run.metrics();
                let x0 = gr.offset() + dx;
                let x1 = x0 + gr.advance();
                let base = gr.baseline() + ddy;
                if style.underline {
                    let th = m.underline_size.max(0.5);
                    let y = base - m.underline_offset;
                    items.push(Item::Rect { rect: document::Rect::new(x0, y - th * 0.5, x1, y + th * 0.5), color: style.color });
                }
                if style.strike {
                    let th = m.strikethrough_size.max(0.5);
                    let y = base - m.strikethrough_offset;
                    items.push(Item::Rect { rect: document::Rect::new(x0, y - th * 0.5, x1, y + th * 0.5), color: style.color });
                }
            }
            items.push(Item::Glyphs(GlyphRun {
                font: run.font().clone(),
                size: run.font_size(),
                color: style.color,
                embolden: synthesis.embolden(),
                skew: synthesis.skew(),
                glyphs,
                text: sub.get(lo..hi).unwrap_or_default().to_string(),
                link: style.link.clone(),
            }));
        }
    }
}

fn run_style(p: &Paragraph, brush: RunIndex) -> &TextStyle {
    &p.runs.get(brush as usize).unwrap_or(&p.runs[0]).style
}

/// Shapes a list marker and places it with its baseline at `baseline` and
/// its left edge at `x`.
pub fn emit_marker(
    text: &str,
    style: &TextStyle,
    x: f32,
    baseline: f32,
    fonts: &mut FontSystem,
    lcx: &mut LayoutContext<RunIndex>,
    items: &mut Vec<Item>,
) {
    let stack = fonts.family_stack(&style.font);
    let mut b = lcx.ranged_builder(&mut fonts.fcx, text, 1.0, false);
    b.push_default(StyleProperty::FontFamily(stack));
    b.push_default(StyleProperty::FontSize(style.size));
    b.push_default(StyleProperty::FontWeight(FontWeight::new(style.font.weight as f32)));
    let mut layout = b.build(text);
    layout.break_all_lines(None);
    let marker = Paragraph::new(text, style.clone(), Default::default());
    let shaped = Shaped {
        text: text.to_string(),
        segments: vec![Segment { layout, start: 0, end: text.len() }],
        seg_x: vec![0.0],
        line_count: 1,
        line_sizes: vec![style.size],
        content_x: x,
        width: 0.0,
    };
    emit_line(&marker, &shaped, 0, |b| baseline - style.baseline_shift - b, items);
}

pub fn marker_text(kind: &ListKind, n: u32) -> String {
    match kind {
        ListKind::Bullet(s) => s.clone(),
        ListKind::Ordered { format, suffix, .. } if suffix == "()" => format!("({})", format.format(n)),
        ListKind::Ordered { format, suffix, .. } => format!("{}{suffix}", format.format(n)),
    }
}
