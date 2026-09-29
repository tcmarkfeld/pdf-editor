//! CPU rasterizer for laid-out pages (tiny-skia + swash masks). Used for the
//! path/decoration layer on screen, PNG previews, and visual diffing.

use document::{ImageResource, PathSeg, Rgba};
use image::RgbaImage;
use layout::{Item, PageLayout};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use crate::glyphs::GlyphCache;

pub struct Layers {
    pub text: bool,
    pub graphics: bool,
    pub background: bool,
}

pub const ALL: Layers = Layers { text: true, graphics: true, background: true };

pub fn render_page(page: &PageLayout, scale: f32, glyphs: &mut GlyphCache, layers: &Layers) -> RgbaImage {
    let w = (page.size.w * scale).round().max(1.0) as u32;
    let h = (page.size.h * scale).round().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h).expect("pixmap size");
    if layers.background {
        pm.fill(tiny_skia::Color::WHITE);
    }
    let ts = Transform::from_scale(scale, scale);
    for item in &page.items {
        match item {
            Item::Rect { rect, color } if layers.graphics => {
                if let Some(r) = tiny_skia::Rect::from_ltrb(rect.x0, rect.y0, rect.x1, rect.y1) {
                    pm.fill_rect(r, &paint(*color), ts, None);
                }
            }
            Item::Path(shape) if layers.graphics => {
                let Some(path) = build_path(&shape.segments) else { continue };
                if let Some(fill) = shape.fill {
                    pm.fill_path(&path, &paint(fill), FillRule::Winding, ts, None);
                }
                if let Some((color, width)) = shape.stroke {
                    let stroke = Stroke { width: width.max(0.1), ..Default::default() };
                    pm.stroke_path(&path, &paint(color), &stroke, ts, None);
                }
            }
            Item::Image { rect, image } if layers.graphics => {
                if let Some(src) = decode(image) {
                    let sx = rect.width() / src.width() as f32;
                    let sy = rect.height() / src.height() as f32;
                    let t = Transform::from_row(sx, 0.0, 0.0, sy, rect.x0, rect.y0).post_concat(ts);
                    pm.draw_pixmap(0, 0, src.as_ref(), &PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() }, t, None);
                }
            }
            Item::Glyphs(run) if layers.text => {
                let px = run.size * scale;
                for g in &run.glyphs {
                    let x = g.x * scale;
                    let y = (g.y * scale).round();
                    let key = GlyphCache::key(&run.font, g.id, px, x, run.embolden, run.skew.is_some());
                    if let Some(mask) = glyphs.get(&run.font, key) {
                        blit(&mut pm, &mask, x.floor() as i32 + mask.left, y as i32 - mask.top, run.color);
                    }
                }
            }
            _ => {}
        }
    }
    to_image(pm)
}

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0[0], c.0[1], c.0[2], c.0[3]);
    p.anti_alias = true;
    p
}

fn build_path(segs: &[PathSeg]) -> Option<tiny_skia::Path> {
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

pub fn decode(image: &ImageResource) -> Option<Pixmap> {
    let img = image::load_from_memory(&image.bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut data = img.into_raw();
    for px in data.chunks_mut(4) {
        let a = px[3] as u16;
        for c in &mut px[..3] {
            *c = ((*c as u16 * a + 127) / 255) as u8;
        }
    }
    Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(w, h)?)
}

fn blit(pm: &mut Pixmap, mask: &crate::glyphs::Mask, x0: i32, y0: i32, color: Rgba) {
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let [r, g, b, a] = color.0.map(|c| c as u32);
    for my in 0..mask.height as i32 {
        let y = y0 + my;
        if y < 0 || y >= ph {
            continue;
        }
        for mx in 0..mask.width as i32 {
            let x = x0 + mx;
            if x < 0 || x >= pw {
                continue;
            }
            let cov = mask.data[(my * mask.width as i32 + mx) as usize] as u32 * a / 255;
            if cov == 0 {
                continue;
            }
            let i = ((y * pw + x) * 4) as usize;
            let inv = 255 - cov;
            // Premultiplied source-over.
            data[i] = ((r * cov + data[i] as u32 * inv) / 255) as u8;
            data[i + 1] = ((g * cov + data[i + 1] as u32 * inv) / 255) as u8;
            data[i + 2] = ((b * cov + data[i + 2] as u32 * inv) / 255) as u8;
            data[i + 3] = ((255 * cov + data[i + 3] as u32 * inv) / 255) as u8;
        }
    }
}

fn to_image(pm: Pixmap) -> RgbaImage {
    let (w, h) = (pm.width(), pm.height());
    let mut data = pm.take();
    for px in data.chunks_mut(4) {
        let a = px[3] as u32;
        if a > 0 && a < 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(w, h, data).expect("buffer size")
}

/// Visual difference between two renders of equal size. Returns an overlay
/// (red: only in `a`, blue: only in `b`, grey: both) and the fraction of
/// pixels whose luminance differs noticeably.
pub fn diff(a: &RgbaImage, b: &RgbaImage) -> (RgbaImage, f32) {
    let (w, h) = (a.width().min(b.width()), a.height().min(b.height()));
    let mut out = RgbaImage::new(w, h);
    let mut differing = 0usize;
    let lum = |p: &image::Rgba<u8>| (p[0] as u32 * 3 + p[1] as u32 * 6 + p[2] as u32) / 10;
    for y in 0..h {
        for x in 0..w {
            let (la, lb) = (lum(a.get_pixel(x, y)), lum(b.get_pixel(x, y)));
            let (da, db) = (255 - la, 255 - lb);
            if da.abs_diff(db) > 64 {
                differing += 1;
            }
            let px = match (da > 64, db > 64) {
                (true, true) => [110, 110, 110, 255],
                (true, false) => [230, 40, 40, 255],
                (false, true) => [40, 90, 230, 255],
                _ => [255, 255, 255, 255],
            };
            out.put_pixel(x, y, image::Rgba(px));
        }
    }
    (out, differing as f32 / (w * h).max(1) as f32)
}
