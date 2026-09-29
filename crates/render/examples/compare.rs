//! Renders original (PDFium) and reconstructed pages plus a diff overlay:
//! `cargo run -p render --example compare -- in.pdf out_dir [scale]`
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(&args[2]);
    let scale: f32 = args.get(3).map_or(1.5, |s| s.parse().unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let src = pdf_source::PdfSource::open(std::path::Path::new(&args[1])).unwrap();
    let mut doc = document::Document::default();
    for i in 0..src.page_count() {
        doc.sections.push(Arc::new(reconstruction::import_page(&src, i).unwrap().0));
    }
    let mut layouter = layout::Layouter::new();
    let laid = layouter.layout(&doc);
    let mut glyphs = render::GlyphCache::default();
    for p in 0..laid.page_count() {
        let page = laid.page(p);
        let recon = render::raster::render_page(page, scale, &mut glyphs, &render::raster::ALL);
        recon.save(out.join(format!("p{p}-recon.png"))).unwrap();
        if !page.continuation {
            let si = page.section as u32;
            let orig = src.render(si, scale).unwrap();
            orig.save(out.join(format!("p{p}-orig.png"))).unwrap();
            let (d, score) = render::raster::diff(&orig, &recon);
            d.save(out.join(format!("p{p}-diff.png"))).unwrap();
            println!("page {p}: {:.2}% pixels differ", score * 100.0);
        }
    }
    for (family, r) in layouter.fonts.substitutions() {
        println!("font substitution: {family} -> {:?} ({})", r.family, r.reason);
    }
}
