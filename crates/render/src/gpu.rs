//! GPU text rendering: glyph masks packed into an alpha atlas, emitted as
//! textured triangle meshes (drawn by egui's wgpu renderer). Glyphs are
//! rasterized at the exact device-pixel size for the current zoom, with
//! quarter-pixel horizontal positioning and whole-pixel baselines.

use std::collections::HashMap;

use epaint::{Color32, Mesh, Pos2, Rect, TextureId, Vec2};
use etagere::{AtlasAllocator, size2};
use layout::{Item, PageLayout};

use crate::glyphs::{GlyphCache, GlyphKey};

pub const ATLAS_SIZE: u32 = 2048;

#[derive(Clone, Copy)]
struct Entry {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    left: i32,
    top: i32,
}

pub struct Atlas {
    alloc: AtlasAllocator,
    /// Coverage per texel.
    pub pixels: Vec<u8>,
    entries: HashMap<GlyphKey, Option<Entry>>,
    /// Texel rectangle modified since the last upload `(x0, y0, x1, y1)`.
    pub dirty: Option<(u32, u32, u32, u32)>,
    /// Bumped when the atlas is cleared (full re-upload needed).
    pub generation: u64,
}

impl Default for Atlas {
    fn default() -> Self {
        Self {
            alloc: AtlasAllocator::new(size2(ATLAS_SIZE as i32, ATLAS_SIZE as i32)),
            pixels: vec![0; (ATLAS_SIZE * ATLAS_SIZE) as usize],
            entries: HashMap::new(),
            dirty: None,
            generation: 0,
        }
    }
}

impl Atlas {
    fn clear(&mut self) {
        *self = Atlas { generation: self.generation + 1, ..Default::default() };
        self.dirty = Some((0, 0, ATLAS_SIZE, ATLAS_SIZE));
    }

    fn entry(&mut self, cache: &mut GlyphCache, font: &parley::FontData, key: GlyphKey) -> Option<Entry> {
        if let Some(e) = self.entries.get(&key) {
            return *e;
        }
        let mask = cache.get(font, key);
        let entry = mask.and_then(|m| {
            let alloc = self.alloc.allocate(size2(m.width as i32 + 2, m.height as i32 + 2)).or_else(|| {
                // Full: start over. Zoom changes are the usual cause.
                self.clear();
                self.alloc.allocate(size2(m.width as i32 + 2, m.height as i32 + 2))
            })?;
            let (x, y) = (alloc.rectangle.min.x as u32 + 1, alloc.rectangle.min.y as u32 + 1);
            for row in 0..m.height {
                let dst = ((y + row) * ATLAS_SIZE + x) as usize;
                let src = (row * m.width) as usize;
                self.pixels[dst..dst + m.width as usize].copy_from_slice(&m.data[src..src + m.width as usize]);
            }
            let d = self.dirty.get_or_insert((x, y, x + m.width, y + m.height));
            *d = (d.0.min(x), d.1.min(y), d.2.max(x + m.width), d.3.max(y + m.height));
            Some(Entry { x, y, w: m.width, h: m.height, left: m.left, top: m.top })
        });
        self.entries.insert(key, entry);
        entry
    }

    /// Dirty region as RGBA (premultiplied white with coverage alpha).
    pub fn take_dirty(&mut self) -> Option<([usize; 2], [usize; 2], Vec<Color32>)> {
        let (x0, y0, x1, y1) = self.dirty.take()?;
        let mut px = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let a = self.pixels[(y * ATLAS_SIZE + x) as usize];
                px.push(Color32::from_rgba_premultiplied(a, a, a, a));
            }
        }
        Some(([x0 as usize, y0 as usize], [(x1 - x0) as usize, (y1 - y0) as usize], px))
    }
}

/// Maps page points to screen points.
#[derive(Clone, Copy)]
pub struct View {
    pub origin: Pos2,
    /// Screen points per page point.
    pub zoom: f32,
    pub pixels_per_point: f32,
}

impl View {
    pub fn to_screen(&self, x: f32, y: f32) -> Pos2 {
        self.origin + Vec2::new(x, y) * self.zoom
    }

    pub fn rect(&self, r: &document::Rect) -> Rect {
        Rect::from_min_max(self.to_screen(r.x0, r.y0), self.to_screen(r.x1, r.y1))
    }
}

fn color(c: document::Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

/// Builds the text mesh for a page. Returns `None` if the atlas was reset
/// mid-build (the caller should rebuild next frame after re-uploading).
pub fn text_mesh(page: &PageLayout, view: View, atlas: &mut Atlas, cache: &mut GlyphCache, tex: TextureId, clip: Rect) -> Option<Mesh> {
    let generation = atlas.generation;
    let mut mesh = Mesh::with_texture(tex);
    let ppp = view.pixels_per_point;
    let inv = 1.0 / ATLAS_SIZE as f32;
    for item in &page.items {
        let Item::Glyphs(run) = item else { continue };
        let px_size = run.size * view.zoom * ppp;
        if px_size < 1.0 {
            continue;
        }
        let tint = color(run.color);
        for g in &run.glyphs {
            let p = view.to_screen(g.x, g.y);
            if p.y < clip.min.y - px_size || p.y > clip.max.y + px_size || p.x > clip.max.x || p.x < clip.min.x - 4.0 * px_size {
                continue;
            }
            let (dx, dy) = (p.x * ppp, (p.y * ppp).round());
            let key = GlyphCache::key(&run.font, g.id, px_size, dx, run.embolden, run.skew.is_some());
            let Some(e) = atlas.entry(cache, &run.font, key) else { continue };
            if atlas.generation != generation {
                return None;
            }
            let x0 = (dx.floor() as i32 + e.left) as f32 / ppp;
            let y0 = (dy as i32 - e.top) as f32 / ppp;
            let rect = Rect::from_min_size(Pos2::new(x0, y0), Vec2::new(e.w as f32, e.h as f32) / ppp);
            let uv = Rect::from_min_max(
                Pos2::new(e.x as f32 * inv, e.y as f32 * inv),
                Pos2::new((e.x + e.w) as f32 * inv, (e.y + e.h) as f32 * inv),
            );
            mesh.add_rect_with_uv(rect, uv, tint);
        }
    }
    Some(mesh)
}

/// Filled rectangles (rules, borders, underlines) as a flat-colour mesh.
pub fn rect_mesh(page: &PageLayout, view: View) -> Mesh {
    let mut mesh = Mesh::default();
    for item in &page.items {
        if let Item::Rect { rect, color: c } = item {
            let mut r = view.rect(rect);
            // Keep hairlines visible at low zoom.
            let min = 1.0 / view.pixels_per_point;
            if r.height() < min {
                r.max.y = r.min.y + min;
            }
            if r.width() < min {
                r.max.x = r.min.x + min;
            }
            mesh.add_colored_rect(r, color(*c));
        }
    }
    mesh
}
