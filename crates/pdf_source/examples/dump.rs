//! Prints extracted glyphs/fonts/paths for a page: `cargo run -p pdf_source --example dump -- file.pdf [page]`
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = pdf_source::PdfSource::open(std::path::Path::new(&args[1])).unwrap();
    let page = src.extract(args.get(2).map_or(0, |p| p.parse().unwrap())).unwrap();
    println!("page {}x{}  glyphs={} images={} paths={} links={}", page.width, page.height, page.glyphs.len(), page.images.len(), page.paths.len(), page.links.len());
    for (i, f) in page.fonts.iter().enumerate() { println!("font {i}: {f:?}"); }
    for g in page.glyphs.iter().filter(|g| std::env::var("CH").map_or(true, |c| c.contains(g.ch))).take(80) {
        println!("{:?} x={:.2} base={:.2} loose=({:.2},{:.2},{:.2},{:.2}) f{} {:.2}pt gen={} {:?}", g.ch, g.origin_x, g.baseline, g.loose.x0, g.loose.y0, g.loose.x1, g.loose.y1, g.font, g.size, g.generated, g.color.0);
    }
    for p in page.paths.iter().filter(|p| p.bounds.width() < 8.0 && p.bounds.height() < 8.0).take(20) { println!("path {:?} fill={:?} stroke={:?} segs={}", p.bounds, p.fill, p.stroke, p.segments.len()); }
    for l in &page.links { println!("link {:?} {}", l.rect, l.uri); }
}
