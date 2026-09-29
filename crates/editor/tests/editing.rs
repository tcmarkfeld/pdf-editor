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

/// Mirrors the GUI loop: apply a command, relayout, then run every geometry
/// query the canvas uses. Exercised at many caret positions on real pages.
#[test]
fn enter_enter_backspace_backspace_everywhere() {
    for name in ["resume-single.pdf", "resume-two-column.pdf"] {
        let path = format!("{}/../../fixtures/pdf/{name}", env!("CARGO_MANIFEST_DIR"));
        let Ok(src) = pdf_source::PdfSource::open(std::path::Path::new(&path)) else { return };
        let sections = (0..src.page_count()).map(|i| Arc::new(reconstruction::import_page(&src, i).unwrap().0)).collect();
        let base = Editor::new(Document { sections });
        for para in 0..base.para_count() {
            let len = base.paragraph(para).len();
            for offset in [0, len / 2, len] {
                let offset = (0..=offset).rev().find(|&o| base.paragraph(para).text().is_char_boundary(o)).unwrap();
                let mut e = Editor::new(base.doc.clone());
                let mut layouter = Layouter::new();
                e.set_caret(Pos::new(para, offset));
                let steps: [fn(&mut Editor); 4] = [|e| e.enter(), |e| e.enter(), |e| e.backspace(false), |e| e.backspace(false)];
                for step in steps {
                    step(&mut e);
                    let laid = layouter.layout(&e.doc);
                    assert_eq!(laid.para_count(), e.para_count(), "{name} p{para}@{offset}");
                    let f = e.sel.focus;
                    laid.caret(f.para, f.offset);
                    laid.line_of(f.para, f.offset);
                    for p in 0..e.para_count() {
                        laid.selection_rects(p, 0..e.paragraph(p).len() + 1);
                    }
                }
                if base.paragraph(para).style.list.is_none() {
                    assert_eq!(e.paragraph(para).text(), base.paragraph(para).text(), "{name} p{para}@{offset} round trip");
                }
            }
        }
    }
}

/// Seeded random editing sessions over real pages; every step re-lays out
/// and runs the canvas' geometry queries. Catches panics anywhere in the
/// editor/layout stack.
#[test]
fn random_editing_sessions_never_panic() {
    let mut seed: u64 = 0x5eed;
    let mut rnd = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n.max(1) as u64) as usize
    };
    for name in ["resume-single.pdf", "resume-two-column.pdf"] {
        let path = format!("{}/../../fixtures/pdf/{name}", env!("CARGO_MANIFEST_DIR"));
        let Ok(src) = pdf_source::PdfSource::open(std::path::Path::new(&path)) else { return };
        let sections: Vec<_> = (0..src.page_count()).map(|i| Arc::new(reconstruction::import_page(&src, i).unwrap().0)).collect();
        for session in 0..40 {
            let mut e = Editor::new(Document { sections: sections.clone() });
            let mut layouter = Layouter::new();
            let mut laid = layouter.layout(&e.doc);
            for step in 0..60 {
                let n = e.para_count();
                let pick = |e: &Editor, r: &mut dyn FnMut(usize) -> usize| {
                    let p = r(e.para_count());
                    let t = e.paragraph(p).text();
                    let o = r(t.len() + 1);
                    let o = (0..=o).rev().find(|&o| t.is_char_boundary(o)).unwrap();
                    Pos::new(p, o)
                };
                match rnd(26) {
                    14 => e.toggle_list(rnd(2) == 0),
                    15 => e.change_indent(rnd(2) == 0),
                    16 => e.set_block_type([editor::BlockType::Normal, editor::BlockType::Heading(1), editor::BlockType::Heading(2)][rnd(3)]),
                    17 => e.insert_table(1 + rnd(3), 1 + rnd(3), 200.0),
                    18 => e.table_insert_row(rnd(2) == 0),
                    19 => e.table_insert_col(rnd(2) == 0),
                    20 => e.table_delete_row(),
                    21 => e.table_delete_col(),
                    22 => { e.table_next_cell(rnd(2) == 0); }
                    23 => e.toggle_style(|s| s.font.is_bold(), |s, on| s.font.weight = if on { 700 } else { 400 }),
                    24 => e.insert_rule(),
                    25 => e.table_delete(),
                    0 | 1 => { let p = pick(&e, &mut rnd); e.set_caret(p) }
                    2 => { let p = pick(&e, &mut rnd); e.extend_to(p) }
                    3 => e.insert_text(["x", "hello ", "é", "\t", "a\nb"][rnd(5)]),
                    4 | 5 => e.enter(),
                    6 | 7 => e.backspace(rnd(4) == 0),
                    8 => e.delete_forward(rnd(4) == 0),
                    9 => e.undo(),
                    10 => e.redo(),
                    11 => { let t = e.copy(); e.paste(&t) }
                    12 => e.move_caret([Move::Up, Move::Down, Move::Left, Move::Right, Move::WordLeft, Move::LineEnd][rnd(6)], rnd(2) == 0, &laid),
                    _ => e.select_all(),
                }
                laid = layouter.layout(&e.doc);
                let ctx = format!("{name} session {session} step {step} (paras {n} -> {})", e.para_count());
                assert_eq!(laid.para_count(), e.para_count(), "{ctx}");
                let (a, b) = e.sel.ordered();
                assert!(b.para < e.para_count() && b.offset <= e.paragraph(b.para).len(), "{ctx}");
                laid.caret(e.sel.focus.para, e.sel.focus.offset);
                for p in a.para..=b.para {
                    laid.selection_rects(p, 0..e.paragraph(p).len() + 1);
                }
                for page in 0..laid.page_count() {
                    laid.hit(page, 300.0, 400.0);
                }
            }
        }
    }
}

/// Regression: pages arriving from background reconstruction must not push
/// the initial caret past the end (Enter/Backspace/toolbar then panicked).
#[test]
fn background_sections_keep_selection_in_bounds() {
    let pending = |_| Arc::new(Section { page_size: Size::new(300.0, 400.0), pending: true, ..Default::default() });
    let mut e = Editor::new(Document { sections: (0..3).map(pending).collect() });
    let filled = |e: &mut Editor, i: usize, texts: &[&str]| {
        let mut s = (*doc(texts).sections[0]).clone();
        s.pending = false;
        e.replace_section(i, Arc::new(s));
    };
    filled(&mut e, 0, &["a", "b", "c"]);
    assert_eq!(e.sel.focus, Pos::new(0, 0));
    e.enter();
    e.enter();
    e.backspace(false);
    e.backspace(false);
    assert_eq!(e.paragraph(0).text(), "a");

    // A caret in a later section shifts when an earlier section arrives.
    filled(&mut e, 2, &["z"]);
    e.set_caret(Pos::new(3, 1));
    filled(&mut e, 1, &["m", "n"]);
    assert_eq!(e.sel.focus, Pos::new(5, 1));
    assert_eq!(e.paragraph(5).text(), "z");
}
