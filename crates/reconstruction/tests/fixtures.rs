//! Reconstruction of synthetic pages whose geometry is known exactly, so the
//! expected structure follows directly from the thresholds.

mod common;

use common::{PageBuilder, advance, paragraphs, reconstruct, section};
use document::{Align, Block, LineSpacing, ListKind, NumberFormat, Role, TabAlign, TabStop};
use pdf_source::{FontId, SourcePage};

const LEFT: f32 = 72.0;
const WIDTH: f32 = 468.0;
const SIZE: f32 = 11.0;
const PITCH: f32 = 14.0;

const BODY: &str = "Reconstruction turns positioned glyphs back into editable paragraphs by measuring \
    the gaps between characters and lines. Every decision is a threshold on geometry, so the same \
    page always yields the same document. This sentence only exists to make the paragraph wrap \
    across several lines at a known width.";

const BODY_2: &str = "A second paragraph follows after a larger vertical gap than the line pitch, \
    which must start a new paragraph with positive space before it.";

fn page() -> (PageBuilder, FontId, FontId) {
    let mut b = PageBuilder::new(612.0, 792.0);
    let regular = b.font("Helvetica", 400, false);
    let bold = b.font("Helvetica-Bold", 700, false);
    (b, regular, bold)
}

fn widest(size: f32, lines: &[String]) -> f32 {
    lines.iter().map(|l| advance(size, l)).fold(0.0, f32::max)
}

fn assert_close(actual: f32, expected: f32, what: &str) {
    assert!((actual - expected).abs() < 0.01, "{what}: expected {expected}, got {actual}");
}

#[test]
fn wrapped_paragraph_joins_lines() {
    let (mut b, regular, _) = page();
    let lines = b.wrap_paragraph(LEFT, 100.0, WIDTH, SIZE, PITCH, regular, BODY);
    assert!(lines.len() >= 3, "fixture should wrap: {lines:?}");
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 1, "{}", doc.outline());
    let p = paras[0];
    assert_eq!(p.text(), lines.join(" "));
    assert_eq!(p.style.align, Align::Left);
    assert_eq!(p.style.line_spacing, LineSpacing::Exact(PITCH));
    assert_eq!(p.style.role, Role::Body);
    assert!(p.style.list.is_none() && p.style.tab_stops.is_empty());
}

#[test]
fn vertical_gap_separates_paragraphs() {
    let (mut b, regular, _) = page();
    let first = b.wrap_paragraph(LEFT, 100.0, WIDTH, SIZE, PITCH, regular, BODY);
    let extra = 10.0;
    let next = 100.0 + first.len() as f32 * PITCH + extra;
    let second = b.wrap_paragraph(LEFT, next, WIDTH, SIZE, PITCH, regular, BODY_2);
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 2, "{}", doc.outline());
    assert_eq!(paras[0].text(), first.join(" "));
    assert_eq!(paras[1].text(), second.join(" "));
    assert_close(paras[1].style.space_before, extra, "space_before");
}

#[test]
fn larger_bold_line_is_heading() {
    let (mut b, regular, bold) = page();
    b.text(LEFT, 100.0, 16.0, bold, "Experience");
    let body = b.wrap_paragraph(LEFT, 124.0, WIDTH, SIZE, PITCH, regular, BODY);
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 2, "{}", doc.outline());
    assert_eq!(paras[0].text(), "Experience");
    assert_eq!(paras[0].style.role, Role::Heading(1));
    assert!(paras[0].runs.iter().all(|r| r.style.font.is_bold()));
    assert_eq!(paras[1].text(), body.join(" "));
    assert_eq!(paras[1].style.role, Role::Body);
}

#[test]
fn bullets_with_hanging_continuations() {
    let (mut b, regular, _) = page();
    let items = [
        "Short first item.",
        "A long second item that keeps going well past the right edge of the text column so that it \
         wraps onto a hanging continuation line aligned with the item text.",
        "Third item.",
    ];
    let text_x = LEFT + 12.0;
    let mut baseline = 100.0;
    let mut expected = Vec::new();
    for item in items {
        b.text(LEFT, baseline, SIZE, regular, "•");
        let lines = b.wrap_paragraph(text_x, baseline, LEFT + WIDTH - text_x, SIZE, PITCH, regular, item);
        baseline += lines.len() as f32 * PITCH;
        expected.push(lines);
    }
    assert_eq!(expected[1].len(), 2, "second item should wrap once");
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 3, "{}", doc.outline());
    let id = paras[0].style.list.as_ref().expect("list").id;
    for (p, lines) in paras.iter().zip(&expected) {
        assert_eq!(p.text(), lines.join(" "));
        let list = p.style.list.as_ref().expect("list item");
        assert_eq!(list.id, id);
        assert_eq!(list.level, 0);
        assert_eq!(list.kind, ListKind::Bullet("•".into()));
        assert_close(list.marker_offset, -12.0, "marker_offset");
        assert_close(p.style.indent_left, 12.0, "indent_left");
    }
}

#[test]
fn numbered_list_is_ordered() {
    let (mut b, regular, _) = page();
    let items = ["Gather the source pages.", "Group glyphs into lines.", "Emit editable paragraphs."];
    for (i, item) in items.iter().enumerate() {
        let baseline = 100.0 + i as f32 * PITCH;
        b.text(LEFT, baseline, SIZE, regular, &format!("{}.", i + 1));
        b.text(LEFT + 18.0, baseline, SIZE, regular, item);
    }
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 3, "{}", doc.outline());
    let id = paras[0].style.list.as_ref().expect("list").id;
    for (i, (p, item)) in paras.iter().zip(items).enumerate() {
        assert_eq!(p.text(), item);
        let list = p.style.list.as_ref().expect("list item");
        assert_eq!(list.id, id);
        assert_eq!(
            list.kind,
            ListKind::Ordered { start: i as u32 + 1, format: NumberFormat::Decimal, suffix: ".".into() }
        );
    }
}

#[test]
fn bold_word_inside_sentence_is_a_run() {
    let (mut b, regular, bold) = page();
    let mut x = b.text(LEFT, 100.0, SIZE, regular, "This sentence has one ");
    x = b.text(x, 100.0, SIZE, bold, "bold");
    b.text(x, 100.0, SIZE, regular, " word inside it.");
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 1, "{}", doc.outline());
    let p = paras[0];
    assert_eq!(p.text(), "This sentence has one bold word inside it.");
    assert_eq!(p.runs.len(), 3, "{:#?}", p.runs);
    let bold_runs: Vec<_> = p.runs.iter().filter(|r| r.style.font.is_bold()).collect();
    assert_eq!(bold_runs.len(), 1);
    assert_eq!(bold_runs[0].text.trim(), "bold");
    assert_eq!(p.style.role, Role::Body);
}

#[test]
fn right_aligned_date_becomes_tab_stop() {
    let (mut b, regular, _) = page();
    let body = b.wrap_paragraph(LEFT, 114.0, WIDTH, SIZE, PITCH, regular, BODY);
    let right = LEFT + widest(SIZE, &body);
    b.text(LEFT, 100.0, SIZE, regular, "FirmPilot");
    b.text_right(right, 100.0, SIZE, regular, "2024 - Present");
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 2, "{}", doc.outline());
    assert_eq!(paras[0].text(), "FirmPilot\t2024 - Present");
    assert_eq!(paras[0].style.tab_stops, vec![TabStop { pos: right - LEFT, align: TabAlign::Right }]);
    assert!(paras[0].style.list.is_none());
    assert_eq!(paras[1].text(), body.join(" "));
    assert!(paras[1].style.tab_stops.is_empty());
}

const SIDEBAR: [&str; 7] = ["Programming Languages", "Rust", "TypeScript", "SQL", "Spoken Languages", "English", "Spanish"];

fn resume_page() -> SourcePage {
    let (mut b, regular, bold) = page();
    let size = 10.0;
    for (i, line) in SIDEBAR.iter().enumerate() {
        // Bold group titles keep the short items from reading as wrapped text.
        let font = if line.contains("Languages") { bold } else { regular };
        b.text(36.0, 100.0 + i as f32 * 13.0, size, font, line);
    }
    let first = b.wrap_paragraph(200.0, 104.0, 376.0, size, 14.5, regular, BODY);
    let next = 104.0 + first.len() as f32 * 14.5 + 10.0;
    b.wrap_paragraph(200.0, next, 376.0, size, 14.5, regular, BODY_2);
    b.build()
}

#[test]
fn two_column_resume_is_columns_block() {
    let doc = reconstruct(&resume_page());
    let blocks = &section(&doc).blocks;
    assert_eq!(blocks.len(), 1, "{}", doc.outline());
    let Block::Columns(cols) = &blocks[0] else { panic!("expected columns\n{}", doc.outline()) };
    assert_eq!(cols.columns.len(), 2, "{}", doc.outline());

    let left: Vec<String> = paragraphs(&doc, &cols.columns[0].blocks).iter().map(|p| p.text()).collect();
    assert_eq!(left, SIDEBAR);
    let right = paragraphs(&doc, &cols.columns[1].blocks);
    assert_eq!(right.len(), 2, "{}", doc.outline());
    assert!(right[0].text().starts_with("Reconstruction turns"));
    assert!(right[1].text().starts_with("A second paragraph"));
    assert!(cols.columns[1].x > cols.columns[0].x + cols.columns[0].width);
}

const TABLE_CELLS: [[&str; 3]; 4] = [
    ["Name", "Role", "City"],
    ["Alice", "Engineer", "Boston"],
    ["Bob", "Designer", "NYC"],
    ["Carol", "Manager", "Chicago"],
];

/// A 3x4 grid of 10pt cells (text 6pt right of each left rule) ruled on all
/// sides; the widest last-column cell ends at x = 467.
fn assert_ruled_table(right_rule: f32) {
    let (mut b, regular, _) = page();
    let rules_x = [66.0, 246.0, 426.0, right_rule];
    for (r, row) in TABLE_CELLS.iter().enumerate() {
        for (c, text) in row.iter().enumerate() {
            b.text(rules_x[c] + 6.0, 110.0 + r as f32 * 20.0, 10.0, regular, text);
        }
    }
    for r in 0..=TABLE_CELLS.len() {
        b.hrule(66.0, right_rule, 100.0 + r as f32 * 20.0, 0.5);
    }
    for x in rules_x {
        b.vrule(x, 100.0, 180.0, 0.5);
    }
    let doc = reconstruct(&b.build());

    let blocks = &section(&doc).blocks;
    assert_eq!(blocks.len(), 1, "{}", doc.outline());
    let Block::Table(t) = &blocks[0] else { panic!("expected table\n{}", doc.outline()) };
    let widths: Vec<f32> = rules_x.windows(2).map(|w| w[1] - w[0]).collect();
    assert_eq!(t.col_widths, widths, "{}", doc.outline());
    assert_eq!(t.rows.len(), 4);
    assert!(t.borders.is_some(), "{}", doc.outline());
    for (row, expected) in t.rows.iter().zip(TABLE_CELLS) {
        let texts: Vec<String> =
            row.cells.iter().map(|c| paragraphs(&doc, &c.blocks).iter().map(|p| p.text()).collect()).collect();
        assert_eq!(texts, expected);
    }
    assert!(section(&doc).decorations.is_empty(), "ruling lines should belong to the table\n{}", doc.outline());
}

#[test]
fn ruled_grid_is_table() {
    assert_ruled_table(474.0);
}

#[test]
fn ruled_grid_with_padded_last_column_is_table() {
    assert_ruled_table(546.0);
}

#[test]
fn letter_spaced_heading_keeps_words() {
    let (mut b, regular, bold) = page();
    let size = 14.0;
    b.text_tracked(LEFT, 100.0, size, bold, "ABOUT ME", 0.1 * size);
    b.wrap_paragraph(LEFT, 122.0, WIDTH, SIZE, PITCH, regular, BODY);
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 2, "{}", doc.outline());
    assert_eq!(paras[0].text(), "ABOUT ME");
    assert_eq!(paras[0].runs.len(), 1);
    assert_close(paras[0].runs[0].style.letter_spacing, 0.1 * size, "letter_spacing");
    assert!(paras[1].runs.iter().all(|r| r.style.letter_spacing == 0.0));
}

#[test]
fn geometric_gaps_become_spaces() {
    let (mut b, regular, _) = page();
    b.text_no_spaces(LEFT, 100.0, SIZE, regular, "Words separated only by gaps");
    let page = b.build();
    assert!(page.glyphs.iter().all(|g| g.ch != ' '));
    let doc = reconstruct(&page);

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 1, "{}", doc.outline());
    assert_eq!(paras[0].text(), "Words separated only by gaps");
}

#[test]
fn centered_name_is_center_aligned() {
    let (mut b, regular, bold) = page();
    let body = b.wrap_paragraph(LEFT, 130.0, WIDTH, SIZE, PITCH, regular, BODY);
    let center = LEFT + widest(SIZE, &body) * 0.5;
    let name = "Jane Doe";
    b.text(center - advance(20.0, name) * 0.5, 100.0, 20.0, bold, name);
    let doc = reconstruct(&b.build());

    let paras = paragraphs(&doc, &section(&doc).blocks);
    assert_eq!(paras.len(), 2, "{}", doc.outline());
    assert_eq!(paras[0].text(), name);
    assert_eq!(paras[0].style.align, Align::Center);
    assert_eq!(paras[1].style.align, Align::Left);
}

#[test]
fn reconstruction_is_deterministic() {
    let page = resume_page();
    let a = reconstruct(&page).outline();
    let b = reconstruct(&page).outline();
    let c = reconstruct(&resume_page()).outline();
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
fn rotated_text_becomes_raster_fallback_not_lost() {
    let (mut b, regular, _) = page();
    b.text(72.0, 100.0, 11.0, regular, "Body text stays editable.");
    b.text(560.0, 400.0, 11.0, regular, "SIDEWAYS");
    let mut page = b.build();
    let n = page.glyphs.len();
    for g in &mut page.glyphs[n - 8..] {
        g.angle = 90.0;
    }
    let r = reconstruction::reconstruct(&page);
    assert_eq!(r.raster_fallbacks.len(), 1, "one preserved region");
    assert!(r.raster_fallbacks[0].x0 >= 559.0);
    let doc = document::Document { sections: vec![std::sync::Arc::new(r.section)] };
    let outline = doc.outline();
    assert!(outline.contains("Body text stays editable."), "{outline}");
    assert!(!outline.contains("SIDEWAYS"), "{outline}");
}

#[test]
fn overlapping_text_is_kept_in_an_absolute_frame() {
    let (mut b, regular, bold) = page();
    b.text(72.0, 100.0, 11.0, regular, "First line of ordinary flow text");
    b.text(72.0, 114.0, 11.0, regular, "Second line of ordinary flow text");
    // A stamp drawn over the flow, slightly offset vertically.
    b.text(120.0, 110.0, 14.0, bold, "DRAFT");
    let doc = reconstruct(&b.build());
    let outline = doc.outline();
    assert!(outline.contains("FRAME"), "{outline}");
    assert!(outline.contains("\"DRAFT\""), "{outline}");
    let blocks = &section(&doc).blocks;
    let flow_texts: Vec<String> =
        blocks.iter().filter_map(|b| if let Block::Paragraph(p) = b { Some(p.text()) } else { None }).collect();
    assert!(flow_texts.iter().all(|t| !t.contains("DRAFT")), "{outline}");
}
