use std::path::{Path, PathBuf};
use std::sync::Arc;

use document::{Document, Section, Size};
use editor::Editor;

use crate::capture::Capture;
use crate::view::DocView;
use crate::worker::{Response, Worker};

pub struct App {
    doc: Option<DocView>,
    /// A PDF whose worker has not reported its page list yet.
    opening: Option<Worker>,
    /// Source PDF (if imported) and the `.reflow` file backing the document.
    source_path: Option<PathBuf>,
    save_path: Option<PathBuf>,
    saved_revision: u64,
    status: String,
    show_debug: bool,
    capture: Option<Capture>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        let mut app = App {
            doc: None,
            opening: None,
            source_path: None,
            save_path: None,
            saved_revision: 0,
            status: String::new(),
            show_debug: std::env::var_os("REFLOW_DEBUG").is_some(),
            capture: Capture::from_env(),
        };
        if let Some(path) = file {
            app.open(&cc.egui_ctx, path);
        }
        app
    }

    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        if path.extension().is_some_and(|e| e == "reflow") {
            match Document::load(&path) {
                Ok(doc) => {
                    self.doc = Some(DocView::new(Editor::new(doc), None));
                    self.status = format!("Opened {}", path.display());
                    self.save_path = Some(path);
                    self.source_path = None;
                    self.saved_revision = 0;
                }
                Err(e) => self.status = format!("Cannot open {}: {e}", path.display()),
            }
            return;
        }
        let wake = ctx.clone();
        self.opening = Some(Worker::open(path.clone(), move || wake.request_repaint()));
        self.status = format!("Opening {}…", path.display());
        self.source_path = Some(path);
        self.save_path = None;
        self.doc = None;
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let mut carried = Vec::new();
        if let Some(worker) = &self.opening {
            let mut responses = worker.poll().into_iter();
            while let Some(resp) = responses.next() {
                match resp {
                    Response::Opened { page_sizes } => {
                        // Pages appear immediately as PDFium renders; each is
                        // swapped for its reconstruction when that finishes.
                        let sections = page_sizes
                            .iter()
                            .enumerate()
                            .map(|(i, &(w, h))| {
                                Arc::new(Section { page_size: Size::new(w, h), source_page: Some(i as u32), pending: true, ..Default::default() })
                            })
                            .collect();
                        let worker = self.opening.take().expect("opening worker");
                        self.doc = Some(DocView::new(Editor::new(Document { sections }), Some(worker)));
                        self.saved_revision = 0;
                        self.status = format!("{} pages — reconstructing…", page_sizes.len());
                        carried.extend(responses);
                        break;
                    }
                    Response::Failed(e) => {
                        self.status = e;
                        self.opening = None;
                        break;
                    }
                    _ => {}
                }
            }
        }
        let Some(view) = &mut self.doc else { return };
        carried.extend(view.worker.as_ref().map(|w| w.poll()).unwrap_or_default());
        for resp in carried {
            match resp {
                Response::Rendered { page, scale, image } => view.set_original(ctx, page, scale, image),
                Response::Reconstructed { index, section, analysis } => {
                    let i = index as usize;
                    // Installing a reconstruction is not an edit.
                    let clean = view.editor.revision == self.saved_revision;
                    view.editor.replace_section(i, Arc::new(section));
                    if clean {
                        self.saved_revision = view.editor.revision;
                    }
                    if let Some(slot) = view.analyses.get_mut(i) {
                        *slot = Some(*analysis);
                    }
                    let done = view.editor.doc.sections.iter().filter(|s| !s.pending).count();
                    let total = view.editor.doc.sections.len();
                    self.status = if done == total { format!("{total} pages reconstructed") } else { format!("Reconstructing {done}/{total}…") };
                }
                Response::Failed(e) => self.status = e,
                Response::Opened { .. } => {}
            }
        }
    }

    fn dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.editor.revision != self.saved_revision)
    }

    fn file_stem(&self) -> String {
        self.save_path
            .as_deref()
            .or(self.source_path.as_deref())
            .and_then(Path::file_stem)
            .map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
    }

    fn save(&mut self, save_as: bool) {
        let stem = self.file_stem();
        let Some(view) = &self.doc else { return };
        let path = match (&self.save_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new().add_filter("Reflow document", &["reflow"]).set_file_name(format!("{stem}.reflow")).save_file(),
        };
        let Some(path) = path else { return };
        match view.editor.doc.save(&path) {
            Ok(()) => {
                self.status = format!("Saved {}", path.display());
                self.saved_revision = view.editor.revision;
                self.save_path = Some(path);
            }
            Err(e) => self.status = format!("Save failed: {e}"),
        }
    }

    fn export(&mut self) {
        let stem = self.file_stem();
        let Some(view) = &mut self.doc else { return };
        let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(format!("{stem}-edited.pdf")).save_file() else {
            return;
        };
        view.relayout();
        self.status = match export::export_pdf(&view.layout) {
            Ok(bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => format!("Exported {}", path.display()),
                Err(e) => format!("Export failed: {e}"),
            },
            Err(e) => format!("Export failed: {e}"),
        };
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let cmd = |k| KeyboardShortcut::new(Modifiers::COMMAND, k);
        let cmd_shift = |k| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, k);
        if ctx.input_mut(|i| i.consume_shortcut(&cmd_shift(Key::S))) {
            self.save(true);
        } else if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::S))) {
            self.save(false);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::O))) {
            self.open_dialog(ctx);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::E))) {
            self.export();
        }
        if let Some(view) = &mut self.doc {
            if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::Plus)) || i.consume_shortcut(&cmd(Key::Equals))) {
                view.zoom_by(1.25);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::Minus))) {
                view.zoom_by(0.8);
            }
            if ctx.input_mut(|i| i.consume_shortcut(&cmd(Key::Num0))) {
                view.zoom = 1.0;
            }
        }
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new().add_filter("Documents", &["pdf", "reflow"]).pick_file() {
            self.open(ctx, path);
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open…  ⌘O").clicked() {
                    ui.close();
                    self.open_dialog(ui.ctx());
                }
                let has_doc = self.doc.is_some();
                if ui.add_enabled(has_doc, egui::Button::new("Save  ⌘S")).clicked() {
                    ui.close();
                    self.save(false);
                }
                if ui.add_enabled(has_doc, egui::Button::new("Save As…  ⇧⌘S")).clicked() {
                    ui.close();
                    self.save(true);
                }
                if ui.add_enabled(has_doc, egui::Button::new("Export PDF…  ⌘E")).clicked() {
                    ui.close();
                    self.export();
                }
            });
            if let Some(view) = &mut self.doc {
                ui.menu_button("Edit", |ui| {
                    if ui.add_enabled(view.editor.can_undo(), egui::Button::new("Undo  ⌘Z")).clicked() {
                        view.editor.undo();
                        ui.close();
                    }
                    if ui.add_enabled(view.editor.can_redo(), egui::Button::new("Redo  ⇧⌘Z")).clicked() {
                        view.editor.redo();
                        ui.close();
                    }
                    if ui.button("Select All  ⌘A").clicked() {
                        view.editor.select_all();
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Zoom In  ⌘+").clicked() {
                        view.zoom_by(1.25);
                    }
                    if ui.button("Zoom Out  ⌘−").clicked() {
                        view.zoom_by(0.8);
                    }
                    if ui.button("Actual Size  ⌘0").clicked() {
                        view.zoom = 1.0;
                    }
                    ui.separator();
                    ui.checkbox(&mut self.show_debug, "Reconstruction debugger");
                });
                ui.separator();
                ui.label(format!("{:.0}%", view.zoom * 100.0));
            }
            let dirty = if self.dirty() { " •" } else { "" };
            if self.doc.is_some() {
                ui.label(format!("{}{dirty}", self.file_stem()));
            }
            ui.weak(&self.status);
        });
    }

    fn debug_panel(&mut self, ui: &mut egui::Ui) {
        let Some(view) = &mut self.doc else { return };
        ui.heading("Reconstruction");
        let o = &mut view.overlays;
        ui.checkbox(&mut o.reconstructed, "Reconstructed document");
        ui.checkbox(&mut o.original, "Original PDF render");
        ui.add_enabled(o.original, egui::Slider::new(&mut o.original_opacity, 0.0..=1.0).text("opacity"));
        ui.checkbox(&mut o.diff, "Difference overlay");
        ui.weak("red: original only · blue: reconstruction only");
        ui.separator();
        ui.label("Boxes");
        ui.checkbox(&mut o.glyphs, "Glyphs");
        ui.checkbox(&mut o.words, "Words");
        ui.checkbox(&mut o.lines, "Lines");
        ui.checkbox(&mut o.blocks, "Blocks (paragraphs, rules, images)");
        ui.checkbox(&mut o.zones, "Zones (flows, columns, tables)");
        if !view.diff_scores.is_empty() {
            ui.separator();
            let mut scores: Vec<_> = view.diff_scores.iter().collect();
            scores.sort_by_key(|(p, _)| **p);
            for (p, s) in scores {
                ui.label(format!("page {}: {:.2}% pixels differ", p + 1, s * 100.0));
            }
        }
        ui.separator();
        ui.label("Font substitutions");
        let subs = view.layouter.fonts.substitutions();
        if subs.is_empty() {
            ui.weak("none — all fonts installed");
        }
        for (family, r) in subs {
            ui.label(format!("{family} → {} ({})", if r.family.is_empty() { "system" } else { &r.family }, r.reason));
        }
        let notes: Vec<String> = view
            .analyses
            .iter()
            .enumerate()
            .flat_map(|(i, a)| a.iter().flat_map(|a| &a.notes).map(move |n| format!("page {}: {n}", i + 1)))
            .collect();
        if !notes.is_empty() {
            ui.separator();
            ui.label("Notes");
            for n in notes {
                ui.label(n);
            }
        }
        ui.separator();
        let sel = view.editor.sel;
        ui.weak(format!("caret: paragraph {} offset {}", sel.focus.para, sel.focus.offset));
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.shortcuts(&ctx);
        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        if self.show_debug && self.doc.is_some() {
            egui::Panel::right("debug").default_size(280.0).show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.debug_panel(ui));
            });
        }
        egui::CentralPanel::default().show(ui, |ui| match &mut self.doc {
            Some(view) => view.ui(ui),
            None => {
                ui.centered_and_justified(|ui| ui.label(if self.opening.is_some() { "Opening…" } else { "Open a PDF (⌘O)" }));
            }
        });
        if let Some(capture) = &mut self.capture {
            let ready = self.doc.as_ref().is_some_and(|d| d.editor.doc.sections.iter().all(|s| !s.pending));
            if ready
                && let Some(view) = &mut self.doc
            {
                for step in capture.take_script() {
                    view.script_step(&ctx, &step);
                }
            }
            capture.update(&ctx, ready);
        }
    }
}
