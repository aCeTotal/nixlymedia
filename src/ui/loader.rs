use std::f32::consts::TAU;

use egui::{pos2, vec2, Color32, FontId, Rect, Sense, Stroke, Ui};

use crate::ui::theme;

pub fn draw(ui: &mut Ui, label: &str) {
    let avail = ui.available_size_before_wrap();
    let (rect, _) = ui.allocate_exact_size(avail, Sense::hover());
    let painter = ui.painter_at(rect);
    let ctx = ui.ctx();
    let t = ctx.input(|i| i.time) as f32;
    ctx.request_repaint();

    let center = rect.center();
    let radius = 46.0;

    for i in 0..3 {
        let phase = (t * 0.9 + i as f32 * 0.33) % 1.0;
        let r = radius + phase * 70.0;
        let a = ((1.0 - phase) * 90.0) as u8;
        painter.circle_stroke(
            center,
            r,
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(
                theme::ACCENT.r(),
                theme::ACCENT.g(),
                theme::ACCENT.b(),
                a,
            )),
        );
    }

    let segs = 56;
    let head = t * 1.6;
    let tail = 0.78;
    for i in 0..segs {
        let f = i as f32 / segs as f32;
        let theta = head + f * TAU;
        let alpha_t = 1.0 - f / tail;
        if alpha_t <= 0.0 {
            continue;
        }
        let a = (alpha_t.powf(1.4) * 230.0) as u8;
        let inner = pos2(
            center.x + theta.cos() * (radius - 5.0),
            center.y + theta.sin() * (radius - 5.0),
        );
        let outer = pos2(
            center.x + theta.cos() * (radius + 5.0),
            center.y + theta.sin() * (radius + 5.0),
        );
        let c = lerp_color(theme::ACCENT, Color32::from_rgb(180, 90, 255), f);
        painter.line_segment(
            [inner, outer],
            Stroke::new(3.0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)),
        );
    }

    let bob = (t * 1.8).sin() * 3.0;
    let dots = 3;
    let dot_w = 9.0;
    let dot_gap = 12.0;
    let total_w = dots as f32 * dot_w + (dots - 1) as f32 * dot_gap;
    let dots_y = center.y + radius + 38.0 + bob;
    let dots_x0 = center.x - total_w / 2.0 + dot_w / 2.0;
    for i in 0..dots {
        let phase = (t * 2.2 - i as f32 * 0.35).sin() * 0.5 + 0.5;
        let r = 3.5 + phase * 3.0;
        let a = (160.0 + phase * 95.0) as u8;
        painter.circle_filled(
            pos2(dots_x0 + i as f32 * (dot_w + dot_gap), dots_y),
            r,
            Color32::from_rgba_unmultiplied(
                theme::ACCENT.r(),
                theme::ACCENT.g(),
                theme::ACCENT.b(),
                a,
            ),
        );
    }

    painter.text(
        pos2(center.x, dots_y + 32.0),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(16.0),
        theme::TEXT_DIM,
    );

    let _ = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
