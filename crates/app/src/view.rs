//! The document canvas: paints laid-out pages (virtualised to what is on
//! screen), debug overlays, caret and selection, and turns pointer and
//! keyboard input into editor commands.

use std::collections::HashMap;
use std::sync::Arc;

use editor::{Editor, Move, Pos};
use egui::{Color32, Event, Key, Modifiers, Pos2, Rect, Sense, Stroke, TextureHandle, TextureOptions, Vec2};
use layout::{DocLayout, Item, Layouter};
use reconstruction::PageAnalysis;
use render::GlyphCache;
use render::gpu::{self, Atlas, View};

use crate::worker::{Request, Worker};

/// Gap between pages, in document points.
pub const PAGE_GAP: f32 = 24.0;
pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 6.0;

#[derive(Clone, Copy, PartialEq)]
pub struct Overlays {
    pub reconstructed: bool,
    pub original: bool,
    pub original_opacity: f32,
    pub diff: bool,
    pub glyphs: bool,
    pub words: bool,
    pub lines: bool,
    pub blocks: bool,
    pub zones: bool,
}

impl Default for Overlays {
    fn default() -> Self {
        Self {
            reconstructed: true,
            original: false,
            original_opacity: 0.5,
            diff: false,
            glyphs: false,
            words: false,
            lines: false,
            blocks: false,
            zones: false,
        }
    }
}

struct Bitmap {
    scale: f32,
    texture: TextureHandle,
    pixels: image::RgbaImage,
}

pub struct DocView {
    pub editor: Editor,
    pub layouter: Layouter,
    pub layout: DocLayout,
    layout_rev: u64,
    pub analyses: Vec<Option<PageAnalysis>>,
    pub worker: Option<Worker>,
    pub zoom: f32,
    pub overlays: Overlays,
    pub diff_scores: HashMap<usize, f32>,
    glyphs: GlyphCache,
    atlas: Atlas,
    atlas_tex: Option<(u64, TextureHandle)>,
    images: HashMap<u64, TextureHandle>,
    path_layers: HashMap<usize, (usize, u32, TextureHandle)>,
    originals: HashMap<u32, Bitmap>,
    requested: HashMap<u32, f32>,
    diffs: HashMap<usize, (u64, u32, TextureHandle)>,
    pending_scroll: Option<Vec2>,
    scroll_to_caret: bool,
    /// Set by ⌘K; the toolbar opens its link editor.
    pub link_requested: bool,
    /// Page nearest the middle of the viewport (for the status bar).
    pub current_page: usize,
    canvas_width: f32,
    words: Option<(u64, usize)>,
    caret_epoch: f64,
}

impl DocView {
    pub fn new(editor: Editor, worker: Option<Worker>) -> Self {
        let mut layouter = Layouter::new();
        let layout = layouter.layout(&editor.doc);
        let n = editor.doc.sections.len();
        Self {
            layout_rev: editor.revision,
            editor,
            layouter,
            layout,
            analyses: vec![None; n],
            worker,
            zoom: 1.0,
            overlays: Overlays::default(),
            diff_scores: HashMap::new(),
            glyphs: GlyphCache::default(),
            atlas: Atlas::default(),
            atlas_tex: None,
            images: HashMap::new(),
            path_layers: HashMap::new(),
            originals: HashMap::new(),
            requested: HashMap::new(),
            diffs: HashMap::new(),
            pending_scroll: None,
            scroll_to_caret: false,
            link_requested: false,
            current_page: 0,
            canvas_width: 0.0,
            words: None,
            caret_epoch: 0.0,
        }
    }

    pub fn relayout(&mut self) {
        if self.editor.revision != self.layout_rev {
            self.layout = self.layouter.layout(&self.editor.doc);
            self.layout_rev = self.editor.revision;
        }
    }

    pub fn set_original(&mut self, ctx: &egui::Context, page: u32, scale: f32, pixels: image::RgbaImage) {
        let size = [pixels.width() as usize, pixels.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw());
        let texture = ctx.load_texture(format!("orig-{page}"), color, TextureOptions::LINEAR);
        self.originals.insert(page, Bitmap { scale, texture, pixels });
    }

    /// Development scripting (`REFLOW_SCRIPT`): `click:page:x:y` (page
    /// points), `shiftclick:page:x:y`, `type:text`, `key:Name[+shift|+alt|+cmd]`,
    /// `overlay:original|diff|lines|blocks|zones`, `zoom:factor`.
    pub fn script_step(&mut self, ctx: &egui::Context, step: &str) {
        self.relayout();
        let (cmd, arg) = step.split_once(':').unwrap_or((step, ""));
        match cmd {
            "click" | "shiftclick" => {
                let v: Vec<f32> = arg.split(':').filter_map(|s| s.parse().ok()).collect();
                if let [page, x, y] = v[..]
                    && let Some(h) = self.layout.hit(page as usize, x, y)
                {
                    let p = Pos::new(h.para, h.offset);
                    if cmd == "click" { self.editor.set_caret(p) } else { self.editor.extend_to(p) }
                }
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("canvas")));
            }
            "type" => self.editor.insert_text(&arg.replace("\\n", "\n")),
            "key" => {
                let mut parts = arg.split('+');
                let name = parts.next().unwrap_or_default();
                let mut m = Modifiers::NONE;
                for p in parts {
                    match p {
                        "shift" => m.shift = true,
                        "alt" => m.alt = true,
                        "cmd" => m.command = true,
                        _ => {}
                    }
                }
                if let Some(k) = Key::from_name(name) {
                    self.key(k, m);
                }
            }
            "overlay" => match arg {
                "original" => self.overlays.original = true,
                "diff" => self.overlays.diff = true,
                "lines" => self.overlays.lines = true,
                "blocks" => self.overlays.blocks = true,
                "zones" => self.overlays.zones = true,
                _ => {}
            },
            "zoom" => self.zoom = arg.parse().unwrap_or(self.zoom),
            "theme" => ctx.set_theme(if arg == "light" { egui::ThemePreference::Light } else { egui::ThemePreference::Dark }),
            "action" => {
                let a: Vec<&str> = arg.split(':').collect();
                let e = &mut self.editor;
                match a.as_slice() {
                    ["table", r, c] => {
                        let width = self.layout.para(e.sel.focus.para).width;
                        e.insert_table(r.parse().unwrap_or(2), c.parse().unwrap_or(2), width);
                    }
                    ["shade_row"] => e.table_set_shading(Some(document::Rgba::rgb(217, 234, 211)), true),
                    ["col_right"] => e.table_insert_col(true),
                    ["row_below"] => e.table_insert_row(true),
                    ["heading", l] => e.set_block_type(editor::BlockType::Heading(l.parse().unwrap_or(1))),
                    ["rule"] => e.insert_rule(),
                    _ => {}
                }
            }
            _ => {}
        }
        if std::env::var_os("REFLOW_TRACE").is_some() {
            eprintln!("step {step:?}: sel {:?} paras {}", self.editor.sel, self.editor.para_count());
        }
        self.relayout();
    }

    /// Returns keyboard focus to the page canvas (after toolbar actions).
    pub fn focus_canvas(&mut self, ctx: &egui::Context) {
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("canvas")));
        self.scroll_to_caret = true;
    }

    /// Zoom so the widest page fills the canvas width.
    pub fn fit_width(&mut self) {
        let w = (0..self.layout.page_count()).map(|i| self.layout.page(i).size.w).fold(0.0, f32::max);
        if w > 0.0 && self.canvas_width > 0.0 {
            self.zoom = ((self.canvas_width - 48.0) / (w + 2.0 * PAGE_GAP)).clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    pub fn word_count(&mut self) -> usize {
        let rev = self.editor.revision;
        if let Some((r, n)) = self.words
            && r == rev
        {
            return n;
        }
        let n = (0..self.editor.para_count()).map(|i| self.editor.paragraph(i).text().split_whitespace().count()).sum();
        self.words = Some((rev, n));
        n
    }

    pub fn zoom_by(&mut self, factor: f32) {
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    fn page_rects(&self, width: f32) -> Vec<Rect> {
        let mut y = PAGE_GAP;
        (0..self.layout.page_count())
            .map(|i| {
                let size = self.layout.page(i).size;
                let x = ((width / self.zoom - size.w) * 0.5).max(PAGE_GAP);
                let r = Rect::from_min_size(Pos2::new(x, y) * self.zoom, Vec2::new(size.w, size.h) * self.zoom);
                y += size.h + PAGE_GAP;
                r
            })
            .collect()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.relayout();
        let doc_w = (0..self.layout.page_count()).map(|i| self.layout.page(i).size.w).fold(0.0, f32::max);
        let doc_h: f32 = (0..self.layout.page_count()).map(|i| self.layout.page(i).size.h + PAGE_GAP).sum::<f32>() + PAGE_GAP;
        let content = Vec2::new(doc_w + 2.0 * PAGE_GAP, doc_h) * self.zoom;

        let mut area = egui::ScrollArea::both().auto_shrink(false);
        if let Some(offset) = self.pending_scroll.take() {
            area = area.scroll_offset(offset);
        }
        self.canvas_width = ui.available_width();
        let out = area.show_viewport(ui, |ui, viewport| {
            let size = content.max(ui.available_size());
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let response = ui.interact(rect, egui::Id::new("canvas"), Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            let origin = response.rect.min;
            let rects: Vec<Rect> = self.page_rects(size.x).into_iter().map(|r| r.translate(origin.to_vec2())).collect();
            self.handle_pointer(ui, &response, &rects);
            self.handle_keys(ui, &response);
            let clip = viewport.translate(origin.to_vec2()).expand(100.0);
            let mid = viewport.center().y + origin.y;
            self.current_page = rects.iter().position(|r| r.bottom() + PAGE_GAP * self.zoom >= mid).unwrap_or(0);
            for (i, rect) in rects.iter().enumerate() {
                if rect.intersects(clip) {
                    self.paint_page(ui, &painter, i, *rect, clip);
                }
            }
            if std::mem::take(&mut self.scroll_to_caret)
                && let Some((page, r)) = self.layout.caret(self.editor.sel.focus.para, self.editor.sel.focus.offset)
                && let Some(pr) = rects.get(page)
            {
                let view = View { origin: pr.min, zoom: self.zoom, pixels_per_point: 1.0 };
                ui.scroll_to_rect(view.rect(&r).expand(24.0), None);
            }
            self.upload_atlas(ui.ctx());
            response
        });

        // Cmd+scroll / pinch zoom, anchored at the pointer.
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if zoom_delta != 1.0 && out.inner.hovered() {
            let old = self.zoom;
            self.zoom_by(zoom_delta);
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                let local = pointer - out.inner_rect.min;
                self.pending_scroll = Some((out.state.offset + local) / old * self.zoom - local);
            }
        }
    }

    fn doc_point(&self, rects: &[Rect], p: Pos2) -> Option<(usize, f32, f32)> {
        let i = rects
            .iter()
            .enumerate()
            .min_by(|a, b| dist(a.1, p).total_cmp(&dist(b.1, p)))
            .map(|(i, _)| i)?;
        let local = (p - rects[i].min) / self.zoom;
        Some((i, local.x, local.y))
    }

    fn hit(&self, rects: &[Rect], p: Pos2) -> Option<Pos> {
        let (page, x, y) = self.doc_point(rects, p)?;
        self.layout.hit(page, x, y).map(|h| Pos::new(h.para, h.offset))
    }

    fn handle_pointer(&mut self, ui: &egui::Ui, response: &egui::Response, rects: &[Rect]) {
        let Some(pointer) = response.interact_pointer_pos() else { return };
        let shift = ui.input(|i| i.modifiers.shift);
        if response.triple_clicked() {
            if let Some(p) = self.hit(rects, pointer) {
                self.editor.select_paragraph(p.para);
            }
        } else if response.double_clicked() {
            if let Some(p) = self.hit(rects, pointer) {
                self.editor.select_word_at(p);
            }
        } else if response.drag_started() || response.clicked() || (response.is_pointer_button_down_on() && !response.dragged()) {
            if response.drag_started() || ui.input(|i| i.pointer.any_pressed()) {
                if let Some(p) = self.hit(rects, pointer) {
                    if shift { self.editor.extend_to(p) } else { self.editor.set_caret(p) }
                }
                response.request_focus();
            }
        } else if response.dragged()
            && let Some(p) = self.hit(rects, pointer)
        {
            self.editor.extend_to(p);
        }
        self.caret_epoch = ui.input(|i| i.time);
    }

    fn handle_keys(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if !response.has_focus() {
            return;
        }
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                response.id,
                egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false },
            )
        });
        let events = ui.input(|i| i.events.clone());
        for ev in events {
            let before = (self.editor.revision, self.editor.sel);
            match ev {
                Event::Text(t) => self.editor.insert_text(&t),
                Event::Ime(egui::ImeEvent::Commit(t)) => self.editor.insert_text(&t),
                Event::Paste(t) => self.editor.paste(&t),
                Event::Copy => {
                    let t = self.editor.copy();
                    ui.ctx().copy_text(t);
                }
                Event::Cut => {
                    let t = self.editor.cut();
                    ui.ctx().copy_text(t);
                }
                Event::Key { key, pressed: true, modifiers, .. } => self.key(key, modifiers),
                _ => {}
            }
            if (self.editor.revision, self.editor.sel) != before {
                self.relayout();
                self.scroll_to_caret = true;
                self.caret_epoch = ui.input(|i| i.time);
            }
        }
    }

    fn key(&mut self, key: Key, m: Modifiers) {
        let e = &mut self.editor;
        let layout = &self.layout;
        let extend = m.shift;
        let mv = |e: &mut Editor, mv: Move| e.move_caret(mv, extend, layout);
        match key {
            Key::ArrowLeft if m.command => mv(e, Move::LineStart),
            Key::ArrowRight if m.command => mv(e, Move::LineEnd),
            Key::ArrowUp if m.command => mv(e, Move::DocStart),
            Key::ArrowDown if m.command => mv(e, Move::DocEnd),
            Key::ArrowLeft if m.alt => mv(e, Move::WordLeft),
            Key::ArrowRight if m.alt => mv(e, Move::WordRight),
            Key::ArrowUp if m.alt => mv(e, Move::ParaStart),
            Key::ArrowDown if m.alt => mv(e, Move::ParaEnd),
            Key::ArrowLeft => mv(e, Move::Left),
            Key::ArrowRight => mv(e, Move::Right),
            Key::ArrowUp => mv(e, Move::Up),
            Key::ArrowDown => mv(e, Move::Down),
            Key::Home => mv(e, if m.command { Move::DocStart } else { Move::LineStart }),
            Key::End => mv(e, if m.command { Move::DocEnd } else { Move::LineEnd }),
            Key::Backspace => e.backspace(m.alt),
            Key::Delete => e.delete_forward(m.alt),
            Key::Enter => e.enter(),
            Key::Tab if !m.command => {
                // Tab moves between table cells and nests list items at
                // their start; otherwise it types a tab.
                let at_list_start = e.sel.is_collapsed() && e.sel.focus.offset == 0 && e.current_paragraph_style().list.is_some();
                if !e.table_next_cell(m.shift) {
                    if at_list_start || (m.shift && e.current_paragraph_style().list.is_some()) {
                        e.change_indent(!m.shift);
                    } else if !m.shift {
                        e.insert_text("\t");
                    }
                }
            }
            Key::A if m.command => e.select_all(),
            Key::Z if m.command && m.shift => e.redo(),
            Key::Z if m.command => e.undo(),
            Key::Y if m.command => e.redo(),
            Key::B if m.command => e.toggle_style(|s| s.font.is_bold(), |s, on| s.font.weight = if on { 700 } else { 400 }),
            Key::I if m.command => e.toggle_style(|s| s.font.italic, |s, on| s.font.italic = on),
            Key::U if m.command => e.toggle_style(|s| s.underline, |s, on| s.underline = on),
            Key::X if m.command && m.shift => e.toggle_style(|s| s.strike, |s, on| s.strike = on),
            Key::K if m.command => self.link_requested = true,
            Key::Num7 if m.command && m.shift => e.toggle_list(true),
            Key::Num8 if m.command && m.shift => e.toggle_list(false),
            Key::Num0 if m.command && m.alt => e.set_block_type(editor::BlockType::Normal),
            Key::Num1 if m.command && m.alt => e.set_block_type(editor::BlockType::Heading(1)),
            Key::Num2 if m.command && m.alt => e.set_block_type(editor::BlockType::Heading(2)),
            Key::Num3 if m.command && m.alt => e.set_block_type(editor::BlockType::Heading(3)),
            Key::OpenBracket if m.command => e.change_indent(false),
            Key::CloseBracket if m.command => e.change_indent(true),
            Key::L if m.command && m.shift => e.set_align(document::Align::Left),
            Key::E if m.command && m.shift => e.set_align(document::Align::Center),
            Key::R if m.command && m.shift => e.set_align(document::Align::Right),
            Key::J if m.command && m.shift => e.set_align(document::Align::Justify),
            Key::Backslash if m.command => e.clear_formatting(),
            _ => {}
        }
    }

    fn upload_atlas(&mut self, ctx: &egui::Context) {
        let generation = self.atlas.generation;
        let stale = self.atlas_tex.as_ref().is_none_or(|(g, _)| *g != generation);
        if stale {
            let size = gpu::ATLAS_SIZE as usize;
            let img = egui::ColorImage::new([size, size], vec![Color32::TRANSPARENT; size * size]);
            let tex = ctx.load_texture("glyph-atlas", img, TextureOptions::NEAREST);
            self.atlas_tex = Some((generation, tex));
            self.atlas.dirty = Some((0, 0, gpu::ATLAS_SIZE, gpu::ATLAS_SIZE));
        }
        if let Some((pos, size, pixels)) = self.atlas.take_dirty()
            && let Some((_, tex)) = &mut self.atlas_tex
        {
            let img = egui::ColorImage::new(size, pixels);
            tex.set_partial(pos, img, TextureOptions::NEAREST);
        }
    }

    fn paint_page(&mut self, ui: &egui::Ui, painter: &egui::Painter, i: usize, rect: Rect, clip: Rect) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let view = View { origin: rect.min, zoom: self.zoom, pixels_per_point: ppp };
        // Two soft shadows (ambient + contact) read as paper above the canvas.
        let dark = ui.visuals().dark_mode;
        let ambient = egui::epaint::Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(if dark { 120 } else { 26 }) };
        let contact = egui::epaint::Shadow { offset: [0, 1], blur: 3, spread: 0, color: Color32::from_black_alpha(if dark { 140 } else { 30 }) };
        painter.add(ambient.as_shape(rect, 1.0));
        painter.add(contact.as_shape(rect, 1.0));
        painter.rect_filled(rect, 1.0, Color32::WHITE);

        let page = self.layout.page(i);
        let section = page.section;
        let pending = self.editor.doc.sections[section].pending;
        let source_page = (!page.continuation).then_some(section as u32);
        let want_original = pending || self.overlays.original || self.overlays.diff;
        if let (true, Some(sp)) = (want_original, source_page) {
            self.request_original(sp, (self.zoom * ppp).min(4.0));
        }
        let full_uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));

        if let Some(bmp) = source_page.and_then(|sp| self.originals.get(&sp))
            && (pending || (self.overlays.original && !self.overlays.diff))
        {
            let alpha = if pending || !self.overlays.reconstructed { 1.0 } else { self.overlays.original_opacity };
            painter.image(bmp.texture.id(), rect, full_uv, Color32::WHITE.gamma_multiply(alpha));
        }

        if self.overlays.diff && !pending {
            if let Some(tex) = self.diff_texture(&ctx, i) {
                painter.image(tex, rect, full_uv, Color32::WHITE);
            }
        } else if self.overlays.reconstructed && !pending {
            self.paint_content(&ctx, painter, i, view, clip);
        }

        self.paint_selection_and_caret(ui, painter, i, view);
        self.paint_debug(painter, i, view);
    }

    fn paint_content(&mut self, ctx: &egui::Context, painter: &egui::Painter, i: usize, view: View, clip: Rect) {
        let page = self.layout.page(i);
        let full_uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        // Vector decorations: rasterized once per zoom level.
        if page.items.iter().any(|it| matches!(it, Item::Path(_))) {
            let scale = (view.zoom * view.pixels_per_point).min(4.0);
            let scale_q = (scale * 8.0).round() as u32;
            let key = Arc::as_ptr(&self.layout.sections[page.section]) as usize;
            let fresh = self.path_layers.get(&i).is_some_and(|(k, s, _)| *k == key && *s == scale_q);
            if !fresh {
                let mut only_paths = page.clone();
                only_paths.items.retain(|it| matches!(it, Item::Path(_)));
                let layers = render::raster::Layers { text: false, graphics: true, background: false };
                let img = render::raster::render_page(&only_paths, scale_q as f32 / 8.0, &mut self.glyphs, &layers);
                let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                let tex = ctx.load_texture(format!("paths-{i}"), color, TextureOptions::LINEAR);
                self.path_layers.insert(i, (key, scale_q, tex));
            }
            if let Some((_, _, tex)) = self.path_layers.get(&i) {
                painter.image(tex.id(), view.rect(&document::Rect::new(0.0, 0.0, page.size.w, page.size.h)), full_uv, Color32::WHITE);
            }
        }
        for item in &page.items {
            if let Item::Image { rect, image } = item {
                let tex = self.images.entry(image.id).or_insert_with(|| {
                    let color = render::raster::decode(image).map_or_else(
                        || egui::ColorImage::new([1, 1], vec![Color32::LIGHT_GRAY]),
                        |pm| {
                            let px = pm.pixels().iter().map(|p| Color32::from_rgba_premultiplied(p.red(), p.green(), p.blue(), p.alpha())).collect();
                            egui::ColorImage::new([pm.width() as usize, pm.height() as usize], px)
                        },
                    );
                    ctx.load_texture(format!("img-{}", image.id), color, TextureOptions::LINEAR)
                });
                painter.image(tex.id(), view.rect(rect), full_uv, Color32::WHITE);
            }
        }
        painter.add(gpu::rect_mesh(page, view));
        let tex = self.atlas_tex.as_ref().map(|(_, t)| t.id()).unwrap_or_default();
        match gpu::text_mesh(page, view, &mut self.atlas, &mut self.glyphs, tex, clip) {
            Some(mesh) => {
                painter.add(mesh);
            }
            None => ctx.request_repaint(),
        }
    }

    fn paint_selection_and_caret(&self, ui: &egui::Ui, painter: &egui::Painter, i: usize, view: View) {
        let sel = self.editor.sel;
        let (a, b) = sel.ordered();
        if !sel.is_collapsed() {
            let fill = Color32::from_rgba_unmultiplied(66, 133, 244, 64);
            for para in a.para..=b.para.min(self.layout.para_count().saturating_sub(1)) {
                let pl = self.layout.para(para);
                let on_page = pl.lines.iter().any(|l| self.layout.line_page(para, l) == i);
                if !on_page {
                    continue;
                }
                let len = self.editor.paragraph(para).len();
                let from = if para == a.para { a.offset } else { 0 };
                let to = if para == b.para { b.offset } else { len + 1 };
                for (page, r) in self.layout.selection_rects(para, from..to) {
                    if page == i {
                        painter.rect_filled(view.rect(&r), 0.0, fill);
                    }
                }
            }
        }
        let focused = ui.memory(|m| m.focused().is_some());
        if focused && sel.focus.para < self.layout.para_count()
            && let Some((page, r)) = self.layout.caret(sel.focus.para, sel.focus.offset)
            && page == i
        {
            let t = ui.input(|i| i.time) - self.caret_epoch;
            if (t % 1.06) < 0.62 {
                let r = view.rect(&r);
                painter.line_segment([r.min, r.max], Stroke::new(2.0, Color32::from_rgb(24, 24, 28)));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    fn paint_debug(&self, painter: &egui::Painter, i: usize, view: View) {
        let o = self.overlays;
        if !(o.glyphs || o.words || o.lines || o.blocks || o.zones) {
            return;
        }
        let page = self.layout.page(i);
        if page.continuation {
            return;
        }
        let Some(Some(a)) = self.analyses.get(page.section) else { return };
        let draw = |rects: &mut dyn Iterator<Item = &document::Rect>, color: Color32| {
            for r in rects {
                painter.rect_stroke(view.rect(r), 0.0, Stroke::new(1.0, color), egui::StrokeKind::Middle);
            }
        };
        if o.glyphs {
            draw(&mut a.glyphs.iter(), Color32::from_rgba_unmultiplied(0, 160, 0, 120));
        }
        if o.words {
            draw(&mut a.words.iter(), Color32::from_rgba_unmultiplied(0, 90, 255, 160));
        }
        if o.lines {
            draw(&mut a.lines.iter(), Color32::from_rgba_unmultiplied(255, 140, 0, 200));
        }
        if o.blocks {
            for (r, kind) in &a.blocks {
                let c = match *kind {
                    "rule" => Color32::from_rgb(150, 0, 150),
                    "image" => Color32::from_rgb(0, 150, 150),
                    _ => Color32::from_rgb(220, 0, 60),
                };
                painter.rect_stroke(view.rect(&r.inflate(1.5)), 0.0, Stroke::new(1.2, c), egui::StrokeKind::Middle);
            }
        }
        if o.zones {
            for (r, kind) in &a.zones {
                let c = match *kind {
                    "columns" | "column" => Color32::from_rgb(0, 170, 90),
                    "table" => Color32::from_rgb(170, 90, 0),
                    _ => Color32::from_rgba_unmultiplied(120, 120, 120, 160),
                };
                let sr = view.rect(&r.inflate(4.0));
                painter.rect_stroke(sr, 2.0, Stroke::new(1.5, c), egui::StrokeKind::Middle);
                painter.text(sr.left_top(), egui::Align2::LEFT_BOTTOM, *kind, egui::FontId::monospace(10.0), c);
            }
        }
    }

    fn request_original(&mut self, page: u32, scale: f32) {
        let have = self.originals.get(&page).map(|b| b.scale).or(self.requested.get(&page).copied());
        if have.is_none_or(|s| (s / scale - 1.0).abs() > 0.15)
            && let Some(w) = &self.worker
        {
            self.requested.insert(page, scale);
            w.request(Request::Render { page, scale });
        }
    }

    /// Original-vs-reconstruction difference overlay for a page.
    fn diff_texture(&mut self, ctx: &egui::Context, i: usize) -> Option<egui::TextureId> {
        let page = self.layout.page(i);
        let orig = self.originals.get(&(page.section as u32))?;
        let scale_q = (orig.scale * 8.0).round() as u32;
        if let Some((rev, s, tex)) = self.diffs.get(&i)
            && *rev == self.layout_rev
            && *s == scale_q
        {
            return Some(tex.id());
        }
        let recon = render::raster::render_page(page, orig.scale, &mut self.glyphs, &render::raster::ALL);
        let (img, score) = render::raster::diff(&orig.pixels, &recon);
        self.diff_scores.insert(i, score);
        let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let tex = ctx.load_texture(format!("diff-{i}"), color, TextureOptions::LINEAR);
        let id = tex.id();
        self.diffs.insert(i, (self.layout_rev, scale_q, tex));
        Some(id)
    }
}

fn dist(r: &Rect, p: Pos2) -> f32 {
    let dx = (r.min.x - p.x).max(p.x - r.max.x).max(0.0);
    let dy = (r.min.y - p.y).max(p.y - r.max.y).max(0.0);
    dx + dy
}
