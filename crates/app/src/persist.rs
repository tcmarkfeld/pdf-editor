//! PDFs saved by Reflow carry their editable document as an embedded
//! attachment (a "hybrid PDF"), so reopening restores tables, lists and
//! styles exactly instead of reconstructing them. The attachment records a
//! fingerprint of the PDF's text; if another program edits the PDF, the
//! fingerprint no longer matches and the file is reconstructed as usual.

use document::Document;
use layout::DocLayout;
use pdf_source::PdfSource;

const ATTACHMENT: &str = "reflow-document.json";

pub fn pdf_bytes(doc: &Document, layout: &DocLayout) -> Result<Vec<u8>, String> {
    // Fingerprint exactly what PDFium will extract from the saved file, so
    // the check on reopening compares like with like.
    let plain = export::export_pdf(layout)?;
    let fingerprint = PdfSource::from_bytes(plain)?.text_fingerprint()?;
    let json = doc.to_bytes(Some(fingerprint)).map_err(|e| e.to_string())?;
    export::export_pdf_with_attachment(layout, Some((ATTACHMENT, &json)))
}

/// The embedded editable document, if present and still matching the PDF.
pub fn embedded_document(source: &PdfSource) -> Option<Document> {
    let (doc, fingerprint) = Document::from_bytes(&source.attachment(ATTACHMENT)?).ok()?;
    let matches = fingerprint? == source.text_fingerprint().ok()? && doc.sections.len() == source.page_count() as usize;
    matches.then_some(doc)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn fixture_doc() -> Option<Document> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf/resume-single.pdf");
        let src = PdfSource::open(&path).ok()?;
        Some(Document { sections: vec![Arc::new(reconstruction::import_page(&src, 0).ok()?.0)] })
    }

    #[test]
    fn saved_pdf_restores_the_exact_document() {
        let Some(doc) = fixture_doc() else { return };
        let mut e = editor::Editor::new(doc);
        e.set_caret(editor::Pos::new(3, 10));
        e.insert_table(2, 2, 300.0);
        e.insert_text("kept");
        let layout = layout::Layouter::new().layout(&e.doc);
        let saved = PdfSource::from_bytes(pdf_bytes(&e.doc, &layout).unwrap()).unwrap();
        let restored = embedded_document(&saved).expect("embedded document restored");
        assert_eq!(restored.outline(), e.doc.outline());
    }

    #[test]
    fn pdf_changed_elsewhere_is_reconstructed_instead() {
        let Some(doc) = fixture_doc() else { return };
        let layout = layout::Layouter::new().layout(&doc);
        let original = pdf_bytes(&doc, &layout).unwrap();
        let fingerprint = PdfSource::from_bytes(original).unwrap().text_fingerprint().unwrap();

        // Same attachment, different page text: as if another app edited it.
        let mut e = editor::Editor::new(doc.clone());
        e.set_caret(editor::Pos::new(0, 0));
        e.insert_text("Edited elsewhere ");
        let other = layout::Layouter::new().layout(&e.doc);
        let json = doc.to_bytes(Some(fingerprint)).unwrap();
        let tampered = export::export_pdf_with_attachment(&other, Some((ATTACHMENT, &json))).unwrap();
        assert!(embedded_document(&PdfSource::from_bytes(tampered).unwrap()).is_none());
    }
}
