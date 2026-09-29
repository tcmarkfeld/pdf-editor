//! Visual design system: palettes for light/dark appearance, the system UI
//! font, and egui style tuned to feel like a native macOS document app.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme};

/// Semantic colours. Everything in the chrome is drawn from these.
#[derive(Clone, Copy)]
pub struct Palette {
    /// Window chrome (title bar, toolbar strip, status bar).
    pub chrome: Color32,
    /// Hairline separating chrome from the canvas.
    pub hairline: Color32,
    /// Area behind the pages.
    pub canvas: Color32,
    /// Floating surfaces (toolbar pill, popovers, cards).
    pub surface: Color32,
    pub surface_stroke: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub icon: Color32,
    pub hover: Color32,
    pub pressed: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
}

pub const LIGHT: Palette = Palette {
    chrome: Color32::from_rgb(246, 246, 248),
    hairline: Color32::from_rgb(222, 222, 227),
    canvas: Color32::from_rgb(234, 235, 239),
    surface: Color32::from_rgb(255, 255, 255),
    surface_stroke: Color32::from_rgb(225, 226, 231),
    text: Color32::from_rgb(29, 29, 31),
    text_muted: Color32::from_rgb(110, 110, 118),
    icon: Color32::from_rgb(60, 60, 67),
    hover: Color32::from_rgba_premultiplied(0, 0, 0, 12),
    pressed: Color32::from_rgba_premultiplied(0, 0, 0, 22),
    accent: Color32::from_rgb(38, 110, 235),
    accent_soft: Color32::from_rgba_premultiplied(5, 16, 33, 36),
    on_accent: Color32::WHITE,
};

pub const DARK: Palette = Palette {
    chrome: Color32::from_rgb(38, 38, 42),
    hairline: Color32::from_rgb(20, 20, 22),
    canvas: Color32::from_rgb(28, 28, 31),
    surface: Color32::from_rgb(50, 50, 55),
    surface_stroke: Color32::from_rgb(64, 64, 70),
    text: Color32::from_rgb(240, 240, 244),
    text_muted: Color32::from_rgb(152, 152, 160),
    icon: Color32::from_rgb(222, 222, 228),
    hover: Color32::from_rgba_premultiplied(18, 18, 18, 18),
    pressed: Color32::from_rgba_premultiplied(30, 30, 30, 30),
    accent: Color32::from_rgb(76, 141, 255),
    accent_soft: Color32::from_rgba_premultiplied(18, 33, 60, 60),
    on_accent: Color32::WHITE,
};

pub fn palette(ctx: &egui::Context) -> Palette {
    if ctx.theme() == Theme::Dark { DARK } else { LIGHT }
}

/// Toolbar icon button size and corner radius.
pub const BUTTON: f32 = 28.0;
pub const RADIUS: u8 = 6;

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::System);
    for (theme, p) in [(Theme::Light, LIGHT), (Theme::Dark, DARK)] {
        ctx.style_mut_of(theme, |style| apply(style, &p, theme == Theme::Dark));
    }
}

/// Uses the macOS system font (San Francisco) for the interface when
/// available, keeping egui's bundled fonts as fallback for symbols.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, path) in [("sf", "/System/Library/Fonts/SFNS.ttf"), ("sf-mono", "/System/Library/Fonts/SFNSMono.ttf")] {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert(name.into(), std::sync::Arc::new(FontData::from_owned(bytes)));
            let family = if name == "sf" { FontFamily::Proportional } else { FontFamily::Monospace };
            fonts.families.entry(family).or_default().insert(0, name.into());
        }
    }
    ctx.set_fonts(fonts);
}

fn apply(style: &mut egui::Style, p: &Palette, dark: bool) {
    style.text_styles = [
        (TextStyle::Small, FontId::proportional(11.0)),
        (TextStyle::Body, FontId::proportional(13.0)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Heading, FontId::proportional(17.0)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(6.0, 6.0);
    s.button_padding = egui::vec2(10.0, 4.0);
    s.interact_size = egui::vec2(28.0, 26.0);
    s.menu_margin = egui::Margin::same(6);
    s.window_margin = egui::Margin::same(14);
    s.scroll = egui::style::ScrollStyle::floating();

    let v = &mut style.visuals;
    v.dark_mode = dark;
    v.override_text_color = Some(p.text);
    v.weak_text_color = Some(p.text_muted);
    v.panel_fill = p.chrome;
    v.window_fill = p.surface;
    v.extreme_bg_color = p.surface;
    v.faint_bg_color = p.chrome;
    v.window_stroke = Stroke::new(1.0, p.surface_stroke);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    let shadow = Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 38 }) };
    v.window_shadow = shadow;
    v.popup_shadow = shadow;
    v.selection.bg_fill = p.accent;
    v.selection.stroke = Stroke::new(1.0, p.on_accent);
    v.hyperlink_color = p.accent;
    v.button_frame = true;
    v.handle_shape = egui::style::HandleShape::Circle;

    let radius = CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.hairline);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = radius;
    for (state, fill, stroke) in [
        (&mut w.inactive, Color32::TRANSPARENT, Stroke::NONE),
        (&mut w.hovered, p.hover, Stroke::NONE),
        (&mut w.active, p.pressed, Stroke::NONE),
        (&mut w.open, p.hover, Stroke::NONE),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = stroke;
        state.fg_stroke = Stroke::new(1.5, p.icon);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    // Text fields and sliders need a visible well.
    w.inactive.bg_fill = if dark { Color32::from_rgb(62, 62, 68) } else { Color32::from_rgb(236, 236, 240) };
}
