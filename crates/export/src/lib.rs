//! PDF export. Writes the *reconstructed* layout — selectable vector text
//! using subset-embedded fonts (krilla subsets the exact faces Parley shaped
//! with, so glyph ids and metrics match what was on screen), vector rules
//! and decorations, images, and link annotations. Never a page bitmap.

use std::collections::HashMap;
use std::sync::Arc;

use document::{PathSeg, Rgba};
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image;
use krilla::page::PageSettings;
use krilla::paint::{Fill, FillRule, Stroke};
use krilla::text::{Font, GlyphId, KrillaGlyph};
use krilla::{Data, Document};
use layout::{DocLayout, GlyphRun, Item};

pub fn export_pdf(layout: &DocLayout) -> Result<Vec<u8>, String> {
    let mut doc = Document::new();
    let mut fonts: HashMap<(u64, u32), Option<Font>> = HashMap::new();
    let mut images: HashMap<u64, Option<Image>> = HashMap::new();
    for i in 0..layout.page_count() {
        let pl = layout.page(i);
        let settings = PageSettings::from_wh(pl.size.w, pl.size.h).ok_or("invalid page size")?;
        let mut page = doc.start_page_with(settings);
        let mut links = Vec::new();
        {
            let mut s = page.surface();
            for item in &pl.items {
                match item {
                    Item::Rect { rect, color } => {
                        let mut pb = PathBuilder::new();
                        if let Some(r) = Rect::from_ltrb(rect.x0, rect.y0, rect.x1, rect.y1) {
                            pb.push_rect(r);
                        }
                        if let Some(path) = pb.finish() {
                            s.set_fill(Some(fill(*color)));
                            s.set_stroke(None);
                            s.draw_path(&path);
                        }
                    }
                    Item::Path(shape) => {
                        let Some(path) = build_path(&shape.segments) else { continue };
                        s.set_fill(shape.fill.map(fill));
                        s.set_stroke(shape.stroke.map(|(c, w)| stroke(c, w)));
                        s.draw_path(&path);
                    }
                    Item::Image { rect, image } => {
                        let img = images
                            .entry(image.id)
                            .or_insert_with(|| {
                                let data = Data::from(Arc::new(image.bytes.clone()));
                                match image.format {
                                    document::ImageFormat::Png => Image::from_png(data, true).ok(),
                                    document::ImageFormat::Jpeg => Image::from_jpeg(data, true).ok(),
                                }
                            })
                            .clone();
                        let (Some(img), Some(size)) = (img, Size::from_wh(rect.width(), rect.height())) else { continue };
                        s.push_transform(&Transform::from_translate(rect.x0, rect.y0));
                        s.draw_image(img, size);
                        s.pop();
                    }
                    Item::Glyphs(run) => {
                        let key = (run.font.data.id(), run.font.index);
                        let font = fonts
                            .entry(key)
                            .or_insert_with(|| {
                                let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(run.font.data.clone());
                                Font::new(Data::from(bytes), run.font.index)
                            })
                            .clone();
                        let Some(font) = font else { continue };
                        draw_run(&mut s, run, font);
                        if let (Some(uri), Some(first), Some(last)) = (&run.link, run.glyphs.first(), run.glyphs.last()) {
                            let r = Rect::from_ltrb(first.x, first.y - run.size * 0.8, last.x + last.advance, first.y + run.size * 0.2);
                            if let Some(r) = r {
                                links.push((r, uri.to_string()));
                            }
                        }
                    }
                }
            }
            s.finish();
        }
        for (rect, uri) in merge_links(links) {
            let target = krilla::annotation::Target::Action(krilla::action::Action::Link(krilla::action::LinkAction::new(uri)));
            page.add_annotation(krilla::annotation::Annotation::new_link(krilla::annotation::LinkAnnotation::new(rect, target), None));
        }
        page.finish();
    }
    doc.finish().map_err(|e| format!("{e:?}"))
}

fn draw_run(s: &mut krilla::surface::Surface, run: &GlyphRun, font: Font) {
    let Some(first) = run.glyphs.first() else { return };
    let size = run.size;
    let n = run.glyphs.len();
    let glyphs: Vec<KrillaGlyph> = run
        .glyphs
        .iter()
        .enumerate()
        .map(|(j, g)| {
            // Advances are taken from final positions so justification and
            // letter-spacing survive exactly.
            let adv = if j + 1 < n { run.glyphs[j + 1].x - g.x } else { g.advance };
            KrillaGlyph::new(
                GlyphId::new(g.id),
                adv / size,
                0.0,
                (g.y - first.y) / size,
                0.0,
                g.text.start as usize..g.text.end.max(g.text.start) as usize,
                None,
            )
        })
        .collect();
    s.set_fill(Some(fill(run.color)));
    s.set_stroke(run.embolden.then(|| stroke(run.color, size * 0.03)));
    s.draw_glyphs(Point::from_xy(first.x, first.y), &glyphs, font, &run.text, size, false);
    s.set_stroke(None);
}

/// Adjacent glyph runs of one hyperlink become a single annotation.
fn merge_links(links: Vec<(Rect, String)>) -> Vec<(Rect, String)> {
    let mut out: Vec<(Rect, String)> = Vec::new();
    for (r, uri) in links {
        if let Some((last, u)) = out.last_mut()
            && *u == uri
            && (last.top() - r.top()).abs() < 1.0
            && r.left() - last.right() < 8.0
            && let Some(m) = Rect::from_ltrb(last.left(), last.top().min(r.top()), r.right(), last.bottom().max(r.bottom()))
        {
            *last = m;
            continue;
        }
        out.push((r, uri));
    }
    out
}

fn color(c: Rgba) -> rgb::Color {
    rgb::Color::new(c.0[0], c.0[1], c.0[2])
}

fn fill(c: Rgba) -> Fill {
    Fill {
        paint: color(c).into(),
        opacity: krilla::num::NormalizedF32::new(c.0[3] as f32 / 255.0).unwrap_or(krilla::num::NormalizedF32::ONE),
        rule: FillRule::NonZero,
    }
}

fn stroke(c: Rgba, width: f32) -> Stroke {
    Stroke { paint: color(c).into(), width: width.max(0.1), ..Default::default() }
}

fn build_path(segs: &[PathSeg]) -> Option<krilla::geom::Path> {
    let mut pb = PathBuilder::new();
    for s in segs {
        match *s {
            PathSeg::MoveTo(p) => pb.move_to(p.x, p.y),
            PathSeg::LineTo(p) => pb.line_to(p.x, p.y),
            PathSeg::CurveTo(a, b, c) => pb.cubic_to(a.x, a.y, b.x, b.y, c.x, c.y),
            PathSeg::Close => pb.close(),
        }
    }
    pb.finish()
}
