use std::sync::Arc;

use document::*;

#[test]
fn save_and_load_round_trip() {
    let image = Arc::new(ImageResource::png(vec![137, 80, 78, 71, 1, 2, 3], 1, 1));
    let mut para = Paragraph::new("FirmPilot\t2024 – Present", TextStyle::default(), ParagraphStyle::default());
    para.style.tab_stops.push(TabStop { pos: 400.0, align: TabAlign::Right });
    let section = Section {
        page_size: Size::new(612.0, 792.0),
        blocks: vec![Block::Paragraph(para), Block::Image(ImageBlock { space_before: 4.0, x: 0.0, width: 10.0, height: 10.0, image: image.clone() })],
        decorations: vec![Decoration::Image { rect: Rect::new(0.0, 0.0, 5.0, 5.0), image }],
        ..Default::default()
    };
    let doc = Document { sections: vec![Arc::new(section)] };
    let dir = std::env::temp_dir().join(format!("reflow-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("doc.reflow");
    doc.save(&path).unwrap();
    let loaded = Document::load(&path).unwrap();
    assert_eq!(loaded.outline(), doc.outline());
    let Block::Image(img) = &loaded.sections[0].blocks[1] else { panic!() };
    assert_eq!(img.image.bytes, vec![137, 80, 78, 71, 1, 2, 3]);
    std::fs::remove_dir_all(dir).ok();
}
