use egui::{
    pos2, vec2, Color32, FontId, Rect, Sense, Stroke, TextureHandle, Ui,
};

use crate::ui::theme;

pub struct CardData {
    pub title: String,
    pub rating: f32,
    pub badge: Option<String>,
    pub collection: bool,
    pub count: usize,
}

const INSERT_DUR: f32 = 0.5;

pub fn draw(
    ui: &mut Ui,
    rect: Rect,
    data: CardData,
    texture: Option<TextureHandle>,
    focused: bool,
    insert_age: Option<f32>,
) -> egui::Response {
    let resp = ui.allocate_rect(rect, Sense::hover());
    let t_focus = ui
        .ctx()
        .animate_bool_with_time(resp.id, focused, 0.18);
    let painter = ui.painter();

    let scale = match insert_age {
        Some(age) if age < INSERT_DUR => {
            let t = (age / INSERT_DUR).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            0.78 + 0.22 * eased
        }
        _ => 1.0,
    };
    let rr = Rect::from_center_size(rect.center(), rect.size() * scale);
    let label_h = (rr.height() * 0.10).clamp(36.0, 60.0);
    let poster_h = rr.height() - label_h;
    let poster_rect = Rect::from_min_size(rr.left_top(), vec2(rr.width(), poster_h));
    let label_rect = Rect::from_min_max(
        pos2(rr.left(), poster_rect.bottom()),
        rr.right_bottom(),
    );

    if data.collection {
        let off1 = Rect::from_min_size(
            rr.min + vec2(8.0, -4.0),
            rr.size(),
        );
        let off2 = Rect::from_min_size(
            rr.min + vec2(14.0, -8.0),
            rr.size(),
        );
        painter.rect_filled(
            off2,
            theme::RADIUS,
            Color32::from_rgba_premultiplied(40, 48, 70, 200),
        );
        painter.rect_stroke(
            off2,
            theme::RADIUS,
            Stroke::new(1.0, theme::PANEL_HOVER),
        );
        painter.rect_filled(
            off1,
            theme::RADIUS,
            Color32::from_rgba_premultiplied(60, 70, 100, 220),
        );
        painter.rect_stroke(
            off1,
            theme::RADIUS,
            Stroke::new(1.0, theme::PANEL_HOVER),
        );
    }
    painter.rect_filled(rr, theme::RADIUS, theme::PANEL);

    if t_focus > 0.01 {
        for i in 1..=4 {
            let glow = rr.expand(2.0 + i as f32 * 3.0 * t_focus);
            let a = ((90 - i * 18).max(0) as f32 * t_focus) as u8;
            painter.rect_stroke(
                glow,
                theme::RADIUS + 2.0 + i as f32 * 2.0,
                Stroke::new(1.2, Color32::from_rgba_premultiplied(0, 180, 255, a)),
            );
        }
    }

    if let Some(tex) = texture {
        let size = tex.size_vec2();
        let img_aspect = size.x / size.y;
        let dst_aspect = poster_rect.width() / poster_rect.height();
        let uv = if img_aspect > dst_aspect {
            let off = (1.0 - dst_aspect / img_aspect) / 2.0;
            Rect::from_min_max(pos2(off, 0.0), pos2(1.0 - off, 1.0))
        } else {
            let off = (1.0 - img_aspect / dst_aspect) / 2.0;
            Rect::from_min_max(pos2(0.0, off), pos2(1.0, 1.0 - off))
        };
        painter.image(tex.id(), poster_rect, uv, Color32::WHITE);
    } else {
        painter.rect_filled(poster_rect, theme::RADIUS, theme::PANEL_HOVER);
        painter.text(
            poster_rect.center(),
            egui::Align2::CENTER_CENTER,
            "…",
            FontId::proportional(24.0),
            theme::TEXT_DIM,
        );
    }

    if data.rating > 0.0 {
        let badge_size = vec2(50.0, 28.0);
        let badge_rect = Rect::from_min_size(
            pos2(poster_rect.right() - badge_size.x - 8.0, poster_rect.top() + 8.0),
            badge_size,
        );
        painter.rect_filled(badge_rect, 6.0, Color32::from_black_alpha(180));
        painter.rect_stroke(badge_rect, 6.0, Stroke::new(1.0, theme::rating_color(data.rating)));
        painter.text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("{:.1}", data.rating),
            FontId::proportional(14.0),
            theme::rating_color(data.rating),
        );
    }

    if let Some(b) = &data.badge {
        let pad = vec2(10.0, 4.0);
        let galley = painter.layout_no_wrap(
            b.clone(),
            FontId::proportional(11.0),
            theme::TEXT_DIM,
        );
        let bg_rect = Rect::from_min_size(
            pos2(poster_rect.left() + 8.0, poster_rect.bottom() - galley.size().y - pad.y * 2.0 - 8.0),
            galley.size() + pad * 2.0,
        );
        painter.rect_filled(bg_rect, 6.0, Color32::from_black_alpha(180));
        painter.galley(bg_rect.min + pad, galley, theme::TEXT);
    }

    painter.text(
        label_rect.center(),
        egui::Align2::CENTER_CENTER,
        elide(&data.title, 28),
        FontId::proportional(14.0),
        theme::TEXT,
    );

    if data.collection {
        let tag_w = 110.0;
        let tag_h = 26.0;
        let tag_rect = Rect::from_min_size(
            pos2(poster_rect.left() + 10.0, poster_rect.top() + 10.0),
            vec2(tag_w, tag_h),
        );
        painter.rect_filled(tag_rect, 6.0, theme::ACCENT);
        painter.text(
            tag_rect.center(),
            egui::Align2::CENTER_CENTER,
            "COLLECTION",
            FontId::proportional(12.0),
            theme::BG,
        );
        if data.count > 0 {
            let count_rect = Rect::from_min_size(
                pos2(poster_rect.right() - 44.0, poster_rect.bottom() - 36.0),
                vec2(36.0, 26.0),
            );
            painter.rect_filled(
                count_rect,
                6.0,
                Color32::from_black_alpha(180),
            );
            painter.text(
                count_rect.center(),
                egui::Align2::CENTER_CENTER,
                format!("×{}", data.count),
                FontId::proportional(13.0),
                theme::TEXT,
            );
        }
    }

    let base_w = 1.0;
    let focus_w = if focused { 3.5 } else { 3.0 };
    let width = base_w + (focus_w - base_w) * t_focus;
    let color = lerp_color(theme::PANEL_HOVER, theme::ACCENT, t_focus);
    painter.rect_stroke(rr, theme::RADIUS, Stroke::new(width, color));

    resp
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), 255)
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}
