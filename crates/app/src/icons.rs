//! Vector icons drawn on a 16×16 grid, so they stay crisp at any scale and
//! match the stroke weight of the system font.

use egui::epaint::{CubicBezierShape, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Undo,
    Redo,
    Link,
    AlignLeft,
    AlignCenter,
    AlignRight,
    AlignJustify,
    Bullets,
    Numbers,
    Outdent,
    Indent,
    Table,
    Rule,
    ClearFormat,
    Borders,
    Shade,
    Distribute,
    Trash,
    RowAbove,
    RowBelow,
    ColLeft,
    ColRight,
    DeleteRow,
    DeleteCol,
    Chevron,
    Plus,
    Minus,
    Sidebar,
    Document,
    LineSpacing,
}

struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    scale: f32,
    color: Color32,
    width: f32,
}

impl Pen<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }

    fn stroke(&self) -> Stroke {
        Stroke::new(self.width, self.color)
    }

    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.painter.line_segment([self.p(x0, y0), self.p(x1, y1)], self.stroke());
        // Round caps.
        let r = self.width * 0.5;
        self.painter.circle_filled(self.p(x0, y0), r, self.color);
        self.painter.circle_filled(self.p(x1, y1), r, self.color);
    }

    fn polyline(&self, pts: &[(f32, f32)]) {
        let points: Vec<Pos2> = pts.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(Shape::Path(PathShape::line(points, PathStroke::new(self.width, self.color))));
    }

    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, radius: f32) {
        let r = Rect::from_min_max(self.p(x0, y0), self.p(x1, y1));
        self.painter.rect_stroke(r, radius * self.scale, self.stroke(), egui::StrokeKind::Middle);
    }

    fn fill(&self, x0: f32, y0: f32, x1: f32, y1: f32, color: Color32) {
        self.painter.rect_filled(Rect::from_min_max(self.p(x0, y0), self.p(x1, y1)), 1.0, color);
    }

    fn dot(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_filled(self.p(x, y), r * self.scale, self.color);
    }

    fn curve(&self, a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) {
        self.painter.add(CubicBezierShape::from_points_stroke(
            [self.p(a.0, a.1), self.p(b.0, b.1), self.p(c.0, c.1), self.p(d.0, d.1)],
            false,
            Color32::TRANSPARENT,
            self.stroke(),
        ));
    }

    fn text_lines(&self, x0: f32, lens: &[f32]) {
        let n = lens.len() as f32;
        for (i, len) in lens.iter().enumerate() {
            let y = 3.5 + i as f32 * 9.0 / (n - 1.0);
            self.line(x0, y, x0 + len, y);
        }
    }

    /// 3×3 grid with one row/column highlighted.
    fn grid(&self, hl_row: Option<usize>, hl_col: Option<usize>, accent: Color32) {
        let cell = |i: usize| 2.0 + i as f32 * 4.0;
        for i in 0..3 {
            if hl_row == Some(i) {
                self.fill(2.0, cell(i), 14.0, cell(i) + 4.0, accent);
            }
            if hl_col == Some(i) {
                self.fill(cell(i), 2.0, cell(i) + 4.0, 14.0, accent);
            }
        }
        self.rect(2.0, 2.0, 14.0, 14.0, 1.5);
        for i in 1..3 {
            let v = cell(i);
            self.painter.line_segment([self.p(2.0, v), self.p(14.0, v)], Stroke::new(self.width * 0.7, self.color));
            self.painter.line_segment([self.p(v, 2.0), self.p(v, 14.0)], Stroke::new(self.width * 0.7, self.color));
        }
    }
}

/// Paints `icon` centred in `rect` (drawn at 16 units → `size` points).
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32, size: f32) {
    let scale = size / 16.0;
    let origin = rect.center() - vec2(8.0, 8.0) * scale;
    let pen = Pen { painter, origin, scale, color, width: 1.35 * scale.max(1.0) };
    let soft = color.gamma_multiply(0.28);
    match icon {
        Icon::Undo | Icon::Redo => {
            let flip = |x: f32| if icon == Icon::Undo { x } else { 16.0 - x };
            pen.curve((flip(3.5), 6.5), (flip(6.5), 3.0), (flip(13.5), 3.5), (flip(13.0), 9.5));
            pen.curve((flip(13.0), 9.5), (flip(12.7), 12.0), (flip(11.0), 13.0), (flip(8.5), 13.0));
            pen.polyline(&[(flip(3.0), 2.5), (flip(3.5), 6.8), (flip(7.8), 6.4)]);
        }
        Icon::Link => {
            // Two interlocking rounded links at 45°.
            let s = pen.width;
            let r = |cx: f32, cy: f32| {
                let pts: Vec<Pos2> = (0..=24)
                    .map(|i| {
                        let t = i as f32 / 24.0 * std::f32::consts::TAU;
                        let (x, y) = (3.2 * t.cos(), 1.9 * t.sin());
                        let a = -std::f32::consts::FRAC_PI_4;
                        pen.p(cx + x * a.cos() - y * a.sin(), cy + x * a.sin() + y * a.cos())
                    })
                    .collect();
                painter.add(Shape::closed_line(pts, Stroke::new(s, color)));
            };
            r(5.6, 10.4);
            r(10.4, 5.6);
        }
        Icon::AlignLeft => pen.text_lines(2.0, &[12.0, 8.0, 12.0, 7.0]),
        Icon::AlignJustify => pen.text_lines(2.0, &[12.0, 12.0, 12.0, 7.0]),
        Icon::AlignCenter => {
            for (i, len) in [12.0, 8.0, 12.0, 6.0].iter().enumerate() {
                let y = 3.5 + i as f32 * 3.0;
                pen.line(8.0 - len / 2.0, y, 8.0 + len / 2.0, y);
            }
        }
        Icon::AlignRight => {
            for (i, len) in [12.0, 8.0, 12.0, 7.0].iter().enumerate() {
                let y = 3.5 + i as f32 * 3.0;
                pen.line(14.0 - len, y, 14.0, y);
            }
        }
        Icon::Bullets => {
            for i in 0..3 {
                let y = 3.5 + i as f32 * 4.5;
                pen.dot(3.0, y, 1.3);
                pen.line(6.5, y, 14.0, y);
            }
        }
        Icon::Numbers => {
            for (i, n) in ["1", "2", "3"].iter().enumerate() {
                let y = 3.5 + i as f32 * 4.5;
                painter.text(pen.p(3.0, y), egui::Align2::CENTER_CENTER, *n, egui::FontId::proportional(5.8 * scale), color);
                pen.line(6.5, y, 14.0, y);
            }
        }
        Icon::Outdent | Icon::Indent => {
            pen.line(2.0, 3.0, 14.0, 3.0);
            pen.line(7.5, 6.5, 14.0, 6.5);
            pen.line(7.5, 9.5, 14.0, 9.5);
            pen.line(2.0, 13.0, 14.0, 13.0);
            if icon == Icon::Indent {
                pen.polyline(&[(2.0, 5.8), (4.8, 8.0), (2.0, 10.2)]);
            } else {
                pen.polyline(&[(4.8, 5.8), (2.0, 8.0), (4.8, 10.2)]);
            }
        }
        Icon::Table => {
            pen.rect(2.0, 2.5, 14.0, 13.5, 2.0);
            pen.line(2.5, 6.2, 13.5, 6.2);
            pen.line(2.5, 9.8, 13.5, 9.8);
            pen.line(6.0, 6.2, 6.0, 13.0);
            pen.line(10.0, 6.2, 10.0, 13.0);
        }
        Icon::Rule => {
            pen.line(2.0, 8.0, 14.0, 8.0);
            pen.painter.line_segment([pen.p(4.0, 4.0), pen.p(12.0, 4.0)], Stroke::new(pen.width, soft));
            pen.painter.line_segment([pen.p(4.0, 12.0), pen.p(12.0, 12.0)], Stroke::new(pen.width, soft));
        }
        Icon::ClearFormat => {
            pen.line(3.5, 3.0, 11.5, 3.0);
            pen.line(7.5, 3.0, 6.0, 12.5);
            pen.line(9.5, 9.5, 13.5, 13.5);
            pen.line(13.5, 9.5, 9.5, 13.5);
        }
        Icon::Borders => pen.grid(None, None, soft),
        Icon::Shade => {
            pen.fill(2.0, 2.0, 14.0, 14.0, soft);
            pen.rect(2.0, 2.0, 14.0, 14.0, 2.0);
        }
        Icon::Distribute => {
            pen.line(2.5, 2.5, 2.5, 13.5);
            pen.line(13.5, 2.5, 13.5, 13.5);
            pen.line(5.0, 8.0, 11.0, 8.0);
            pen.polyline(&[(6.8, 6.2), (5.0, 8.0), (6.8, 9.8)]);
            pen.polyline(&[(9.2, 6.2), (11.0, 8.0), (9.2, 9.8)]);
        }
        Icon::Trash => {
            pen.line(2.5, 4.0, 13.5, 4.0);
            pen.polyline(&[(6.0, 4.0), (6.5, 2.0), (9.5, 2.0), (10.0, 4.0)]);
            pen.polyline(&[(3.8, 4.0), (4.6, 14.0), (11.4, 14.0), (12.2, 4.0)]);
            pen.line(6.8, 6.8, 6.8, 11.5);
            pen.line(9.2, 6.8, 9.2, 11.5);
        }
        // Insert: a small table with the new row/column as a filled band.
        Icon::RowAbove | Icon::RowBelow | Icon::ColLeft | Icon::ColRight => {
            let band = Color32::from_rgb(76, 141, 255).gamma_multiply(0.85);
            match icon {
                Icon::RowAbove => pen.grid(Some(0), None, band),
                Icon::RowBelow => pen.grid(Some(2), None, band),
                Icon::ColLeft => pen.grid(None, Some(0), band),
                _ => pen.grid(None, Some(2), band),
            }
        }
        Icon::DeleteRow | Icon::DeleteCol => {
            let red = Color32::from_rgb(232, 76, 76).gamma_multiply(0.85);
            if icon == Icon::DeleteRow { pen.grid(Some(1), None, red) } else { pen.grid(None, Some(1), red) }
        }
        Icon::Chevron => pen.polyline(&[(4.5, 6.5), (8.0, 10.0), (11.5, 6.5)]),
        Icon::Plus => {
            pen.line(3.5, 8.0, 12.5, 8.0);
            pen.line(8.0, 3.5, 8.0, 12.5);
        }
        Icon::Minus => pen.line(3.5, 8.0, 12.5, 8.0),
        Icon::Sidebar => {
            pen.rect(1.5, 2.5, 14.5, 13.5, 2.0);
            pen.line(10.0, 3.0, 10.0, 13.0);
            pen.fill(10.0, 3.0, 14.0, 13.0, soft);
        }
        Icon::Document => {
            pen.polyline(&[(3.5, 1.5), (9.5, 1.5), (12.5, 4.5), (12.5, 14.5), (3.5, 14.5), (3.5, 1.5)]);
            pen.polyline(&[(9.5, 1.5), (9.5, 4.5), (12.5, 4.5)]);
            pen.line(6.0, 8.0, 10.0, 8.0);
            pen.line(6.0, 11.0, 10.0, 11.0);
        }
        Icon::LineSpacing => {
            pen.line(7.0, 3.5, 14.0, 3.5);
            pen.line(7.0, 8.0, 14.0, 8.0);
            pen.line(7.0, 12.5, 14.0, 12.5);
            pen.line(3.0, 3.0, 3.0, 13.0);
            pen.polyline(&[(1.5, 4.5), (3.0, 3.0), (4.5, 4.5)]);
            pen.polyline(&[(1.5, 11.5), (3.0, 13.0), (4.5, 11.5)]);
        }
    }
}
