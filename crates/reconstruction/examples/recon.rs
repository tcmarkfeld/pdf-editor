//! Prints the reconstructed outline: `cargo run -p reconstruction --example recon -- file.pdf`
use std::sync::Arc;

fn main() {
    let path = std::env::args().nth(1).expect("pdf path");
    let src = pdf_source::PdfSource::open(std::path::Path::new(&path)).unwrap();
    let mut doc = document::Document::default();
    for i in 0..src.page_count() {
        let (section, analysis) = reconstruction::import_page(&src, i).unwrap();
        for n in &analysis.notes {
            eprintln!("page {i}: {n}");
        }
        doc.sections.push(Arc::new(section));
    }
    print!("{}", doc.outline());
}
