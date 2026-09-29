//! Stage 3: lines of one flow -> paragraphs (with list, alignment, tab-stop,
//! spacing and heading properties).

use document::{
    Align, DESCENT_RATIO, LineSpacing, ListInfo, ListKind, NumberFormat, Paragraph, ParagraphStyle, Rect, Role, TabAlign,
    TabStop, TextRun, TextStyle,
};

use crate::glyphs::{G, StyleTable};
use crate::lines::Line;

/// Tolerance for "same x position", in points.
const ALIGN_TOL: f32 = 1.5;

pub struct PageContext<'a> {
    pub styles: StyleTable<'a>,
    /// Character-weighted modal font size of the page.
    pub body_size: f32,
    pub body_bold: bool,
    /// Distinct heading sizes, largest first.
    pub heading_sizes: Vec<f32>,
    pub next_list_id: u32,
}

/// A block draft with its box-model extent, used to compute `space_before`.
pub struct Placed {
    pub top: f32,
    pub bottom: f32,
    pub block: document::Block,
    pub rect: Rect,
    pub kind: &'static str,
    pub score: Option<crate::score::ReconstructionScore>,
}

#[derive(Clone, Debug)]
struct Marker {
    kind: ListKind,
    x0: f32,
    glyph: G,
}

struct Draft {
    lines: Vec<Line>,
    marker: Option<Marker>,
    centered: bool,
}

pub fn build_paragraphs(ctx: &mut PageContext, mut lines: Vec<Line>, x0: f32, x1: f32) -> Vec<Placed> {
    lines = merge_rows(lines, x0, x1);
    // Lines overlapping earlier lines (text over text) cannot participate in
    // flow without corrupting it: keep them exactly where they are.
    let mut frames = Vec::new();
    let mut kept: Vec<Line> = Vec::with_capacity(lines.len());
    for l in lines {
        let r = l.rect();
        let overlaps = kept.iter().any(|k| {
            let kr = k.rect();
            kr.x_overlap(&r) > 0.0 && kr.y_overlap(&r) > 0.3 * kr.height().min(r.height())
        });
        if overlaps { frames.push(l) } else { kept.push(l) }
    }
    let lines = kept;
    let mut drafts: Vec<Draft> = Vec::new();
    for mut line in lines {
        let marker = take_marker(&mut line);
        if line.words.is_empty() {
            continue;
        }
        let centered = is_centered(&line, x0, x1);
        let cont = match (drafts.last(), &marker) {
            (Some(d), None) => continues(ctx, d, &line, centered, x0, x1),
            _ => false,
        };
        if cont {
            drafts.last_mut().expect("draft").lines.push(line);
        } else {
            drafts.push(Draft { lines: vec![line], marker, centered });
        }
    }

    // Pitch for single-line paragraphs: measured pitches of same-size text.
    let mut pitches: Vec<(f32, f32)> = Vec::new();
    for d in &drafts {
        for w in d.lines.windows(2) {
            pitches.push((text_size(&w[0]), w[1].baseline - w[0].baseline));
        }
    }

    let mut out = Vec::with_capacity(drafts.len());
    let mut prev_list: Option<(u32, f32, std::mem::Discriminant<ListKind>)> = None;
    for i in 0..drafts.len() {
        let d = &drafts[i];
        let size = text_size(&d.lines[0]);
        let pitch = if d.lines.len() >= 2 {
            median(d.lines.windows(2).map(|w| w[1].baseline - w[0].baseline).collect())
        } else {
            let next = drafts.get(i + 1).map(|n| &n.lines[0]);
            let to_next = next
                .filter(|n| (text_size(n) - size).abs() < 0.05 * size)
                .map(|n| n.baseline - d.lines[0].baseline)
                .filter(|&p| p > 0.9 * size && p < 1.6 * size);
            to_next.unwrap_or_else(|| {
                let same: Vec<f32> = pitches.iter().filter(|(s, _)| (s - size).abs() < 0.05 * size).map(|p| p.1).collect();
                if same.is_empty() { 1.2 * size } else { median(same) }
            })
        };
        let mut style = paragraph_style(d, x0, x1);
        style.line_spacing = LineSpacing::Exact(round2(pitch));
        if let Some(m) = &d.marker {
            let text_x = d.lines[0].x0;
            let disc = std::mem::discriminant(&m.kind);
            let id = match prev_list {
                Some((id, _, k)) if k == disc && i > 0 && drafts[i - 1].marker.is_some() => id,
                _ => {
                    ctx.next_list_id += 1;
                    ctx.next_list_id
                }
            };
            let level = match prev_list {
                Some((pid, px, _)) if pid == id && m.x0 > px + 4.0 => 1,
                _ => 0,
            };
            prev_list = Some((id, m.x0, disc));
            style.list = Some(ListInfo {
                id,
                level,
                kind: m.kind.clone(),
                marker_offset: round2(m.x0 - text_x),
                marker_style: ctx.styles.style(&m.glyph, 0.0),
            });
        } else {
            prev_list = None;
        }

        let runs = build_runs(ctx, d);
        let para = Paragraph { runs, style };
        let first = &d.lines[0];
        let last = d.lines.last().expect("line");
        let top = first.baseline - (pitch - DESCENT_RATIO * size);
        let bottom = last.baseline + DESCENT_RATIO * text_size(last);
        let mut para = para;
        para.style.role = heading_role(ctx, d, &para);
        let rect = d.lines.iter().fold(Rect::EMPTY, |r, l| r.union(&l.rect()));
        let rect = d.marker.as_ref().map_or(rect, |m| rect.union(&Rect::new(m.x0, rect.y0, m.x0, rect.y1)));
        let score = crate::score::paragraph(&d.lines, para.style.align, x0, x1, |g| ctx.styles.is_bold(g));
        out.push(Placed { top, bottom, block: document::Block::Paragraph(para), rect, kind: "paragraph", score: Some(score) });
    }
    for l in frames {
        out.push(frame(ctx, l));
    }
    out
}

/// Absolutely positioned single-line frame (the low-confidence escape hatch).
fn frame(ctx: &mut PageContext, line: Line) -> Placed {
    let r = line.rect();
    let (x0, x1) = (line.x0, line.x1 + 2.0);
    let inner = build_paragraphs(ctx, vec![line], x0, x1);
    let top = inner.iter().map(|p| p.top).fold(r.y0, f32::min);
    let rect = Rect::new(x0, top, x1, r.y1);
    let blocks = crate::finalize(inner, top);
    Placed { top, bottom: top, block: document::Block::Frame(document::Frame { rect, blocks }), rect, kind: "frame", score: None }
}

/// Fragments sharing a baseline in a flow become one line with tab stops.
fn merge_rows(mut lines: Vec<Line>, x0: f32, x1: f32) -> Vec<Line> {
    lines.sort_by(|a, b| a.baseline.total_cmp(&b.baseline).then(a.x0.total_cmp(&b.x0)));
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    let center = (x0 + x1) * 0.5;
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && (lines[j].baseline - lines[i].baseline).abs() <= 0.25 * lines[i].size.min(lines[j].size) {
            j += 1;
        }
        let mut row: Vec<Line> = lines[i..j].to_vec();
        row.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        let n = row.len();
        let mut iter = row.into_iter().enumerate();
        let (_, mut base) = iter.next().expect("row");
        for (k, part) in iter {
            let (align, pos) = if k == n - 1 && (part.x1 - x1).abs() <= 3.0 {
                (TabAlign::Right, part.x1)
            } else if (part.center() - center).abs() <= 3.0 {
                (TabAlign::Center, part.center())
            } else {
                (TabAlign::Left, part.x0)
            };
            base.append_tabbed(part, align, pos);
        }
        out.push(base);
        i = j;
    }
    out
}

fn is_bullet(ch: char) -> bool {
    matches!(
        ch,
        '•' | '◦' | '▪' | '▫' | '■' | '□' | '●' | '○' | '◆' | '◇' | '►' | '▸' | '▹' | '‣' | '⁃' | '–' | '—' | '-' | '*' | '·'
            | '✓' | '✔' | '➢' | '➤' | '→' | '❖' | '∙' | '⦁' | '\u{f0b7}' | '\u{f0a7}' | '\u{f0d8}' | '\u{f076}' | '\u{f0fc}'
    ) || ('\u{f000}'..='\u{f0ff}').contains(&ch)
}

fn parse_ordinal(s: &str) -> Option<(u32, NumberFormat, String)> {
    let (body, suffix) = if let Some(b) = s.strip_prefix('(').and_then(|b| b.strip_suffix(')')) {
        (b, "()".to_string())
    } else if let Some(b) = s.strip_suffix('.') {
        (b, ".".to_string())
    } else {
        (s.strip_suffix(')')?, ")".to_string())
    };
    if !body.is_empty() && body.len() <= 2 && body.chars().all(|c| c.is_ascii_digit()) {
        return Some((body.parse().ok()?, NumberFormat::Decimal, suffix));
    }
    let roman = |b: &str| b.chars().all(|c| "ivxlc".contains(c));
    if !body.is_empty() && body.len() <= 4 && roman(body) && body != "c" && body != "l" {
        let val = roman_value(body)?;
        return Some((val, NumberFormat::LowerRoman, suffix));
    }
    let mut chars = body.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_lowercase() => Some((c as u32 - 'a' as u32 + 1, NumberFormat::LowerAlpha, suffix)),
        (Some(c), None) if c.is_ascii_uppercase() => Some((c as u32 - 'A' as u32 + 1, NumberFormat::UpperAlpha, suffix)),
        _ => None,
    }
}

fn roman_value(s: &str) -> Option<u32> {
    let v = |c| match c {
        'i' => 1,
        'v' => 5,
        'x' => 10,
        'l' => 50,
        'c' => 100,
        _ => 0,
    };
    let vals: Vec<u32> = s.chars().map(v).collect();
    let mut total = 0i64;
    for (i, &x) in vals.iter().enumerate() {
        if vals.get(i + 1).is_some_and(|&n| n > x) { total -= x as i64 } else { total += x as i64 }
    }
    (total > 0).then_some(total as u32)
}

/// Removes a leading list marker from the line, if the first word is one.
fn take_marker(line: &mut Line) -> Option<Marker> {
    if line.words.is_empty() || line.tabs.first().is_some_and(|t| t.word <= 1) {
        return None;
    }
    let first = line.words[0].clone();
    let text = line.word_text(0);
    let g0 = line.glyphs[first.start].clone();
    let kind = if line.words.len() >= 2 && text.chars().count() == 1 && is_bullet(g0.ch) {
        let ch = if ('\u{f000}'..='\u{f0ff}').contains(&g0.ch) || g0.ch == '⦁' || g0.ch == '∙' { '•' } else { g0.ch };
        ListKind::Bullet(ch.to_string())
    } else if let Some((start, format, suffix)) = parse_ordinal(&text).filter(|_| line.words.len() >= 2) {
        // Letters/roman numerals need a clear gap to count as markers.
        let gap = line.words[1].x0 - first.x1;
        if format != NumberFormat::Decimal && gap < 0.4 * g0.size {
            return None;
        }
        ListKind::Ordered { start, format, suffix }
    } else if is_bullet(g0.ch) && "•●▪■◦".contains(g0.ch) && text.chars().count() > 1 {
        // Glued bullet: split the glyph off as its own marker.
        line.words[0].start += 1;
        line.words[0].x0 = line.glyphs[first.start + 1].x0;
        line.x0 = line.words[0].x0;
        return Some(Marker { kind: ListKind::Bullet(g0.ch.to_string()), x0: g0.x0, glyph: g0 });
    } else {
        return None;
    };
    line.words.remove(0);
    for t in &mut line.tabs {
        t.word -= 1;
    }
    line.x0 = line.words[0].x0;
    line.size = line.glyphs[line.words[0].start..].iter().map(|g| g.size).fold(0.0, f32::max);
    Some(Marker { kind, x0: first.x0, glyph: g0 })
}

/// Largest font size among the line's words (markers excluded): the `S`
/// of the shared box model.
fn text_size(line: &Line) -> f32 {
    line.words.iter().flat_map(|w| &line.glyphs[w.start..w.end]).map(|g| g.size).fold(0.0, f32::max)
}

/// Size used by the majority of characters; compared for style changes.
fn major_size(line: &Line) -> f32 {
    let mut counts: Vec<(f32, usize)> = Vec::new();
    for g in line.words.iter().flat_map(|w| &line.glyphs[w.start..w.end]) {
        match counts.iter_mut().find(|(s, _)| (*s - g.size).abs() < 0.01) {
            Some(c) => c.1 += 1,
            None => counts.push((g.size, 1)),
        }
    }
    counts.iter().max_by_key(|c| c.1).map_or(line.size, |c| c.0)
}

fn majority(line: &Line, f: impl Fn(&G) -> bool) -> bool {
    let (mut yes, mut all) = (0, 0);
    for w in &line.words {
        for g in &line.glyphs[w.start..w.end] {
            all += 1;
            yes += f(g) as usize;
        }
    }
    yes * 2 > all
}

fn is_centered(line: &Line, x0: f32, x1: f32) -> bool {
    (line.center() - (x0 + x1) * 0.5).abs() <= 2.0 && line.x0 - x0 > 2.0 * line.size
}

fn continues(ctx: &PageContext, d: &Draft, next: &Line, next_centered: bool, x0: f32, x1: f32) -> bool {
    let prev = d.lines.last().expect("line");
    if !prev.tabs.is_empty() || !next.tabs.is_empty() {
        return false;
    }
    // Typography must match.
    let (ps, ns) = (major_size(prev), major_size(next));
    if (ps - ns).abs() > 0.05 * ps {
        return false;
    }
    let styles = &ctx.styles;
    if majority(prev, |g| styles.is_bold(g)) != majority(next, |g| styles.is_bold(g))
        || majority(prev, |g| styles.font(g.font).italic) != majority(next, |g| styles.font(g.font).italic)
    {
        return false;
    }
    // Vertical rhythm.
    let gap = next.baseline - prev.baseline;
    let expected = if d.lines.len() >= 2 {
        let n = d.lines.len();
        d.lines[n - 1].baseline - d.lines[n - 2].baseline
    } else {
        1.2 * ps
    };
    let limit = if d.lines.len() >= 2 { expected * 1.2 } else { 1.6 * ps };
    if gap > limit || gap < 0.6 * ps {
        return false;
    }
    // Horizontal geometry.
    let space = 0.3 * ps;
    let next_word = next.words[0].x1 - next.words[0].x0;
    if d.centered && next_centered {
        return prev.width() + space + next_word > (x1 - x0) - 2.0 * (prev.x0 - x0).min(x1 - prev.x1) - 1.0;
    }
    // Continuation lines align with the text after a list marker (hanging
    // indent), with the second line of a paragraph, or with the first line.
    let cont_left = if d.marker.is_some() {
        d.lines[0].x0
    } else if d.lines.len() >= 2 {
        d.lines[1].x0
    } else {
        prev.x0
    };
    let first_line_indent =
        d.lines.len() == 1 && d.marker.is_none() && next.x0 < prev.x0 - ALIGN_TOL && prev.x0 - next.x0 <= 4.0 * ps;
    if (next.x0 - cont_left).abs() > ALIGN_TOL && !first_line_indent {
        return false;
    }
    // Wrap evidence: had the next line's first word fitted on this line, the
    // break was deliberate.
    prev.x1 + space + next_word >= x1 - 1.0
}

fn paragraph_style(d: &Draft, x0: f32, x1: f32) -> ParagraphStyle {
    let mut s = ParagraphStyle::default();
    let lines = &d.lines;
    let first = &lines[0];
    let cont_left = if lines.len() >= 2 { lines[1].x0 } else { first.x0 };
    let max_right = lines.iter().map(|l| l.x1).fold(f32::MIN, f32::max);
    let n = lines.len();
    let lefts_equal = lines.iter().skip(1).all(|l| (l.x0 - cont_left).abs() <= ALIGN_TOL);
    s.align = if d.centered && lines.iter().all(|l| (l.center() - (x0 + x1) * 0.5).abs() <= 2.0) {
        Align::Center
    } else if n >= 2 && lefts_equal {
        let justified = lines[..n - 1].iter().all(|l| (l.x1 - max_right).abs() <= ALIGN_TOL)
            && max_right >= x1 - 2.0
            && lines[n - 1].x1 < max_right - 2.0
            && n >= 3;
        if justified { Align::Justify } else { Align::Left }
    } else if n == 1 && (first.x1 - x1).abs() <= 2.0 && first.x0 - x0 > 0.3 * (x1 - x0) && first.tabs.is_empty() {
        Align::Right
    } else {
        Align::Left
    };
    match s.align {
        Align::Center => {}
        Align::Right => s.indent_right = round2((x1 - max_right).max(0.0)),
        _ => {
            s.indent_left = round2(cont_left - x0);
            s.first_line_indent = round2(first.x0 - cont_left);
        }
    }
    for t in &first.tabs {
        s.tab_stops.push(TabStop { pos: round2(t.pos - (x0 + s.indent_left)), align: t.align });
    }
    s
}

fn build_runs(ctx: &PageContext, d: &Draft) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    let mut push = |text: &str, style: TextStyle| match runs.last_mut() {
        Some(last) if last.style == style => last.text.push_str(text),
        _ => runs.push(TextRun { text: text.to_string(), style }),
    };
    for (li, line) in d.lines.iter().enumerate() {
        let ls = letter_spacing(line);
        for (wi, w) in line.words.iter().enumerate() {
            if wi > 0 || li > 0 {
                let prev_g = if wi > 0 {
                    &line.glyphs[line.words[wi - 1].end - 1]
                } else {
                    let pl = &d.lines[li - 1];
                    &pl.glyphs[pl.words.last().expect("word").end - 1]
                };
                let next_g = &line.glyphs[w.start];
                let tab = wi > 0 && line.tabs.iter().any(|t| t.word == wi);
                let hyphen_wrap = wi == 0 && prev_g.ch == '-';
                if !hyphen_wrap {
                    let mut st = ctx.styles.style(prev_g, ls);
                    if prev_g.link != next_g.link {
                        st.link = None;
                    }
                    st.underline &= next_g.underline;
                    push(if tab { "\t" } else { " " }, st);
                }
            }
            for g in &line.glyphs[w.start..w.end] {
                let mut buf = [0u8; 4];
                push(g.ch.encode_utf8(&mut buf), ctx.styles.style(g, ls));
            }
        }
    }
    let mut p = Paragraph { runs, style: ParagraphStyle::default() };
    p.normalize();
    p.runs
}

/// Uniform extra spacing between letters (tracking), from intra-word gaps.
fn letter_spacing(line: &Line) -> f32 {
    let mut gaps = Vec::new();
    for w in &line.words {
        for pair in line.glyphs[w.start..w.end].windows(2) {
            gaps.push(pair[1].x0 - pair[0].x1);
        }
    }
    if gaps.len() < 3 {
        return 0.0;
    }
    let m = median(gaps);
    if m > 0.02 * line.size { (m * 20.0).round() / 20.0 } else { 0.0 }
}

fn heading_role(ctx: &PageContext, d: &Draft, p: &Paragraph) -> Role {
    if d.lines.len() > 2 || d.marker.is_some() || !d.lines[0].tabs.is_empty() || p.len() > 90 {
        return Role::Body;
    }
    let size = text_size(&d.lines[0]);
    if let Some(level) = ctx.heading_sizes.iter().position(|&s| (s - size).abs() < 0.05 * s) {
        return Role::Heading(level as u8 + 1);
    }
    let styles = &ctx.styles;
    let bold = d.lines.iter().all(|l| majority(l, |g| styles.is_bold(g)));
    let text = p.text();
    let upper = text.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
    if bold && !ctx.body_bold && upper && size >= ctx.body_size * 0.95 {
        return Role::Heading(ctx.heading_sizes.len() as u8 + 1);
    }
    Role::Body
}

pub fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

pub fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}
