//! PDFium-backed implementation of the source layer.
//!
//! PDFium is not reentrant; every call goes through one process-wide
//! instance (pdfium-render's `thread_safe` feature serialises access). The app
//! additionally confines a document to a single worker thread.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use document::{PathSeg, Point, Rect, Rgba};
use image::RgbaImage;
use pdfium_render::prelude::*;

use crate::fontname::parse_font_name;
use crate::model::*;

static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();

/// Locations searched for the PDFium dynamic library, in order.
fn library_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(p) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        dirs.push(PathBuf::from(p));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
        dirs.push(dir.join("../Frameworks"));
        dirs.push(dir.join("../lib"));
    }
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pdfium/lib"));
    dirs
}

pub fn pdfium() -> Result<&'static Pdfium, String> {
    PDFIUM
        .get_or_init(|| {
            for dir in library_candidates() {
                let path = Pdfium::pdfium_platform_library_name_at_path(&dir);
                if path.exists()
                    && let Ok(bindings) = Pdfium::bind_to_library(&path)
                {
                    return Ok(Pdfium::new(bindings));
                }
            }
            Pdfium::bind_to_system_library().map(Pdfium::new).map_err(|e| {
                format!("PDFium library not found ({e}). Run scripts/fetch-pdfium.sh or set PDFIUM_DYNAMIC_LIB_PATH.")
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub struct PdfSource {
    doc: PdfDocument<'static>,
}

impl PdfSource {
    pub fn open(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_bytes(bytes)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, String> {
        let doc = pdfium()?.load_pdf_from_byte_vec(bytes, None).map_err(|e| format!("cannot open PDF: {e}"))?;
        Ok(Self { doc })
    }

    pub fn page_count(&self) -> u32 {
        self.doc.pages().len() as u32
    }

    pub fn page_size(&self, index: u32) -> Result<(f32, f32), String> {
        let page = self.page(index)?;
        Ok((page.width().value, page.height().value))
    }

    fn page(&self, index: u32) -> Result<PdfPage<'_>, String> {
        self.doc.pages().get(index as i32).map_err(|e| format!("page {index}: {e}"))
    }

    /// Renders the page with PDFium at `scale` pixels per point.
    pub fn render(&self, index: u32, scale: f32) -> Result<RgbaImage, String> {
        let page = self.page(index)?;
        let w = (page.width().value * scale).round().max(1.0) as i32;
        let h = (page.height().value * scale).round().max(1.0) as i32;
        let config = PdfRenderConfig::new().set_target_size(w, h).render_form_data(true).render_annotations(true);
        let bitmap = page.render_with_config(&config).map_err(|e| e.to_string())?;
        Ok(bitmap.as_image().map_err(|e| e.to_string())?.to_rgba8())
    }

    /// Bytes of the attachment called `name`, if the PDF embeds one.
    pub fn attachment(&self, name: &str) -> Option<Vec<u8>> {
        self.doc.attachments().iter().find(|a| a.name() == name).and_then(|a| a.save_to_bytes().ok())
    }

    /// Stable hash (FNV-1a) of every page's text as extracted, whitespace
    /// excluded. Identical for a PDF and any byte-identical copy; changes
    /// when another program edits the text.
    pub fn text_fingerprint(&self) -> Result<u64, String> {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for i in 0..self.page_count() {
            let page = self.page(i)?;
            let text = page.text().map_err(|e| e.to_string())?;
            for c in text.chars().iter() {
                let Some(ch) = c.unicode_char() else { continue };
                if ch.is_whitespace() || c.is_generated().unwrap_or(false) {
                    continue;
                }
                let mut buf = [0u8; 4];
                for b in ch.encode_utf8(&mut buf).bytes() {
                    h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
                }
            }
            h = (h ^ 0xff).wrapping_mul(0x0100_0000_01b3);
        }
        Ok(h)
    }

    /// A PNG crop of the rendered page (used to preserve content that
    /// cannot be reconstructed, such as rotated text).
    pub fn render_crop(&self, index: u32, rect: Rect, scale: f32) -> Result<(Vec<u8>, u32, u32), String> {
        let page = self.render(index, scale)?;
        let x = (rect.x0 * scale).floor().max(0.0) as u32;
        let y = (rect.y0 * scale).floor().max(0.0) as u32;
        let w = ((rect.width() * scale).ceil() as u32).min(page.width().saturating_sub(x)).max(1);
        let h = ((rect.height() * scale).ceil() as u32).min(page.height().saturating_sub(y)).max(1);
        let crop = image::imageops::crop_imm(&page, x, y, w, h).to_image();
        let mut png = Vec::new();
        crop.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        Ok((png, w, h))
    }

    pub fn extract(&self, index: u32) -> Result<SourcePage, String> {
        let page = self.page(index)?;
        let width = page.width().value;
        let height = page.height().value;
        // Page space -> top-left origin, relative to the crop box.
        let (ox, oy) = page
            .boundaries()
            .crop()
            .or_else(|_| page.boundaries().media())
            .map(|b| (b.bounds.left().value, b.bounds.top().value))
            .unwrap_or((0.0, height));
        let map = Mapper { ox, oy };

        let mut out = SourcePage { index, width, height, ..Default::default() };
        extract_glyphs(&page, &map, &mut out)?;
        for object in page.objects().iter() {
            extract_object(&self.doc, &object, Affine::IDENTITY, &map, &mut out);
        }
        for link in page.links().iter() {
            if let (Ok(r), Some(PdfAction::Uri(action))) = (link.rect(), link.action())
                && let Ok(uri) = action.uri()
            {
                out.links.push(SourceLink { rect: map.rect(&r), uri });
            }
        }
        Ok(out)
    }
}

#[derive(Clone, Copy)]
struct Mapper {
    ox: f32,
    oy: f32,
}

impl Mapper {
    fn point(&self, x: f32, y: f32) -> Point {
        Point::new(x - self.ox, self.oy - y)
    }

    fn rect(&self, r: &PdfRect) -> Rect {
        Rect::new(r.left().value - self.ox, self.oy - r.top().value, r.right().value - self.ox, self.oy - r.bottom().value)
    }
}

fn color(c: PdfColor) -> Rgba {
    Rgba([c.red(), c.green(), c.blue(), c.alpha()])
}

fn extract_glyphs(page: &PdfPage, map: &Mapper, out: &mut SourcePage) -> Result<(), String> {
    let text = page.text().map_err(|e| e.to_string())?;
    let mut font_ids: HashMap<(String, u16, bool), FontId> = HashMap::new();

    for c in text.chars().iter() {
        let Some(ch) = c.unicode_char() else { continue };
        if ch.is_control() && ch != '\t' {
            continue;
        }
        let generated = c.is_generated().unwrap_or(false);
        let (Ok(tight), Ok(loose), Ok((x, y))) = (c.tight_bounds(), c.loose_bounds(), c.origin()) else {
            continue;
        };
        let mode = c.render_mode().ok();
        if matches!(mode, Some(PdfPageTextRenderMode::Invisible | PdfPageTextRenderMode::InvisibleClipping)) {
            continue;
        }
        let stroked = matches!(mode, Some(PdfPageTextRenderMode::FilledThenStroked | PdfPageTextRenderMode::FilledThenStrokedClipping));

        let name = c.font_name();
        let pdfium_weight = c.font_weight().map(weight_value);
        let bold_flag = c.font_is_bold_reenforced();
        let key = (name.clone(), pdfium_weight.unwrap_or(0), bold_flag);
        let font = *font_ids.entry(key).or_insert_with(|| {
            let parsed = parse_font_name(&name);
            let weight = parsed.weight.or(pdfium_weight.filter(|w| *w >= 600)).unwrap_or(400);
            out.fonts.push(SourceFont {
                family: parsed.family,
                weight: if bold_flag { weight.max(700) } else { weight },
                italic: parsed.italic || c.font_is_italic(),
                serif: c.font_is_serif(),
                fixed_pitch: c.font_is_fixed_pitch(),
                symbolic: c.font_is_symbolic(),
                name,
            });
            (out.fonts.len() - 1) as FontId
        });

        let origin = map.point(x.value, y.value);
        out.glyphs.push(SourceGlyph {
            ch,
            bbox: map.rect(&tight),
            loose: map.rect(&loose),
            origin_x: origin.x,
            baseline: origin.y,
            font,
            size: c.scaled_font_size().value,
            color: c.fill_color().map(color).unwrap_or(Rgba::BLACK),
            angle: c.angle_degrees().unwrap_or(0.0),
            generated,
            stroked,
        });
    }
    Ok(())
}

fn weight_value(w: PdfFontWeight) -> u16 {
    match w {
        PdfFontWeight::Weight100 => 100,
        PdfFontWeight::Weight200 => 200,
        PdfFontWeight::Weight300 => 300,
        PdfFontWeight::Weight400Normal => 400,
        PdfFontWeight::Weight500 => 500,
        PdfFontWeight::Weight600 => 600,
        PdfFontWeight::Weight700Bold => 700,
        PdfFontWeight::Weight800 => 800,
        PdfFontWeight::Weight900 => 900,
        PdfFontWeight::Custom(v) => v as u16,
    }
}

/// 2D affine transform `[a b c d e f]` in PDF convention.
#[derive(Clone, Copy)]
struct Affine([f32; 6]);

impl Affine {
    const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn from_pdf(m: &PdfMatrix) -> Self {
        Affine([m.a(), m.b(), m.c(), m.d(), m.e(), m.f()])
    }

    /// `self` applied first, then `outer`.
    fn then(&self, outer: &Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [oa, ob, oc, od, oe, of] = outer.0;
        Affine([
            a * oa + b * oc,
            a * ob + b * od,
            c * oa + d * oc,
            c * ob + d * od,
            e * oa + f * oc + oe,
            e * ob + f * od + of,
        ])
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.0;
        (a * x + c * y + e, b * x + d * y + f)
    }
}

fn extract_object(doc: &PdfDocument, object: &PdfPageObject, parent: Affine, map: &Mapper, out: &mut SourcePage) {
    let Ok(m) = object.matrix() else { return };
    let ctm = Affine::from_pdf(&m).then(&parent);
    match object {
        PdfPageObject::XObjectForm(form) => {
            for child in form.iter() {
                extract_object(doc, &child, ctm, map, out);
            }
        }
        PdfPageObject::Image(img) => {
            let corners = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(x, y)| {
                let (px, py) = ctm.apply(x, y);
                map.point(px, py)
            });
            let rect = corners.iter().fold(Rect::EMPTY, |r, p| r.union(&Rect::new(p.x, p.y, p.x, p.y)));
            let Ok(pixels) = img.get_processed_image(doc).or_else(|_| img.get_raw_image()) else { return };
            let rgba = pixels.to_rgba8();
            let mut png = Vec::new();
            if rgba.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
                out.images.push(SourceImage { rect, width_px: rgba.width(), height_px: rgba.height(), png });
            }
        }
        PdfPageObject::Path(path) => {
            let mut segments = Vec::new();
            let mut pending: Vec<Point> = Vec::new();
            for seg in path.segments().iter() {
                let (px, py) = ctm.apply(seg.x().value, seg.y().value);
                let p = map.point(px, py);
                match seg.segment_type() {
                    PdfPathSegmentType::MoveTo => segments.push(PathSeg::MoveTo(p)),
                    PdfPathSegmentType::LineTo => segments.push(PathSeg::LineTo(p)),
                    PdfPathSegmentType::BezierTo => {
                        pending.push(p);
                        if pending.len() == 3 {
                            segments.push(PathSeg::CurveTo(pending[0], pending[1], pending[2]));
                            pending.clear();
                        }
                    }
                    PdfPathSegmentType::Unknown => {}
                }
                if seg.is_close() {
                    segments.push(PathSeg::Close);
                }
            }
            if segments.is_empty() {
                return;
            }
            let filled = path.fill_mode().is_ok_and(|m| m != PdfPathFillMode::None);
            let stroked = path.is_stroked().unwrap_or(false);
            let scale = (ctm.0[0] * ctm.0[3] - ctm.0[1] * ctm.0[2]).abs().sqrt();
            let fill = filled.then(|| path.fill_color().map(color).ok()).flatten().filter(|c| c.0[3] > 0);
            let stroke = stroked
                .then(|| path.stroke_color().map(color).ok().map(|c| (c, path.stroke_width().map(|w| w.value).unwrap_or(1.0) * scale)))
                .flatten()
                .filter(|(c, _)| c.0[3] > 0);
            if fill.is_none() && stroke.is_none() {
                return;
            }
            let mut bounds = Rect::EMPTY;
            for s in &segments {
                match *s {
                    PathSeg::MoveTo(p) | PathSeg::LineTo(p) => bounds = bounds.union(&Rect::new(p.x, p.y, p.x, p.y)),
                    PathSeg::CurveTo(a, b, c) => {
                        for p in [a, b, c] {
                            bounds = bounds.union(&Rect::new(p.x, p.y, p.x, p.y));
                        }
                    }
                    PathSeg::Close => {}
                }
            }
            out.paths.push(SourcePath { bounds, segments, fill, stroke });
        }
        _ => {}
    }
}
