use std::sync::Arc;

use document::{Block, Document, LineSpacing, ListInfo, ListKind, Paragraph, ParagraphStyle, Section, Size, TextStyle};
use editor::{Editor, Move, Pos, Selection};
use layout::Layouter;

fn para(text: &str) -> Block {
    let pstyle = ParagraphStyle { line_spacing: LineSpacing::Exact(14.0), ..Default::default() };
    Block::Paragraph(Paragraph::new(text, TextStyle::default(), pstyle))
}

fn doc(paras: &[&str]) -> Document {
    let mut s = Section { page_size: Size::new(300.0, 400.0), ..Default::default() };
    s.margins = document::Margins { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 };
    s.blocks = paras.iter().map(|t| para(t)).collect();
    Document { sections: vec![Arc::new(s)] }
}

fn texts(e: &Editor) -> Vec<String> {
    (0..e.para_count()).map(|i| e.paragraph(i).text()).collect()
}

#[test]
fn typing_inserts_and_coalesces_into_one_undo_step() {
    let mut e = Editor::new(doc(&["Hello"]));
    e.set_caret(Pos::new(0, 5));
    for c in [" ", "w", "o", "r", "l", "d"] {
        e.insert_text(c);
    }
    assert_eq!(texts(&e), ["Hello world"]);
    e.undo();
    assert_eq!(texts(&e), ["Hello"]);
    e.redo();
    assert_eq!(texts(&e), ["Hello world"]);
    assert_eq!(e.sel, Selection::caret(Pos::new(0, 11)));
}

#[test]
fn enter_splits_and_backspace_merges() {
    let mut e = Editor::new(doc(&["Senior Software Engineer"]));
    e.set_caret(Pos::new(0, 6));
    e.enter();
    assert_eq!(texts(&e), ["Senior", " Software Engineer"]);
    assert_eq!(e.sel.focus, Pos::new(1, 0));
    e.backspace(false);
    assert_eq!(texts(&e), ["Senior Software Engineer"]);
    assert_eq!(e.sel.focus, Pos::new(0, 6));
}

#[test]
fn delete_across_paragraphs_merges_ends() {
    let mut e = Editor::new(doc(&["alpha one", "beta", "gamma two"]));
    e.sel = Selection { anchor: Pos::new(0, 5), focus: Pos::new(2, 5) };
    e.backspace(false);
    assert_eq!(texts(&e), ["alpha two"]);
    e.undo();
    assert_eq!(texts(&e), ["alpha one", "beta", "gamma two"]);
}

#[test]
fn paste_multiline_creates_paragraphs() {
    let mut e = Editor::new(doc(&["ab"]));
    e.set_caret(Pos::new(0, 1));
    e.insert_text("1\n2\n3");
    assert_eq!(texts(&e), ["a1", "2", "3b"]);
    assert_eq!(e.sel.focus, Pos::new(2, 1));
}

#[test]
fn copy_paste_preserves_styles_internally() {
    let mut d = doc(&["plain "]);
    if let Block::Paragraph(p) = &mut Arc::make_mut(&mut d.sections[0]).blocks[0] {
        let mut bold = TextStyle::default();
        bold.font.weight = 700;
        p.insert(6, "bold", Some(&bold));
    }
    let mut e = Editor::new(d);
    e.sel = Selection { anchor: Pos::new(0, 6), focus: Pos::new(0, 10) };
    let text = e.copy();
    e.set_caret(Pos::new(0, 0));
    e.paste(&text);
    let p = e.paragraph(0);
    assert_eq!(p.text(), "boldplain bold");
    assert!(p.runs[0].style.font.is_bold());
}

#[test]
fn list_enter_and_backspace_follow_word_processor_rules() {
    let mut d = doc(&["item"]);
    if let Block::Paragraph(p) = &mut Arc::make_mut(&mut d.sections[0]).blocks[0] {
        p.style.list = Some(ListInfo {
            id: 1,
            level: 0,
            kind: ListKind::Bullet("•".into()),
            marker_offset: -10.0,
            marker_style: TextStyle::default(),
        });
    }
    let mut e = Editor::new(d);
    e.set_caret(Pos::new(0, 4));
    e.enter();
    assert!(e.paragraph(1).style.list.is_some(), "new item continues the list");
    e.enter();
    assert!(e.paragraph(1).style.list.is_none(), "Enter on an empty item leaves the list");
    assert_eq!(e.para_count(), 2);
    e.set_caret(Pos::new(0, 0));
    e.backspace(false);
    assert!(e.paragraph(0).style.list.is_none(), "backspace at item start removes the marker");
    assert_eq!(texts(&e)[0], "item");
}

#[test]
fn vertical_navigation_moves_between_wrapped_lines() {
    let long = "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
    let mut e = Editor::new(doc(&[long, "next"]));
    let mut layouter = Layouter::new();
    let laid = layouter.layout(&e.doc);
    assert!(laid.line_count(0) >= 2, "paragraph wraps");
    e.set_caret(Pos::new(0, 0));
    e.move_caret(Move::Down, false, &laid);
    assert_eq!(laid.line_of(0, e.sel.focus.offset), 1);
    e.move_caret(Move::DocEnd, false, &laid);
    assert_eq!(e.sel.focus, Pos::new(1, 4));
    e.move_caret(Move::WordLeft, true, &laid);
    assert_eq!(e.selected_text(), "next");
}

#[test]
fn reflow_pushes_following_content_down() {
    let mut e = Editor::new(doc(&["short", "below"]));
    let mut layouter = Layouter::new();
    let before = layouter.layout(&e.doc).para(1).lines[0].baseline;
    e.set_caret(Pos::new(0, 5));
    e.insert_text(" and now a much longer sentence that will certainly need to wrap onto another line");
    let laid = layouter.layout(&e.doc);
    assert!(laid.line_count(0) >= 2);
    let after = laid.para(1).lines[0].baseline;
    assert!((after - before - 14.0 * (laid.line_count(0) - 1) as f32).abs() < 0.01, "moved by whole line pitches");
}

/// The headline scenario from a real (WebKit-generated) resume PDF: extend a
/// job title that sits on a tabbed row with a right-aligned location.
#[test]
fn edit_title_on_resume_keeps_row_and_reflows() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/pdf/resume-single.pdf");
    let Ok(src) = pdf_source::PdfSource::open(std::path::Path::new(path)) else {
        eprintln!("PDFium unavailable; skipping");
        return;
    };
    let section = reconstruction::reconstruct(&src.extract(0).unwrap()).section;
    let mut e = Editor::new(Document { sections: vec![Arc::new(section)] });
    let i = (0..e.para_count()).find(|&i| e.paragraph(i).text().starts_with("Senior Software Engineer")).unwrap();
    let mut layouter = Layouter::new();
    let before = layouter.layout(&e.doc);
    let next_before = before.para(i + 1).lines[0].baseline;
    let right_edge = before.para(i).lines[0].x1;

    e.set_caret(Pos::new(i, "Senior Software Engineer".len()));
    e.insert_text(", Platform");
    let after = layouter.layout(&e.doc);
    assert_eq!(e.paragraph(i).text(), "Senior Software Engineer, Platform\tSt. Petersburg, FL");
    assert_eq!(after.line_count(i), 1, "title and location still share one line");
    assert!((after.para(i).lines[0].x1 - right_edge).abs() < 0.5, "location stays right-aligned");
    assert!((after.para(i + 1).lines[0].baseline - next_before).abs() < 0.01, "nothing below moved");

    // Grow the summary paragraph by a line: everything after it moves down.
    let s = (0..e.para_count()).find(|&i| e.paragraph(i).text().starts_with("Software engineer with")).unwrap();
    let pitch = after.para(s).lines[1].baseline - after.para(s).lines[0].baseline;
    let len = e.paragraph(s).len();
    e.set_caret(Pos::new(s, len));
    e.insert_text(" Also enjoys hiking, board games, and teaching introductory programming at the local library on weekends.");
    let grown = layouter.layout(&e.doc);
    let delta = grown.line_count(s) as f32 - after.line_count(s) as f32;
    assert!(delta >= 1.0);
    let moved = grown.para(i).lines[0].baseline - after.para(i).lines[0].baseline;
    assert!((moved - delta * pitch).abs() < 0.05, "later rows shift by the added lines: {moved} vs {}", delta * pitch);
}
