//! Formatting toolbar: a floating pill of grouped controls, plus a
//! contextual table bar when the caret is in a table.

use std::sync::Arc;

use document::{Align, LineSpacing, ListKind, Rgba};
use editor::BlockType;
use egui::{Align2, Color32, CornerRadius, FontId, Frame, Margin, Popup, PopupCloseBehavior, Rect, Response, RichText, Sense, Stroke, Ui, Vec2, vec2};

use crate::icons::{self, Icon};
use crate::theme::{self, BUTTON, Palette, RADIUS};
use crate::view::DocView;

const FONTS: &[&str] = &[
    "Arial", "Avenir Next", "Baskerville", "Calibri", "Cambria", "Courier New", "Futura", "Garamond", "Georgia",
    "Gill Sans", "Helvetica", "Helvetica Neue", "Menlo", "Palatino", "Times New Roman", "Trebuchet MS", "Verdana",
];
const SIZES: &[f32] = &[8.0, 9.0, 10.0, 10.5, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 24.0, 28.0, 32.0, 36.0, 48.0, 72.0];
const PALETTE: &[[u8; 3]] = &[
    [0, 0, 0], [67, 67, 67], [102, 102, 102], [153, 153, 153], [204, 204, 204], [255, 255, 255],
    [152, 0, 0], [230, 45, 45], [255, 153, 0], [241, 194, 50], [52, 168, 83], [0, 172, 193],
    [38, 110, 235], [26, 58, 140], [103, 58, 183], [194, 24, 91], [45, 74, 107], [120, 80, 50],
    [244, 204, 204], [252, 229, 205], [255, 242, 204], [217, 234, 211], [207, 226, 243], [217, 210, 233],
];

#[derive(Default)]
pub struct Toolbar {
    link_open: bool,
    link_text: String,
    grid: (usize, usize),
    size_text: Option<String>,
}

fn color32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

// ---------------------------------------------------------------------------
// Widgets

fn button_frame(ui: &Ui, rect: Rect, resp: &Response, active: bool, p: &Palette) {
    let fill = if active {
        p.accent_soft
    } else if resp.is_pointer_button_down_on() {
        p.pressed
    } else if resp.hovered() {
        p.hover
    } else {
        return;
    };
    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), fill);
}

pub fn icon_button(ui: &mut Ui, icon: Icon, active: bool, enabled: bool, tip: &str) -> Response {
    sized_icon_button(ui, BUTTON, icon, active, enabled, tip)
}

pub fn sized_icon_button(ui: &mut Ui, size: f32, icon: Icon, active: bool, enabled: bool, tip: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), sense);
    button_frame(ui, rect, &resp, active, &p);
    let color = if !enabled {
        p.icon.gamma_multiply(0.35)
    } else if active {
        p.accent
    } else {
        p.icon
    };
    icons::paint(ui.painter(), rect, icon, color, (size * 0.57).round());
    resp.on_hover_text(tip)
}

fn glyph_button(ui: &mut Ui, text: RichText, active: bool, tip: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    button_frame(ui, rect, &resp, active, &p);
    let color = if active { p.accent } else { p.icon };
    let galley = egui::WidgetText::from(text.size(15.0).color(color)).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    resp.on_hover_text(tip)
}

/// A dropdown "chip": label plus chevron; returns the response to attach a
/// popup to.
fn chip(ui: &mut Ui, label: &str, width: f32, tip: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, BUTTON), Sense::click());
    let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&resp));
    button_frame(ui, rect, &resp, open, &p);
    let text_rect = rect.shrink2(vec2(9.0, 0.0));
    let clipped = ui.painter().with_clip_rect(text_rect.with_max_x(rect.right() - 20.0));
    clipped.text(text_rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(13.0), p.text);
    let chevron = Rect::from_center_size(egui::pos2(rect.right() - 11.0, rect.center().y), Vec2::splat(12.0));
    icons::paint(ui.painter(), chevron, Icon::Chevron, p.text_muted, 10.0);
    resp.on_hover_text(tip)
}

fn divider(ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, BUTTON), Sense::hover());
    let x = rect.center().x;
    ui.painter().line_segment([egui::pos2(x, rect.top() + 6.0), egui::pos2(x, rect.bottom() - 6.0)], Stroke::new(1.0, p.hairline));
}

fn menu_popup(resp: &Response, add: impl FnOnce(&mut Ui)) {
    Popup::menu(resp).close_behavior(PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.set_min_width(resp.rect.width().max(140.0));
        add(ui);
    });
}

fn menu_item(ui: &mut Ui, selected: bool, text: impl Into<RichText>) -> bool {
    let clicked = ui.add(egui::Button::selectable(selected, text.into()).min_size(vec2(0.0, 26.0))).clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// Swatch grid; `Some(None)` means "no colour".
fn swatches(ui: &mut Ui, none_label: Option<&str>) -> Option<Option<Rgba>> {
    let p = theme::palette(ui.ctx());
    let mut picked = None;
    if let Some(label) = none_label
        && menu_item(ui, false, label)
    {
        picked = Some(None);
    }
    egui::Grid::new(ui.next_auto_id()).spacing(Vec2::splat(5.0)).show(ui, |ui| {
        for (i, [r, g, b]) in PALETTE.iter().enumerate() {
            let c = Color32::from_rgb(*r, *g, *b);
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
            let rect = if resp.hovered() { rect.expand(1.5) } else { rect };
            ui.painter().rect_filled(rect, 5.0, c);
            ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0, p.surface_stroke), egui::StrokeKind::Inside);
            if resp.clicked() {
                picked = Some(Some(Rgba::rgb(*r, *g, *b)));
                ui.close();
            }
            if i % 6 == 5 {
                ui.end_row();
            }
        }
    });
    picked
}

/// Floating rounded container for a group of toolbar controls.
pub fn pill<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = theme::palette(ui.ctx());
    let shadow_alpha = if ui.visuals().dark_mode { 60 } else { 14 };
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.surface_stroke))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(6, 4))
        .shadow(egui::Shadow { offset: [0, 1], blur: 6, spread: 0, color: Color32::from_black_alpha(shadow_alpha) })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(add).inner
        })
        .inner
}

/// Lays out `add` horizontally centred in the available width (using the
/// width measured on the previous frame).
pub fn centered(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    let id = ui.id().with(id);
    let width: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
        let r = ui.scope(add).response;
        ui.data_mut(|d| d.insert_temp(id, r.rect.width()));
    });
}

/// Filled accent button (the one primary action in a context).
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(Color32::WHITE).strong())
        .fill(theme::LIGHT.accent)
        .corner_radius(CornerRadius::same(RADIUS + 1))
        .min_size(vec2(0.0, 28.0))
}

// ---------------------------------------------------------------------------

impl Toolbar {
    pub fn open_link_editor(&mut self, current: Option<&str>) {
        self.link_open = true;
        self.link_text = current.unwrap_or("https://").to_string();
    }

    pub fn ui(&mut self, ui: &mut Ui, view: &mut DocView) {
        if std::mem::take(&mut view.link_requested) {
            let current = view.editor.current_link();
            self.open_link_editor(current.as_deref());
        }
        let mut acted = false;
        centered(ui, "main-toolbar", |ui| acted |= self.main_bar(ui, view));
        if view.editor.table_at_caret().is_some() {
            ui.add_space(4.0);
            centered(ui, "table-toolbar", |ui| acted |= self.table_bar(ui, view));
        }
        acted |= self.link_window(ui, view);
        if acted {
            view.focus_canvas(ui.ctx());
        }
    }

    fn main_bar(&mut self, ui: &mut Ui, view: &mut DocView) -> bool {
        let style = view.editor.current_style();
        let pstyle = view.editor.current_paragraph_style();
        let p = theme::palette(ui.ctx());
        let mut acted = false;
        pill(ui, |ui| {
            let e = &mut view.editor;
            if icon_button(ui, Icon::Undo, false, e.can_undo(), "Undo  ⌘Z").clicked() {
                e.undo();
                acted = true;
            }
            if icon_button(ui, Icon::Redo, false, e.can_redo(), "Redo  ⇧⌘Z").clicked() {
                e.redo();
                acted = true;
            }
            divider(ui);

            let block = e.block_type();
            let name = |b: BlockType| match b {
                BlockType::Normal => "Normal text".to_string(),
                BlockType::Heading(l) => format!("Heading {l}"),
            };
            let r = chip(ui, &name(block), 112.0, "Paragraph style");
            menu_popup(&r, |ui| {
                for (b, size) in [(BlockType::Normal, 13.0), (BlockType::Heading(1), 20.0), (BlockType::Heading(2), 17.0), (BlockType::Heading(3), 15.0)] {
                    let text = RichText::new(name(b)).size(size);
                    let text = if b == BlockType::Normal { text } else { text.strong() };
                    if menu_item(ui, b == block, text) {
                        e.set_block_type(b);
                        acted = true;
                    }
                }
            });

            let family = style.font.family.to_string();
            let r = chip(ui, &family, 132.0, "Font");
            let fonts = &mut view.layouter.fonts;
            menu_popup(&r, |ui| {
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for f in FONTS.iter().copied().filter(|f| fonts.is_installed(f)) {
                        if menu_item(ui, f == family, f) {
                            let name: Arc<str> = Arc::from(f);
                            e.set_text_style(|s| {
                                s.font.family = name.clone();
                                s.font.source_name = None;
                            });
                            acted = true;
                        }
                    }
                });
            });

            // Font size stepper with an editable value.
            if icon_button(ui, Icon::Minus, false, true, "Decrease font size").clicked() {
                let next = SIZES.iter().rev().copied().find(|&s| s < style.size - 0.01).unwrap_or(style.size - 1.0).max(1.0);
                e.set_text_style(|s| s.size = next);
                acted = true;
            }
            let shown = format!("{}", (style.size * 10.0).round() / 10.0);
            let text = self.size_text.get_or_insert_with(|| shown.clone());
            let field = ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(34.0)
                    .horizontal_align(egui::Align::Center)
                    .margin(Margin::symmetric(2, 5))
                    .font(FontId::proportional(13.0)),
            );
            if field.lost_focus() {
                if let Ok(v) = text.trim().parse::<f32>()
                    && (1.0..=400.0).contains(&v)
                {
                    e.set_text_style(|s| s.size = v);
                    acted = true;
                }
                self.size_text = None;
            } else if !field.has_focus() {
                self.size_text = None;
            }
            if icon_button(ui, Icon::Plus, false, true, "Increase font size").clicked() {
                let next = SIZES.iter().copied().find(|&s| s > style.size + 0.01).unwrap_or(style.size + 1.0);
                e.set_text_style(|s| s.size = next);
                acted = true;
            }
            divider(ui);

            if glyph_button(ui, RichText::new("B").strong(), style.font.is_bold(), "Bold  ⌘B").clicked() {
                e.toggle_style(|s| s.font.is_bold(), |s, on| s.font.weight = if on { 700 } else { 400 });
                acted = true;
            }
            if glyph_button(ui, RichText::new("I").italics(), style.font.italic, "Italic  ⌘I").clicked() {
                e.toggle_style(|s| s.font.italic, |s, on| s.font.italic = on);
                acted = true;
            }
            if glyph_button(ui, RichText::new("U").underline(), style.underline, "Underline  ⌘U").clicked() {
                e.toggle_style(|s| s.underline, |s, on| s.underline = on);
                acted = true;
            }
            if glyph_button(ui, RichText::new("S").strikethrough(), style.strike, "Strikethrough  ⇧⌘X").clicked() {
                e.toggle_style(|s| s.strike, |s, on| s.strike = on);
                acted = true;
            }
            // Text colour: an "A" over a bar of the current colour (as in Docs).
            let (rect, r) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
            let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&r));
            button_frame(ui, rect, &r, open, &p);
            ui.painter().text(rect.center() - vec2(0.0, 2.5), Align2::CENTER_CENTER, "A", FontId::proportional(14.0), p.icon);
            let bar = Rect::from_center_size(rect.center() + vec2(0.0, 7.0), vec2(14.0, 3.0));
            ui.painter().rect_filled(bar, 1.0, color32(style.color));
            let r = r.on_hover_text("Text colour");
            menu_popup(&r, |ui| {
                if let Some(Some(c)) = swatches(ui, None) {
                    e.set_text_style(|s| s.color = c);
                    acted = true;
                }
            });
            if icon_button(ui, Icon::Link, style.link.is_some(), true, "Link  ⌘K").clicked() {
                self.link_open = true;
                self.link_text = style.link.as_deref().unwrap_or("https://").to_string();
            }
            divider(ui);

            for (align, icon, tip) in [
                (Align::Left, Icon::AlignLeft, "Align left  ⇧⌘L"),
                (Align::Center, Icon::AlignCenter, "Centre  ⇧⌘E"),
                (Align::Right, Icon::AlignRight, "Align right  ⇧⌘R"),
                (Align::Justify, Icon::AlignJustify, "Justify  ⇧⌘J"),
            ] {
                if icon_button(ui, icon, pstyle.align == align, true, tip).clicked() {
                    e.set_align(align);
                    acted = true;
                }
            }
            let spacing = match pstyle.line_spacing {
                LineSpacing::Multiple(m) => m / 1.2,
                LineSpacing::Exact(v) => v / (style.size * 1.2).max(1.0),
            };
            let r = icon_button(ui, Icon::LineSpacing, false, true, "Line spacing");
            menu_popup(&r, |ui| {
                for (m, label) in [(1.0, "Single"), (1.15, "1.15"), (1.5, "1.5"), (2.0, "Double")] {
                    if menu_item(ui, (spacing - m).abs() < 0.03, label) {
                        e.set_line_spacing(m);
                        acted = true;
                    }
                }
            });
            divider(ui);

            let (bullets, numbers) = match pstyle.list.as_ref().map(|l| &l.kind) {
                Some(ListKind::Bullet(_)) => (true, false),
                Some(ListKind::Ordered { .. }) => (false, true),
                None => (false, false),
            };
            if icon_button(ui, Icon::Bullets, bullets, true, "Bulleted list  ⇧⌘8").clicked() {
                e.toggle_list(false);
                acted = true;
            }
            if icon_button(ui, Icon::Numbers, numbers, true, "Numbered list  ⇧⌘7").clicked() {
                e.toggle_list(true);
                acted = true;
            }
            if icon_button(ui, Icon::Outdent, false, true, "Decrease indent  ⌘[").clicked() {
                e.change_indent(false);
                acted = true;
            }
            if icon_button(ui, Icon::Indent, false, true, "Increase indent  ⌘]").clicked() {
                e.change_indent(true);
                acted = true;
            }
            divider(ui);

            let r = icon_button(ui, Icon::Table, false, true, "Insert table");
            let grid = &mut self.grid;
            let layout = &view.layout;
            menu_popup(&r, |ui| {
                ui.label(RichText::new(format!("{} × {} table", grid.1 + 1, grid.0 + 1)).color(p.text_muted));
                ui.add_space(4.0);
                let (cell, gap) = (18.0, 3.0);
                let (area, _) = ui.allocate_exact_size(Vec2::splat(8.0 * cell + 7.0 * gap), Sense::hover());
                for r in 0..8 {
                    for c in 0..8 {
                        let rect = Rect::from_min_size(area.min + vec2(c as f32, r as f32) * (cell + gap), Vec2::splat(cell));
                        let resp = ui.interact(rect, ui.id().with(("cell", r, c)), Sense::click());
                        if resp.hovered() {
                            *grid = (r, c);
                        }
                        let on = r <= grid.0 && c <= grid.1;
                        ui.painter().rect_filled(rect, 3.0, if on { p.accent_soft } else { p.hover });
                        let border = if on { p.accent } else { p.surface_stroke };
                        ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
                        if resp.clicked() {
                            let para = e.sel.focus.para.min(layout.para_count().saturating_sub(1));
                            e.insert_table(r + 1, c + 1, layout.para(para).width);
                            acted = true;
                            ui.close();
                        }
                    }
                }
            });
            if icon_button(ui, Icon::Image, false, true, "Insert image").clicked() {
                view.insert_image_requested = true;
            }
            if icon_button(ui, Icon::Rule, false, true, "Insert horizontal line").clicked() {
                e.insert_rule();
                acted = true;
            }
            if icon_button(ui, Icon::ClearFormat, false, true, "Clear formatting  ⌘\\").clicked() {
                e.clear_formatting();
                acted = true;
            }
        });
        acted
    }

    fn table_bar(&mut self, ui: &mut Ui, view: &mut DocView) -> bool {
        let Some(cursor) = view.editor.table_at_caret() else { return false };
        let mut acted = false;
        let p = theme::palette(ui.ctx());
        pill(ui, |ui| {
            let e = &mut view.editor;
            let label = format!("TABLE  ·  row {} of {}  ·  column {} of {}", cursor.row + 1, cursor.rows, cursor.col + 1, cursor.cols);
            let (rect, _) = ui.allocate_exact_size(vec2(214.0, BUTTON), Sense::hover());
            ui.painter().text(rect.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(11.5), p.text_muted);
            divider(ui);
            type Action = fn(&mut editor::Editor);
            let buttons: [(Icon, &str, Action); 6] = [
                (Icon::RowAbove, "Insert row above", |e| e.table_insert_row(false)),
                (Icon::RowBelow, "Insert row below", |e| e.table_insert_row(true)),
                (Icon::ColLeft, "Insert column left", |e| e.table_insert_col(false)),
                (Icon::ColRight, "Insert column right", |e| e.table_insert_col(true)),
                (Icon::DeleteRow, "Delete row", |e| e.table_delete_row()),
                (Icon::DeleteCol, "Delete column", |e| e.table_delete_col()),
            ];
            for (i, (icon, tip, f)) in buttons.into_iter().enumerate() {
                if i == 4 {
                    divider(ui);
                }
                if icon_button(ui, icon, false, true, tip).clicked() {
                    f(e);
                    acted = true;
                }
            }
            divider(ui);
            if icon_button(ui, Icon::Merge, false, e.can_merge_cells(), "Merge cells (select across cells first)").clicked() {
                e.table_merge_cells();
                acted = true;
            }
            if icon_button(ui, Icon::Split, false, e.can_split_cell(), "Unmerge cells").clicked() {
                e.table_split_cell();
                acted = true;
            }
            divider(ui);
            let borders = e.table_borders();
            if icon_button(ui, Icon::Borders, borders.is_some(), true, "Borders").clicked() {
                e.table_toggle_borders();
                acted = true;
            }
            if let Some(b) = borders {
                let (rect, r) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
                let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&r));
                button_frame(ui, rect, &r, open, &p);
                let sq = Rect::from_center_size(rect.center(), Vec2::splat(12.0));
                ui.painter().rect_stroke(sq, 2.0, Stroke::new(2.0, color32(b.color)), egui::StrokeKind::Inside);
                let r = r.on_hover_text("Border colour");
                menu_popup(&r, |ui| {
                    if let Some(Some(c)) = swatches(ui, None) {
                        e.table_set_border_color(c);
                        acted = true;
                    }
                });
            }
            let r = icon_button(ui, Icon::Shade, false, true, "Cell shading");
            menu_popup(&r, |ui| {
                let id = egui::Id::new("shade-row");
                let mut row = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
                ui.checkbox(&mut row, "Apply to whole row");
                ui.data_mut(|d| d.insert_temp(id, row));
                ui.add_space(2.0);
                if let Some(c) = swatches(ui, Some("No fill")) {
                    e.table_set_shading(c, row);
                    acted = true;
                }
            });
            if icon_button(ui, Icon::Distribute, false, true, "Distribute columns evenly").clicked() {
                e.table_distribute_columns();
                acted = true;
            }
            divider(ui);
            if icon_button(ui, Icon::Trash, false, true, "Delete table").clicked() {
                e.table_delete();
                acted = true;
            }
        });
        acted
    }

    fn link_window(&mut self, ui: &mut Ui, view: &mut DocView) -> bool {
        if !self.link_open {
            return false;
        }
        let mut acted = false;
        let mut open = true;
        egui::Window::new("Link")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, vec2(0.0, 110.0))
            .show(ui.ctx(), |ui| {
                ui.label(RichText::new("Address").color(theme::palette(ui.ctx()).text_muted));
                let resp = ui.add(egui::TextEdit::singleline(&mut self.link_text).desired_width(320.0).margin(Margin::symmetric(8, 6)));
                resp.request_focus();
                if view.editor.sel.is_collapsed() {
                    ui.weak("Select the text to link first.");
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.add(primary_button("Apply")).clicked() || enter {
                        view.editor.set_link(Some(self.link_text.trim()));
                        self.link_open = false;
                        acted = true;
                    }
                    if view.editor.current_link().is_some() && ui.button("Remove link").clicked() {
                        view.editor.set_link(None);
                        self.link_open = false;
                        acted = true;
                    }
                });
            });
        self.link_open &= open;
        acted
    }
}
