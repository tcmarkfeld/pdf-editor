//! Glyph rasterization (swash), shared by the GPU atlas and CPU renderer.

use std::collections::HashMap;
use std::sync::Arc;

use parley::FontData;
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Format, Transform, Vector};

/// Sub-pixel horizontal positions per pixel.
pub const SUBPIXEL_BINS: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font: u64,
    pub index: u32,
    pub glyph: u32,
    /// Size in 1/4 device pixels.
    pub size_q: u32,
    pub subpixel: u32,
    pub embolden: bool,
    pub skew: bool,
}

pub struct Mask {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    /// 8-bit coverage, row-major.
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct GlyphCache {
    ctx: ScaleContext,
    masks: HashMap<GlyphKey, Option<Arc<Mask>>>,
}

impl GlyphCache {
    pub fn key(font: &FontData, glyph: u32, size_px: f32, x: f32, embolden: bool, skew: bool) -> GlyphKey {
        let frac = x - x.floor();
        GlyphKey {
            font: font.data.id(),
            index: font.index,
            glyph,
            size_q: (size_px * 4.0).round() as u32,
            subpixel: ((frac * SUBPIXEL_BINS as f32).round() as u32) % SUBPIXEL_BINS,
            embolden,
            skew,
        }
    }

    pub fn get(&mut self, font: &FontData, key: GlyphKey) -> Option<Arc<Mask>> {
        if let Some(m) = self.masks.get(&key) {
            return m.clone();
        }
        let mask = self.rasterize(font, key).map(Arc::new);
        self.masks.insert(key, mask.clone());
        mask
    }

    fn rasterize(&mut self, font: &FontData, key: GlyphKey) -> Option<Mask> {
        let font_ref = swash::FontRef::from_index(font.data.data(), font.index as usize)?;
        let size = key.size_q as f32 / 4.0;
        let mut scaler = self.ctx.builder(font_ref).size(size).hint(false).build();
        let mut render = Render::new(&[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline]);
        render
            .format(Format::Alpha)
            .offset(Vector::new(key.subpixel as f32 / SUBPIXEL_BINS as f32, 0.0));
        if key.embolden {
            render.embolden(size * 0.03);
        }
        if key.skew {
            render.transform(Some(Transform::skew(swash::zeno::Angle::from_degrees(14.0), swash::zeno::Angle::ZERO)));
        }
        let img = render.render(&mut scaler, key.glyph as u16)?;
        if img.placement.width == 0 || img.placement.height == 0 {
            return None;
        }
        let data = if img.data.len() == (img.placement.width * img.placement.height) as usize {
            img.data
        } else {
            // Colour bitmaps come back RGBA; keep alpha as coverage.
            img.data.chunks(4).map(|c| c[3]).collect()
        };
        Some(Mask { left: img.placement.left, top: img.placement.top, width: img.placement.width, height: img.placement.height, data })
    }
}
