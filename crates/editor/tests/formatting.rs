use std::sync::Arc;

use document::{Align, Block, Document, LineSpacing, ListKind, Margins, Paragraph, ParagraphStyle, Role, Section, Size, TextStyle};
use editor::{BlockType, Editor, Pos, Selection};
use layout::Layouter;

fn doc(paras: &[&str]) -> Document {
    let pstyle = ParagraphStyle { line_spacing: LineSpacing::Exact(14.0), ..Default::default() };
    let blocks = paras.iter().map(|t| Block::Paragraph(Paragraph::new(*t, TextStyle::default(), pstyle.clone()))).collect();
    let s = Section {
        page_size: Size::new(400.0, 500.0),
        margins: Margins { top: 30.0, right: 30.0, bottom: 30.0, left: 30.0 },
        blocks,
        ..Default::default()
    };
    Document { sections: vec![Arc::new(s)] }
}

fn select(e: &mut Editor, a: (usize, usize), b: (usize, usize)) {
    e.sel = Selection { anchor: Pos::new(a.0, a.1), focus: Pos::new(b.0, b.1) };
}

#[test]
fn bold_toggles_on_selection_and_as_pending_style() {
    let mut e = Editor::new(doc(&["hello world"]));
    select(&mut e, (0, 0), (0, 5));
    e.toggle_style(|s| s.font.is_bold(), |s, on| s.font.weight = if on { 700 } else { 400 });
    assert!(e.paragraph(0).runs[0].style.font.is_bold());
    assert_eq!(e.paragraph(0).runs[0].text, "hello");
    e.toggle_style(|s| s.font.is_bold(), |s, on| s.font.weight = if on { 700 } else { 400 });
    assert_eq!(e.paragraph(0).runs.len(), 1, "toggling again removes bold");

    // Collapsed caret: the next typed text is italic, existing text untouched.
    e.set_caret(Pos::new(0, 11));
    e.toggle_style(|s| s.font.italic, |s, on| s.font.italic = on);
    assert!(e.current_style().font.italic);
    e.insert_text("!");
    let p = e.paragraph(0);
    assert_eq!(p.runs.last().unwrap().text, "!");
    assert!(p.runs.last().unwrap().style.font.italic);
    assert!(!p.runs[0].style.font.italic);
    e.insert_text("?");
    assert!(e.paragraph(0).runs.last().unwrap().style.font.italic, "typing continues in the new style");
}

#[test]
fn font_size_change_scales_exact_line_spacing() {
    let mut e = Editor::new(doc(&["abc"]));
    select(&mut e, (0, 0), (0, 3));
    e.set_text_style(|s| s.size = 22.0);
    assert_eq!(e.paragraph(0).style.line_spacing, LineSpacing::Exact(28.0));
}

#[test]
fn alignment_and_line_spacing_apply_to_all_selected_paragraphs() {
    let mut e = Editor::new(doc(&["one", "two", "three"]));
    select(&mut e, (0, 1), (1, 1));
    e.set_align(Align::Center);
    e.set_line_spacing(2.0);
    assert_eq!(e.paragraph(0).style.align, Align::Center);
    assert_eq!(e.paragraph(1).style.align, Align::Center);
    assert_eq!(e.paragraph(2).style.align, Align::Left);
    assert_eq!(e.paragraph(1).style.line_spacing, LineSpacing::Multiple(2.4));
}

#[test]
fn headings_and_normal_text() {
    let mut e = Editor::new(doc(&["Title", "body text here", "more body"]));
    e.set_caret(Pos::new(0, 2));
    e.set_block_type(BlockType::Heading(1));
    let p = e.paragraph(0);
    assert_eq!(p.style.role, Role::Heading(1));
    assert!(p.runs[0].style.font.is_bold());
    assert!(p.runs[0].style.size > 18.0);
    e.set_block_type(BlockType::Normal);
    assert_eq!(e.paragraph(0).style.role, Role::Body);
    assert_eq!(e.paragraph(0).runs[0].style.size, TextStyle::default().size);
}

#[test]
fn bullets_numbering_and_nesting() {
    let mut e = Editor::new(doc(&["a", "b", "c"]));
    select(&mut e, (0, 0), (2, 1));
    e.toggle_list(false);
    let ids: Vec<u32> = (0..3).map(|i| e.paragraph(i).style.list.as_ref().unwrap().id).collect();
    assert!(ids.iter().all(|&i| i == ids[0]), "one list");
    let indent = e.paragraph(0).style.indent_left;
    assert!(indent > 0.0);

    // Convert to numbering in place.
    e.toggle_list(true);
    assert!(matches!(e.paragraph(1).style.list.as_ref().unwrap().kind, ListKind::Ordered { .. }));

    // Nest the middle item; it becomes lower-alpha and indents further.
    e.set_caret(Pos::new(1, 0));
    e.change_indent(true);
    let l = e.paragraph(1).style.list.clone().unwrap();
    assert_eq!(l.level, 1);
    assert!(e.paragraph(1).style.indent_left > indent);

    // Layout numbers 1, a, 2.
    let laid = Layouter::new().layout(&e.doc);
    let texts: Vec<String> = laid
        .page(0)
        .items
        .iter()
        .filter_map(|it| if let layout::Item::Glyphs(g) = it { Some(g.text.clone()) } else { None })
        .collect();
    assert!(texts.contains(&"1.".to_string()) && texts.contains(&"a.".to_string()) && texts.contains(&"2.".to_string()), "{texts:?}");

    // Toggling off removes the list and restores the indent.
    e.change_indent(false);
    select(&mut e, (0, 0), (2, 1));
    e.toggle_list(true);
    assert!((0..3).all(|i| e.paragraph(i).style.list.is_none()));
    assert!((0..3).all(|i| e.paragraph(i).style.indent_left < indent));
}

#[test]
fn new_list_joins_the_list_directly_above() {
    let mut e = Editor::new(doc(&["a", "b"]));
    e.set_caret(Pos::new(0, 0));
    e.toggle_list(false);
    e.set_caret(Pos::new(1, 0));
    e.toggle_list(false);
    assert_eq!(e.paragraph(0).style.list.as_ref().unwrap().id, e.paragraph(1).style.list.as_ref().unwrap().id);
}

#[test]
fn links_and_clear_formatting() {
    let mut e = Editor::new(doc(&["see docs here"]));
    select(&mut e, (0, 4), (0, 8));
    e.set_link(Some("https://example.com"));
    assert_eq!(e.current_link().as_deref(), Some("https://example.com"));
    assert!(e.paragraph(0).runs[1].style.underline);
    select(&mut e, (0, 0), (0, 13));
    e.clear_formatting();
    assert_eq!(e.paragraph(0).runs.len(), 1);
    assert!(e.paragraph(0).runs[0].style.link.is_none());
    e.undo();
    assert_eq!(e.paragraph(0).runs.len(), 3, "clear formatting is one undo step");
}

#[test]
fn insert_rule_after_paragraph() {
    let mut e = Editor::new(doc(&["above", "below"]));
    e.set_caret(Pos::new(0, 2));
    e.insert_rule();
    let blocks = &e.doc.sections[0].blocks;
    assert!(matches!(blocks[1], Block::Rule(_)));
    let laid = Layouter::new().layout(&e.doc);
    let rule_w = laid.page(0).items.iter().find_map(|it| if let layout::Item::Rect { rect, .. } = it { Some(rect.width()) } else { None });
    assert_eq!(rule_w, Some(340.0), "full content width");
}

#[test]
fn table_insert_type_navigate_and_restructure() {
    let mut e = Editor::new(doc(&["intro", "outro"]));
    e.set_caret(Pos::new(0, 5));
    e.insert_table(2, 3, 300.0);
    let c = e.table_at_caret().expect("caret in table");
    assert_eq!((c.row, c.col, c.rows, c.cols), (0, 0, 2, 3));
    e.insert_text("A1");
    assert!(e.table_next_cell(false));
    e.insert_text("B1");
    assert_eq!(e.table_at_caret().unwrap().col, 1);

    // Tab from the last cell appends a row.
    for _ in 0..4 {
        e.table_next_cell(false);
    }
    assert_eq!(e.table_at_caret().unwrap().row, 1);
    e.table_next_cell(false);
    let c = e.table_at_caret().unwrap();
    assert_eq!((c.rows, c.row, c.col), (3, 2, 0));

    e.table_insert_col(true);
    let c = e.table_at_caret().unwrap();
    assert_eq!((c.cols, c.col), (4, 1));
    e.table_delete_col();
    e.table_delete_row();
    let c = e.table_at_caret().unwrap();
    assert_eq!((c.rows, c.cols), (2, 3));

    // Text survived restructuring; layout shows a 2x3 grid.
    let texts: Vec<String> = (0..e.para_count()).map(|i| e.paragraph(i).text()).collect();
    assert!(texts.contains(&"A1".into()) && texts.contains(&"B1".into()), "{texts:?}");
    let laid = Layouter::new().layout(&e.doc);
    let a1 = (0..e.para_count()).find(|&i| e.paragraph(i).text() == "A1").unwrap();
    let b1 = a1 + 1;
    let (l1, l2) = (&laid.para(a1).lines[0], &laid.para(b1).lines[0]);
    assert!((l1.baseline - l2.baseline).abs() < 0.01, "cells share a row");
    assert!(l2.x0 > l1.x0 + 90.0, "second column to the right");
    assert_eq!(e.paragraph(e.para_count() - 1).text(), "outro");

    e.table_set_shading(Some(document::Rgba::rgb(230, 230, 230)), true);
    e.table_toggle_borders();
    assert!(e.table_borders().is_none());
    e.table_delete();
    assert!(e.table_at_caret().is_none());
    assert!(e.doc.sections[0].blocks.iter().all(|b| !matches!(b, Block::Table(_))));
    e.undo();
    assert!(e.doc.sections[0].blocks.iter().any(|b| matches!(b, Block::Table(_))), "undo restores the table");
}

#[test]
fn table_goes_below_the_paragraph_without_splitting_it() {
    let mut e = Editor::new(doc(&["before after", "next"]));
    e.set_caret(Pos::new(0, 7));
    e.insert_table(1, 2, 200.0);
    let blocks = &e.doc.sections[0].blocks;
    assert!(matches!(&blocks[0], Block::Paragraph(p) if p.text() == "before after"));
    assert!(matches!(blocks[1], Block::Table(_)));
    assert!(matches!(&blocks[2], Block::Paragraph(p) if p.text() == "next"));

    // At the very start of a paragraph the table goes above it.
    let mut e = Editor::new(doc(&["first"]));
    e.set_caret(Pos::new(0, 0));
    e.insert_table(1, 1, 200.0);
    assert!(matches!(e.doc.sections[0].blocks[0], Block::Table(_)));
}

#[test]
fn bullets_inside_table_cells_get_markers() {
    let mut e = Editor::new(doc(&["x"]));
    e.set_caret(Pos::new(0, 1));
    e.insert_table(1, 2, 300.0);
    e.insert_text("cell");
    e.enter();
    e.toggle_list(false);
    e.insert_text("item");
    assert!(e.paragraph(e.sel.focus.para).style.list.is_some());
    let laid = Layouter::new().layout(&e.doc);
    let markers = laid.page(0).items.iter().filter(|it| matches!(it, layout::Item::Glyphs(g) if g.text == "•")).count();
    assert_eq!(markers, 1);
}

#[test]
fn enter_after_heading_continues_in_body_style() {
    let mut e = Editor::new(doc(&["Title", "body"]));
    e.set_caret(Pos::new(0, 5));
    e.set_block_type(BlockType::Heading(1));
    e.enter();
    let p = e.paragraph(1);
    assert_eq!(p.style.role, Role::Body);
    assert!(!p.runs[0].style.font.is_bold());
    assert_eq!(p.runs[0].style.size, TextStyle::default().size);
    // Also when the heading is the last block (nothing below to copy).
    e.set_caret(Pos::new(2, 4));
    e.set_block_type(BlockType::Heading(2));
    e.enter();
    assert!(!e.paragraph(3).runs[0].style.font.is_bold());
}
