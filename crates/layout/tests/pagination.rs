use std::sync::Arc;

use document::{Block, Column, Columns, Document, LineSpacing, Margins, Paragraph, ParagraphStyle, Section, Size, TextStyle};
use layout::Layouter;

fn para(text: &str) -> Block {
    let st = ParagraphStyle { line_spacing: LineSpacing::Exact(20.0), ..Default::default() };
    Block::Paragraph(Paragraph::new(text, TextStyle::default(), st))
}

fn section(blocks: Vec<Block>) -> Section {
    Section {
        page_size: Size::new(200.0, 200.0),
        margins: Margins { top: 20.0, right: 20.0, bottom: 20.0, left: 20.0 },
        blocks,
        ..Default::default()
    }
}

#[test]
fn overflow_adds_continuation_pages_and_splits_paragraphs_by_line() {
    // 160pt of usable height at 20pt per line = 8 lines per page.
    let long = "word ".repeat(60);
    let doc = Document { sections: vec![Arc::new(section(vec![para("title"), para(&long)]))] };
    let laid = Layouter::new().layout(&doc);
    assert!(laid.page_count() >= 2);
    assert!(laid.page(1).continuation);
    let p = laid.para(1);
    let pages: Vec<usize> = p.lines.iter().map(|l| l.page).collect();
    assert_eq!(pages.iter().filter(|&&pg| pg == 0).count(), 7, "7 lines fit under the title");
    let first_on_next = p.lines.iter().find(|l| l.page == 1).unwrap();
    assert!((first_on_next.top - 20.0).abs() < 0.01, "continues at the top margin");
    for l in &p.lines {
        assert!(l.bottom <= 180.0 + 0.5, "never below the bottom margin");
    }
}

#[test]
fn sections_start_on_their_own_page() {
    let doc = Document { sections: vec![Arc::new(section(vec![para("a")])), Arc::new(section(vec![para("b")]))] };
    let laid = Layouter::new().layout(&doc);
    assert_eq!(laid.page_count(), 2);
    assert_eq!(laid.line_page(1, &laid.para(1).lines[0]), 1);
}

#[test]
fn columns_paginate_independently() {
    let long = "word ".repeat(40);
    let cols = Block::Columns(Columns {
        space_before: 0.0,
        columns: vec![
            Column { x: 0.0, width: 70.0, blocks: vec![para(&long)] },
            Column { x: 90.0, width: 70.0, blocks: vec![para("short")] },
        ],
    });
    let doc = Document { sections: vec![Arc::new(section(vec![cols, para("after")]))] };
    let laid = Layouter::new().layout(&doc);
    let tall = laid.para(0);
    let after = laid.para(2);
    let last = tall.lines.last().unwrap();
    assert!(last.page >= 1);
    assert_eq!(after.lines[0].page, last.page, "content after columns follows the taller column");
    assert!(after.lines[0].top >= last.bottom - 0.01);
}

#[test]
fn unchanged_sections_reuse_cached_layout() {
    let doc = Document { sections: vec![Arc::new(section(vec![para("a")])), Arc::new(section(vec![para("b")]))] };
    let mut layouter = Layouter::new();
    let first = layouter.layout(&doc);
    let mut edited = doc.clone();
    edited.section_mut(1);
    let second = layouter.layout(&edited);
    assert!(Arc::ptr_eq(&first.sections[0], &second.sections[0]));
    assert!(!Arc::ptr_eq(&first.sections[1], &second.sections[1]));
}
