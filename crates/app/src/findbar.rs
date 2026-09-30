//! Find & replace bar (⌘F / ⌥⌘F), shown under the toolbar.

use editor::Match;
use egui::{Align2, FontId, Key, Margin, RichText, Sense, Ui, vec2};

use crate::icons::Icon;
use crate::theme::{self, BUTTON};
use crate::toolbar::{centered, icon_button, pill, primary_button};
use crate::view::DocView;

#[derive(Default)]
pub struct FindBar {
    pub open: bool,
    show_replace: bool,
    query: String,
    replacement: String,
    case_sensitive: bool,
    matches: Vec<Match>,
    current: usize,
    /// (query, case, document revision) the matches were computed for.
    computed: Option<(String, bool, u64)>,
    focus_query: bool,
}

impl FindBar {
    pub fn show(&mut self, replace: bool) {
        self.open = true;
        self.show_replace |= replace;
        self.focus_query = true;
    }

    fn close(&mut self, view: &mut DocView, ui: &Ui) {
        self.open = false;
        view.find_highlights.clear();
        view.focus_canvas(ui.ctx());
    }

    fn refresh(&mut self, view: &mut DocView) {
        let key = (self.query.clone(), self.case_sensitive, view.editor.revision);
        if self.computed.as_ref() != Some(&key) {
            self.matches = view.editor.find_all(&self.query, self.case_sensitive);
            // Continue from the caret rather than jumping back to the top.
            let caret = view.editor.sel.ordered().0;
            self.current = self.matches.iter().position(|m| (m.para, m.range.start) >= (caret.para, caret.offset)).unwrap_or(0);
            self.computed = Some(key);
        }
        view.find_highlights = self.matches.iter().map(|m| (m.para, m.range.clone())).collect();
        view.find_current = (!self.matches.is_empty()).then_some(self.current);
    }

    /// Moves to (and selects) the next / previous match.
    pub fn step(&mut self, view: &mut DocView, backwards: bool) {
        self.refresh(view);
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len();
        let at_current = view.editor.sel.ordered() == {
            let m = &self.matches[self.current.min(n - 1)];
            (editor::Pos::new(m.para, m.range.start), editor::Pos::new(m.para, m.range.end))
        };
        if at_current {
            self.current = if backwards { (self.current + n - 1) % n } else { (self.current + 1) % n };
        }
        let m = self.matches[self.current.min(n - 1)].clone();
        view.editor.select_match(&m);
        view.reveal_caret();
    }

    pub fn ui(&mut self, ui: &mut Ui, view: &mut DocView) {
        if !self.open {
            return;
        }
        self.refresh(view);
        let p = theme::palette(ui.ctx());
        ui.add_space(4.0);
        centered(ui, "find-bar", |ui| {
            pill(ui, |ui| {
                let (r, _) = ui.allocate_exact_size(egui::Vec2::splat(BUTTON), Sense::hover());
                crate::icons::paint(ui.painter(), r, Icon::Search, p.text_muted, 16.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Find in document")
                        .desired_width(220.0)
                        .margin(Margin::symmetric(8, 5)),
                );
                if std::mem::take(&mut self.focus_query) {
                    field.request_focus();
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if enter {
                    let back = ui.input(|i| i.modifiers.shift);
                    self.step(view, back);
                    field.request_focus();
                }
                let count = match (self.query.is_empty(), self.matches.len()) {
                    (true, _) => String::new(),
                    (false, 0) => "No results".into(),
                    (false, n) => format!("{} of {n}", self.current.min(n - 1) + 1),
                };
                let (r, _) = ui.allocate_exact_size(vec2(74.0, BUTTON), Sense::hover());
                ui.painter().text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, count, FontId::proportional(12.0), p.text_muted);
                let aa = RichText::new("Aa").size(13.0).color(if self.case_sensitive { p.accent } else { p.icon });
                if ui.add(egui::Button::selectable(self.case_sensitive, aa).min_size(egui::Vec2::splat(BUTTON))).on_hover_text("Match case").clicked() {
                    self.case_sensitive = !self.case_sensitive;
                }
                if icon_button(ui, Icon::ChevronUp, false, !self.matches.is_empty(), "Previous  ⇧⌘G").clicked() {
                    self.step(view, true);
                }
                if icon_button(ui, Icon::Chevron, false, !self.matches.is_empty(), "Next  ⌘G").clicked() {
                    self.step(view, false);
                }
                if !self.show_replace && ui.button("Replace…").clicked() {
                    self.show_replace = true;
                }
                if self.show_replace {
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::singleline(&mut self.replacement).hint_text("Replace with").desired_width(180.0).margin(Margin::symmetric(8, 5)));
                    let has = !self.matches.is_empty();
                    if ui.add_enabled(has, egui::Button::new("Replace")).clicked() {
                        let m = self.matches[self.current.min(self.matches.len() - 1)].clone();
                        view.editor.replace_match(&m, &self.replacement);
                        self.computed = None;
                        self.refresh(view);
                        if !self.matches.is_empty() {
                            self.current %= self.matches.len();
                            let next = self.matches[self.current].clone();
                            view.editor.select_match(&next);
                            view.reveal_caret();
                        }
                    }
                    if ui.add_enabled(has, primary_button("Replace All")).clicked() {
                        let n = view.editor.replace_all(&self.query, &self.replacement, self.case_sensitive);
                        view.notice = Some(format!("Replaced {n} occurrence{}", if n == 1 { "" } else { "s" }));
                    }
                }
                if icon_button(ui, Icon::Close, false, true, "Close  Esc").clicked() {
                    self.close(view, ui);
                }
            });
        });
        if self.open && ui.input(|i| i.key_pressed(Key::Escape)) {
            self.close(view, ui);
        }
    }
}
