//! Page Setup dialog: paper size, orientation and margins.

use document::{Margins, Size};
use egui::RichText;

use crate::theme;
use crate::toolbar::primary_button;
use crate::view::DocView;

const PAPERS: &[(&str, f32, f32)] = &[
    ("US Letter", 612.0, 792.0),
    ("US Legal", 612.0, 1008.0),
    ("Tabloid", 792.0, 1224.0),
    ("A4", 595.28, 841.89),
    ("A5", 419.53, 595.28),
];

#[derive(Default)]
pub struct PageSetup {
    open: bool,
    size: Size,
    /// Margins in inches while editing.
    margins: [f32; 4],
    whole_document: bool,
}

impl PageSetup {
    pub fn show(&mut self, view: &DocView) {
        let (size, m) = view.editor.page_setup();
        self.size = size;
        self.margins = [m.top, m.bottom, m.left, m.right].map(|v| (v / 72.0 * 100.0).round() / 100.0);
        self.whole_document = true;
        self.open = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context, view: &mut DocView) {
        if !self.open {
            return;
        }
        let p = theme::palette(ctx);
        let (w, h) = (self.size.w.min(self.size.h), self.size.w.max(self.size.h));
        let landscape = self.size.w > self.size.h;
        let paper = PAPERS.iter().find(|(_, pw, ph)| (pw - w).abs() < 1.0 && (ph - h).abs() < 1.0).map(|p| p.0);
        let mut apply = false;
        let modal = egui::Modal::new(egui::Id::new("page-setup")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new("Page Setup").size(16.0).strong());
            ui.add_space(12.0);
            egui::Grid::new("page-setup-grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                ui.label("Paper size");
                let label = paper.map_or_else(|| format!("Custom ({:.1} × {:.1} in)", w / 72.0, h / 72.0), str::to_string);
                egui::ComboBox::from_id_salt("paper").width(200.0).selected_text(label).show_ui(ui, |ui| {
                    for &(name, pw, ph) in PAPERS {
                        if ui.selectable_label(paper == Some(name), format!("{name}  ({:.1} × {:.1} in)", pw / 72.0, ph / 72.0)).clicked() {
                            self.size = if landscape { Size::new(ph, pw) } else { Size::new(pw, ph) };
                        }
                    }
                });
                ui.end_row();
                ui.label("Orientation");
                ui.horizontal(|ui| {
                    if ui.selectable_label(!landscape, "Portrait").clicked() {
                        self.size = Size::new(w, h);
                    }
                    if ui.selectable_label(landscape, "Landscape").clicked() {
                        self.size = Size::new(h, w);
                    }
                });
                ui.end_row();
                for (i, name) in ["Top margin", "Bottom margin", "Left margin", "Right margin"].iter().enumerate() {
                    ui.label(*name);
                    ui.add(egui::DragValue::new(&mut self.margins[i]).range(0.0..=4.0).speed(0.01).fixed_decimals(2).suffix(" in"));
                    ui.end_row();
                }
                ui.label("Apply to");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.whole_document, true, "Whole document");
                    ui.radio_value(&mut self.whole_document, false, "This page");
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Text reflows to fit the new page area.").size(12.0).color(p.text_muted));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary_button("Apply")).clicked() {
                        apply = true;
                    }
                    if ui.button("Cancel").clicked() {
                        self.open = false;
                    }
                });
            });
        });
        if apply {
            let [top, bottom, left, right] = self.margins.map(|v| v * 72.0);
            view.editor.set_page_setup(self.size, Margins { top, right, bottom, left }, self.whole_document);
            self.open = false;
            view.focus_canvas(ctx);
        } else if modal.should_close() {
            self.open = false;
        }
    }
}
