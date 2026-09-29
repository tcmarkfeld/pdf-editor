use std::path::{Path, PathBuf};
use std::sync::Arc;

use document::{Document, Section, Size};
use editor::Editor;

use egui::{Align2, Color32, FontId, Frame, Margin, RichText, Sense, Stroke, Vec2, vec2};

use crate::capture::Capture;
use crate::icons::{self, Icon};
use crate::theme;
use crate::toolbar::{icon_button, primary_button, sized_icon_button};
use crate::view::DocView;
use crate::worker::{Response, Worker};

/// Everything the menus, header buttons and shortcuts can ask for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Link,
    Bold,
    Italic,
    Underline,
    Strike,
    /// 0 = Normal text.
    Heading(u8),
    Bullets,
    Numbers,
    Indent,
    Outdent,
    Align(document::Align),
    InsertRule,
    ClearFormatting,
    ZoomIn,
    ZoomOut,
    ActualSize,
    FitWidth,
    ToggleInspector,
    Appearance(egui::ThemePreference),
}

/// Space left of the header for the window's traffic-light buttons.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS: f32 = 78.0;
#[cfg(not(target_os = "macos"))]
const TRAFFIC_LIGHTS: f32 = 8.0;

pub struct App {
    doc: Option<DocView>,
    /// A PDF whose worker has not reported its page list yet.
    opening: Option<Worker>,
    /// The PDF this window saves to (None for documents opened from a
    /// legacy `.reflow` file: Save then asks where to write the PDF).
    pdf_path: Option<PathBuf>,
    /// Name shown in the title bar.
    title: String,
    /// Short-lived message in the title bar (text, time shown).
    notice: Option<(String, f64)>,
    saved_revision: u64,
    status: String,
    show_debug: bool,
    toolbar: crate::toolbar::Toolbar,
    capture: Option<Capture>,
    /// Input events to feed egui next frame (menu edit commands, scripts).
    injected: Vec<egui::Event>,
    #[cfg(target_os = "macos")]
    menu: crate::menus::NativeMenu,
    logo: egui::TextureHandle,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>) -> Self {
        let mut app = App {
            doc: None,
            opening: None,
            pdf_path: None,
            title: String::new(),
            notice: None,
            saved_revision: 0,
            status: String::new(),
            show_debug: std::env::var_os("REFLOW_DEBUG").is_some(),
            toolbar: Default::default(),
            capture: Capture::from_env(),
            injected: Vec::new(),
            #[cfg(target_os = "macos")]
            menu: crate::menus::NativeMenu::install(&cc.egui_ctx),
            logo: load_logo(&cc.egui_ctx),
        };
        theme::install(&cc.egui_ctx);
        #[cfg(target_os = "macos")]
        crate::macos::set_context(&cc.egui_ctx);
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
                    self.title = stem(&path);
                    self.pdf_path = None;
                    self.saved_revision = 0;
                }
                Err(e) => self.status = format!("Cannot open {}: {e}", path.display()),
            }
            return;
        }
        let wake = ctx.clone();
        self.opening = Some(Worker::open(path.clone(), move || wake.request_repaint()));
        self.status = format!("Opening {}…", path.display());
        self.title = stem(&path);
        self.pdf_path = Some(path);
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
                Response::Restored(doc) => {
                    // A PDF saved by Reflow: install its exact editable document.
                    let clean = view.editor.revision == self.saved_revision;
                    for (i, section) in doc.sections.into_iter().enumerate() {
                        view.editor.replace_section(i, section);
                    }
                    if clean {
                        self.saved_revision = view.editor.revision;
                    }
                    self.status.clear();
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
        if self.title.is_empty() { "Untitled".into() } else { self.title.clone() }
    }

    /// Save writes the edited document back to its PDF; Save As writes a
    /// PDF wherever the user picks and continues editing that file.
    fn save(&mut self, save_as: bool) {
        let title = self.file_stem();
        let Some(view) = &mut self.doc else { return };
        if view.editor.doc.sections.iter().any(|s| s.pending) {
            // Unfinished pages would be written out blank.
            self.notice = Some(("Still preparing pages — save again in a moment".into(), now()));
            return;
        }
        let path = match (&self.pdf_path, save_as) {
            (Some(p), false) => Some(p.clone()),
            (current, _) => {
                let mut dialog = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(format!("{title}.pdf"));
                if let Some(dir) = current.as_deref().and_then(Path::parent) {
                    dialog = dialog.set_directory(dir);
                }
                dialog.save_file()
            }
        };
        let Some(mut path) = path else { return };
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("pdf")) {
            path.set_extension("pdf");
        }
        view.relayout();
        let result = crate::persist::pdf_bytes(&view.editor.doc, &view.layout).and_then(|bytes| write_atomically(&path, &bytes).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.saved_revision = view.editor.revision;
                self.title = stem(&path);
                self.pdf_path = Some(path);
                self.notice = Some(("Saved".into(), now()));
            }
            Err(e) => self.notice = Some((format!("Couldn’t save: {e}"), now())),
        }
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new().add_filter("Documents", &["pdf", "reflow"]).pick_file() {
            self.open(ctx, path);
        }
    }

    pub fn run(&mut self, ctx: &egui::Context, cmd: Command) {
        use egui::{Event, Key, Modifiers};
        let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        match cmd {
            Command::Open => return self.open_dialog(ctx),
            Command::Save => return self.save(false),
            Command::SaveAs => return self.save(true),
            Command::ToggleInspector => {
                self.show_debug = !self.show_debug;
                return;
            }
            Command::Appearance(pref) => return ctx.set_theme(pref),
            // Edit commands go through egui's input so they reach whatever
            // has focus (the page, or a text field such as the link box).
            Command::Undo => return self.injected.push(key(Key::Z, Modifiers::COMMAND)),
            Command::Redo => return self.injected.push(key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)),
            Command::SelectAll => return self.injected.push(key(Key::A, Modifiers::COMMAND)),
            Command::Copy => return self.injected.push(Event::Copy),
            Command::Cut => return self.injected.push(Event::Cut),
            Command::Paste => {
                if let Some(text) = arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()) {
                    self.injected.push(Event::Paste(text));
                }
                return;
            }
            _ => {}
        }
        let Some(view) = &mut self.doc else { return };
        let e = &mut view.editor;
        match cmd {
            Command::Link => view.link_requested = true,
            Command::Bold => e.toggle_style(|s| s.font.is_bold(), |s, on| s.font.weight = if on { 700 } else { 400 }),
            Command::Italic => e.toggle_style(|s| s.font.italic, |s, on| s.font.italic = on),
            Command::Underline => e.toggle_style(|s| s.underline, |s, on| s.underline = on),
            Command::Strike => e.toggle_style(|s| s.strike, |s, on| s.strike = on),
            Command::Heading(0) => e.set_block_type(editor::BlockType::Normal),
            Command::Heading(l) => e.set_block_type(editor::BlockType::Heading(l)),
            Command::Bullets => e.toggle_list(false),
            Command::Numbers => e.toggle_list(true),
            Command::Indent => e.change_indent(true),
            Command::Outdent => e.change_indent(false),
            Command::Align(a) => e.set_align(a),
            Command::InsertRule => e.insert_rule(),
            Command::ClearFormatting => e.clear_formatting(),
            Command::ZoomIn => view.zoom_by(1.25),
            Command::ZoomOut => view.zoom_by(0.8),
            Command::ActualSize => view.zoom = 1.0,
            Command::FitWidth => view.fit_width(),
            _ => {}
        }
        view.focus_canvas(ctx);
    }

    /// Fallback shortcuts for platforms without a native menu bar (on macOS
    /// the menu bar owns these key equivalents).
    #[cfg(not(target_os = "macos"))]
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let table = [
            (Modifiers::COMMAND, Key::O, Command::Open),
            (Modifiers::COMMAND | Modifiers::SHIFT, Key::S, Command::SaveAs),
            (Modifiers::COMMAND, Key::S, Command::Save),
            (Modifiers::COMMAND, Key::Equals, Command::ZoomIn),
            (Modifiers::COMMAND, Key::Minus, Command::ZoomOut),
            (Modifiers::COMMAND, Key::Num0, Command::ActualSize),
        ];
        for (m, k, cmd) in table {
            if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k))) {
                self.run(ctx, cmd);
            }
        }
    }

    /// Unified title bar: document name and state on the left, primary
    /// actions on the right. Dragging empty space moves the window.
    fn header(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let rect = ui.available_rect_before_wrap().with_max_y(ui.available_rect_before_wrap().top() + 40.0);
        let bg = ui.interact(rect, egui::Id::new("titlebar"), Sense::click_and_drag());
        if bg.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if bg.double_clicked() {
            let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        let mut cmd = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(TRAFFIC_LIGHTS);
                if self.doc.is_some() || self.opening.is_some() {
                    let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    icons::paint(ui.painter(), icon_rect, Icon::Document, p.text_muted, 15.0);
                    ui.label(RichText::new(self.file_stem()).size(13.5).strong().color(p.text));
                    let notice = self.notice.as_ref().filter(|(_, t)| now() - t < 4.0).map(|(n, _)| n.clone());
                    if notice.is_some() {
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                    }
                    let state = if let Some(n) = notice {
                        Some(n)
                    } else if self.opening.is_some() || self.doc.as_ref().is_some_and(|d| d.editor.doc.sections.iter().any(|s| s.pending)) {
                        Some(self.status.clone())
                    } else if self.dirty() {
                        Some("Edited".into())
                    } else {
                        None
                    };
                    if let Some(s) = state {
                        ui.label(RichText::new(format!("—  {s}")).size(12.5).color(p.text_muted));
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(10.0);
                    let has_doc = self.doc.is_some();
                    if has_doc && ui.add(primary_button("Save")).on_hover_text("Save  ⌘S").clicked() {
                        cmd = Some(Command::Save);
                    }
                    let open = egui::Button::new(RichText::new("Open").size(13.0)).min_size(vec2(0.0, 28.0));
                    if ui.add(open).on_hover_text("Open a PDF  ⌘O").clicked() {
                        cmd = Some(Command::Open);
                    }
                    if has_doc && icon_button(ui, Icon::Sidebar, self.show_debug, true, "Reconstruction inspector  ⌥⌘I").clicked() {
                        cmd = Some(Command::ToggleInspector);
                    }
                });
            });
        });
        ui.advance_cursor_after_rect(rect);
        if let Some(c) = cmd {
            self.run(ui.ctx(), c);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let Some(view) = &mut self.doc else { return };
        let mut cmd = None;
        ui.horizontal_centered(|ui| {
            ui.add_space(12.0);
            let pages = view.layout.page_count();
            let words = view.word_count();
            let muted = |t: String| RichText::new(t).size(12.0).color(p.text_muted);
            ui.label(muted(format!("Page {} of {}", (view.current_page + 1).min(pages.max(1)), pages)));
            ui.label(muted("·".into()));
            ui.label(muted(format!("{words} words")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                ui.spacing_mut().item_spacing.x = 0.0;
                if sized_icon_button(ui, 22.0, Icon::Plus, false, true, "Zoom in  ⌘=").clicked() {
                    cmd = Some(Command::ZoomIn);
                }
                let zoom = egui::Button::new(RichText::new(format!("{:.0}%", view.zoom * 100.0)).size(12.0).color(p.text)).min_size(vec2(52.0, 20.0));
                let r = ui.add(zoom).on_hover_text("Zoom");
                egui::Popup::menu(&r).show(|ui| {
                    for z in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
                        if ui.add(egui::Button::selectable((view.zoom - z).abs() < 0.01, format!("{:.0}%", z * 100.0))).clicked() {
                            view.zoom = z;
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Fit width").clicked() {
                        view.fit_width();
                        ui.close();
                    }
                });
                if sized_icon_button(ui, 22.0, Icon::Minus, false, true, "Zoom out  ⌘−").clicked() {
                    cmd = Some(Command::ZoomOut);
                }
            });
        });
        if let Some(c) = cmd {
            self.run(ui.ctx(), c);
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.5 - 170.0).max(24.0));
            ui.add(egui::Image::new(&self.logo).fit_to_exact_size(Vec2::splat(96.0)));
            ui.add_space(18.0);
            if self.opening.is_some() {
                ui.label(RichText::new("Opening…").size(22.0).color(p.text));
                ui.add_space(8.0);
                ui.spinner();
                return;
            }
            ui.label(RichText::new("Edit any PDF like a document").size(24.0).strong().color(p.text));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Reflow rebuilds your PDF into real paragraphs, lists and tables,\nso text wraps and moves as you type.")
                    .size(14.0)
                    .color(p.text_muted),
            );
            ui.add_space(22.0);
            if ui.add(primary_button("Open PDF…").min_size(vec2(150.0, 36.0))).clicked() {
                self.open_dialog(ui.ctx());
            }
            ui.add_space(12.0);
            ui.label(RichText::new("or drop a PDF anywhere in this window").size(12.5).color(p.text_muted));
            if !self.status.is_empty() {
                ui.add_space(18.0);
                ui.label(RichText::new(&self.status).size(12.0).color(Color32::from_rgb(210, 70, 70)));
            }
        });
    }

    /// Accepts files dragged onto the window, with a drop-target overlay.
    fn drag_and_drop(&mut self, ctx: &egui::Context) {
        let p = theme::palette(ctx);
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let rect = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop")));
            painter.rect_filled(rect, 0.0, p.canvas.gamma_multiply(0.85));
            let target = rect.shrink(24.0);
            painter.rect_stroke(target, 16.0, Stroke::new(2.0, p.accent), egui::StrokeKind::Inside);
            painter.text(target.center(), Align2::CENTER_CENTER, "Drop to open", FontId::proportional(22.0), p.accent);
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).find(|p| !p.as_os_str().is_empty()));
        if let Some(path) = dropped {
            self.open(ctx, path);
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let Some(view) = &mut self.doc else { return };
        let section = |ui: &mut egui::Ui, title: &str| {
            ui.add_space(10.0);
            ui.label(RichText::new(title).size(11.0).strong().color(p.text_muted));
            ui.add_space(2.0);
        };
        ui.add_space(6.0);
        ui.label(RichText::new("Reconstruction").size(15.0).strong());
        let o = &mut view.overlays;
        section(ui, "LAYERS");
        ui.checkbox(&mut o.reconstructed, "Reconstructed document");
        ui.checkbox(&mut o.original, "Original PDF");
        if o.original {
            ui.add(egui::Slider::new(&mut o.original_opacity, 0.0..=1.0).text("opacity").show_value(false));
        }
        ui.checkbox(&mut o.diff, "Difference view");
        if o.diff {
            ui.label(RichText::new("Red: original only · Blue: reconstruction only").size(11.5).color(p.text_muted));
        }
        section(ui, "STRUCTURE");
        ui.checkbox(&mut o.glyphs, "Glyphs");
        ui.checkbox(&mut o.words, "Words");
        ui.checkbox(&mut o.lines, "Lines");
        ui.checkbox(&mut o.blocks, "Blocks");
        ui.checkbox(&mut o.zones, "Zones (flows, columns, tables)");
        if !view.diff_scores.is_empty() {
            section(ui, "FIDELITY");
            let mut scores: Vec<_> = view.diff_scores.iter().collect();
            scores.sort_by_key(|(p, _)| **p);
            for (page, s) in scores {
                ui.label(format!("Page {}: {:.2}% of pixels differ", page + 1, s * 100.0));
            }
        }
        section(ui, "FONTS");
        let subs = view.layouter.fonts.substitutions();
        if subs.is_empty() {
            ui.label(RichText::new("All fonts available").color(p.text_muted));
        }
        for (family, r) in subs {
            ui.label(format!("{family} → {}", if r.family.is_empty() { "system" } else { &r.family }));
            ui.label(RichText::new(r.reason).size(11.0).color(p.text_muted));
        }
        let notes: Vec<String> = view
            .analyses
            .iter()
            .enumerate()
            .flat_map(|(i, a)| a.iter().flat_map(|a| &a.notes).map(move |n| format!("Page {}: {n}", i + 1)))
            .collect();
        if !notes.is_empty() {
            section(ui, "NOTES");
            for n in notes {
                ui.label(RichText::new(n).size(12.0));
            }
        }
    }
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn stem(path: &Path) -> String {
    path.file_stem().map_or("Untitled".into(), |s| s.to_string_lossy().into_owned())
}

/// Writes next to the destination, then renames over it, so a failed save
/// never leaves a truncated PDF behind.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.saving"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn load_logo(ctx: &egui::Context) -> egui::TextureHandle {
    let bytes = include_bytes!("../../../scripts/icon.png");
    let img = image::load_from_memory(bytes).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    ctx.load_texture("logo", color, egui::TextureOptions::LINEAR)
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        raw.events.append(&mut self.injected);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(target_os = "macos")]
        {
            if let Some(path) = crate::macos::take_opened().pop() {
                self.open(&ctx, path);
            }
            for cmd in self.menu.take() {
                self.run(&ctx, cmd);
            }
        }
        #[cfg(not(target_os = "macos"))]
        self.shortcuts(&ctx);
        self.poll(&ctx);
        self.drag_and_drop(&ctx);
        let p = theme::palette(&ctx);

        let chrome = Frame::new().fill(p.chrome).inner_margin(Margin { left: 0, right: 0, top: 0, bottom: 8 });
        egui::Panel::top("chrome").frame(chrome).show(ui, |ui| {
            self.header(ui);
            if let Some(view) = &mut self.doc {
                self.toolbar.ui(ui, view);
            }
            let r = ui.max_rect();
            ui.painter().hline(r.x_range(), r.bottom() + 8.0, Stroke::new(1.0, p.hairline));
        });
        if self.doc.is_some() {
            let status = Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(0, 3));
            egui::Panel::bottom("status").frame(status).exact_size(30.0).show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(r.x_range(), r.top() - 3.0, Stroke::new(1.0, p.hairline));
                self.status_bar(ui);
            });
        }
        if self.show_debug && self.doc.is_some() {
            let side = Frame::new().fill(p.chrome).inner_margin(Margin::symmetric(14, 8));
            egui::Panel::right("inspector").frame(side).default_size(270.0).show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().vline(r.left() - 14.0, r.y_range(), Stroke::new(1.0, p.hairline));
                egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
            });
        }
        let canvas = Frame::new().fill(p.canvas);
        egui::CentralPanel::default().frame(canvas).show(ui, |ui| match &mut self.doc {
            Some(view) => view.ui(ui),
            None => self.welcome(ui),
        });

        let ready = match &self.doc {
            Some(d) => d.editor.doc.sections.iter().all(|s| !s.pending),
            None => self.opening.is_none(),
        };
        let step = self.capture.as_mut().filter(|_| ready && self.doc.is_some()).and_then(Capture::next_step);
        if let Some(step) = step {
            if step == "save" {
                self.save(false);
            } else if let Some(events) = crate::capture::pointer_events(&step) {
                self.injected.extend(events);
            } else if let Some(view) = &mut self.doc {
                view.script_step(&ctx, &step);
            }
        }
        if let Some(capture) = &mut self.capture {
            let done = capture.script_done();
            capture.update(&ctx, ready && done);
        }
    }
}
