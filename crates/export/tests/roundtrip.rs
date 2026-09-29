//! Export round-trip: the exported PDF must contain real (extractable) text
//! reflecting edits, keep page sizes and links, and reconstruct again to the
//! same structure.

use std::path::Path;
use std::sync::Arc;

use document::Document;
use editor::{Editor, Pos};
use layout::Layouter;
use pdf_source::PdfSource;

fn fixture(name: &str) -> Option<PdfSource> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name);
    PdfSource::open(&path).map_err(|e| eprintln!("skipping: {e}")).ok()
}

fn import(src: &PdfSource) -> Document {
    let sections = (0..src.page_count()).map(|i| Arc::new(reconstruction::reconstruct(&src.extract(i).unwrap()).section)).collect();
    Document { sections }
}

fn text_of(page: &pdf_source::SourcePage) -> String {
    page.glyphs.iter().map(|g| g.ch).collect()
}

#[test]
fn edited_resume_exports_as_vector_text() {
    let Some(src) = fixture("resume-single.pdf") else { return };
    let mut e = Editor::new(import(&src));
    let i = (0..e.para_count()).find(|&i| e.paragraph(i).text().starts_with("Senior Software Engineer")).unwrap();
    e.set_caret(Pos::new(i, "Senior Software Engineer".len()));
    e.insert_text(", Platform");

    let laid = Layouter::new().layout(&e.doc);
    let bytes = export::export_pdf(&laid).expect("export");
    let out = PdfSource::from_bytes(bytes).expect("exported PDF opens");
    assert_eq!(out.page_count(), 1);
    assert_eq!(out.page_size(0).unwrap(), src.page_size(0).unwrap());

    let page = out.extract(0).unwrap();
    assert!(page.images.is_empty(), "no page bitmap");
    let text = text_of(&page);
    assert!(text.contains("SeniorSoftwareEngineer,Platform") || text.contains("Senior Software Engineer, Platform"), "{text}");
    assert!(text.contains("Northwind"));
    assert_eq!(page.links.len(), src.extract(0).unwrap().links.len(), "hyperlinks preserved");

    // Re-importing the export yields the same structure (with the edit).
    let again = reconstruction::reconstruct(&page).section;
    let reimported = Document { sections: vec![Arc::new(again)] }.outline();
    assert!(reimported.contains("Senior Software Engineer, Platform\\tSt. Petersburg, FL"), "{reimported}");
    assert!(reimported.contains("TABLE 3x4 borders=true"), "{reimported}");
}

#[test]
fn export_matches_source_visually() {
    let Some(src) = fixture("resume-two-column.pdf") else { return };
    let laid = Layouter::new().layout(&import(&src));
    let out = PdfSource::from_bytes(export::export_pdf(&laid).unwrap()).unwrap();
    let a = src.render(0, 1.0).unwrap();
    let b = out.render(0, 1.0).unwrap();
    let (_, score) = diff(&a, &b);
    assert!(score < 0.03, "exported page differs from source in {:.2}% of pixels", score * 100.0);
}

fn diff(a: &image::RgbaImage, b: &image::RgbaImage) -> ((), f32) {
    let mut n = 0usize;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let la = pa.0[0] as i32 + pa.0[1] as i32 + pa.0[2] as i32;
        let lb = pb.0[0] as i32 + pb.0[1] as i32 + pb.0[2] as i32;
        if (la - lb).abs() > 192 {
            n += 1;
        }
    }
    ((), n as f32 / (a.width() * a.height()) as f32)
}
