//! Headless commands. Returns `None` when the arguments ask for the GUI.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use document::Document;

const USAGE: &str = "usage:
  reflow [file.pdf|file.reflow]            open the editor
  reflow outline <in.pdf|in.reflow>        print the reconstructed structure
  reflow convert <in.pdf> <out.reflow>     reconstruct and save the editable document
  reflow export <in.pdf|in.reflow> <out.pdf>
  reflow compare <in.pdf> <out-dir> [scale]  original / reconstructed / diff PNGs";

pub fn run(args: &[String]) -> Option<i32> {
    let cmd = args.first()?.as_str();
    let result = match (cmd, &args[1..]) {
        ("outline", [input]) => load(input).map(|d| print!("{}", d.outline())),
        ("convert", [input, out]) => load(input).and_then(|d| d.save(Path::new(out)).map_err(|e| e.to_string())),
        ("export", [input, out]) => load(input).and_then(|d| {
            let laid = layout::Layouter::new().layout(&d);
            let bytes = crate::persist::pdf_bytes(&d, &laid)?;
            std::fs::write(out, bytes).map_err(|e| e.to_string())
        }),
        ("compare", [input, dir, rest @ ..]) => compare(input, Path::new(dir), rest.first().and_then(|s| s.parse().ok()).unwrap_or(1.5)),
        ("help" | "--help" | "-h", _) => {
            println!("{USAGE}");
            Ok(())
        }
        (c, _) if ["outline", "convert", "export", "compare"].contains(&c) => Err(USAGE.to_string()),
        _ => return None,
    };
    Some(match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    })
}

fn load(input: &str) -> Result<Document, String> {
    let path = Path::new(input);
    if path.extension().is_some_and(|e| e == "reflow") {
        return Document::load(path).map_err(|e| e.to_string());
    }
    let start = Instant::now();
    let src = pdf_source::PdfSource::open(path)?;
    if let Some(doc) = crate::persist::embedded_document(&src) {
        eprintln!("restored the editable document embedded by Reflow");
        return Ok(doc);
    }
    let mut sections = Vec::new();
    for i in 0..src.page_count() {
        let (section, analysis) = reconstruction::import_page(&src, i)?;
        for n in analysis.notes {
            eprintln!("page {}: {n}", i + 1);
        }
        sections.push(Arc::new(section));
    }
    eprintln!("imported {} pages in {:.0} ms", sections.len(), start.elapsed().as_secs_f64() * 1000.0);
    Ok(Document { sections })
}

fn compare(input: &str, dir: &Path, scale: f32) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let src = pdf_source::PdfSource::open(Path::new(input))?;
    let doc = load(input)?;
    let mut layouter = layout::Layouter::new();
    let laid = layouter.layout(&doc);
    let mut glyphs = render::GlyphCache::default();
    for p in 0..laid.page_count() {
        let page = laid.page(p);
        let recon = render::raster::render_page(page, scale, &mut glyphs, &render::raster::ALL);
        recon.save(dir.join(format!("p{}-recon.png", p + 1))).map_err(|e| e.to_string())?;
        if page.continuation {
            println!("page {}: continuation (content overflowed its source page)", p + 1);
            continue;
        }
        let orig = src.render(page.section as u32, scale)?;
        orig.save(dir.join(format!("p{}-orig.png", p + 1))).map_err(|e| e.to_string())?;
        let (diff, score) = render::raster::diff(&orig, &recon);
        diff.save(dir.join(format!("p{}-diff.png", p + 1))).map_err(|e| e.to_string())?;
        println!("page {}: {:.2}% pixels differ", p + 1, score * 100.0);
    }
    for (family, r) in layouter.fonts.substitutions() {
        println!("font substitution: {family} -> {} ({})", if r.family.is_empty() { "system" } else { &r.family }, r.reason);
    }
    Ok(())
}
