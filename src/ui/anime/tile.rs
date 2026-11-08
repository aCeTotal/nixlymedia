use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    pos2, vec2, Align2, Color32, FontId, Id, Image, Painter, Pos2, Rect, Stroke, TextureHandle, Ui,
};

use crate::ui::card::{cover_uv, lerp_color, paint_glow};
use crate::ui::theme;

const FOCUS_ANIM: f32 = 0.18;
const PROGRESS_H: f32 = 5.0;

/* Where a focusable tile sits. */
#[derive(Clone, Copy)]
pub struct Slot {
    pub rect: Rect,
    pub focused: bool,
    pub id: Id,
}

pub struct Art<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub progress: Option<f32>,
    pub tex: Option<TextureHandle>,
}

pub struct Text<'a> {
    pub text: &'a str,
    pub size: f32,
    pub color: Color32,
    pub max_w: f32,
}

/* 16:9 image, two caption lines. */
pub fn wide(ui: &Ui, slot: Slot, art: Art) {
    let rect = slot.rect;
    let image = Rect::from_min_size(rect.min, vec2(rect.width(), rect.width() * 9.0 / 16.0));
    frame(
        ui,
        Slot {
            rect: image,
            ..slot
        },
        art.tex,
    );
    let painter = ui.painter();
    if let Some(p) = art.progress {
        let track = Rect::from_min_max(
            pos2(image.left(), image.bottom() - PROGRESS_H),
            image.right_bottom(),
        );
        painter.rect_filled(track, 0.0, Color32::from_black_alpha(160));
        let fill = Rect::from_min_size(track.min, vec2(track.width() * p, PROGRESS_H));
        painter.rect_filled(fill, 0.0, theme::ACCENT);
    }
    let y = image.bottom() + 10.0;
    let max_w = rect.width();
    line(
        painter,
        pos2(rect.left(), y),
        Text {
            text: art.title,
            size: 19.0,
            color: theme::TEXT,
            max_w,
        },
    );
    line(
        painter,
        pos2(rect.left(), y + 26.0),
        Text {
            text: art.subtitle,
            size: 15.0,
            color: theme::TEXT_DIM,
            max_w,
        },
    );
}

/* Genre background, centred title. */
pub fn genre(ui: &Ui, slot: Slot, art: Art) {
    let t = frame(ui, slot, art.tex);
    let painter = ui.painter();
    let dim = Color32::from_black_alpha(110 - (60.0 * t) as u8);
    painter.rect_filled(slot.rect, theme::RADIUS, dim);
    painter.text(
        slot.rect.center(),
        Align2::CENTER_CENTER,
        art.title,
        FontId::proportional(28.0),
        Color32::WHITE,
    );
}

/* Rounded image with focus ring; returns focus. */
fn frame(ui: &Ui, slot: Slot, tex: Option<TextureHandle>) -> f32 {
    let t = ui
        .ctx()
        .animate_bool_with_time(slot.id, slot.focused, FOCUS_ANIM);
    let painter = ui.painter();
    painter.rect_filled(slot.rect, theme::RADIUS, theme::PANEL);
    paint_glow(painter, slot.rect, t);
    if let Some(tex) = tex {
        Image::new(&tex)
            .uv(cover_uv(tex.size_vec2(), slot.rect))
            .rounding(theme::RADIUS)
            .paint_at(ui, slot.rect);
    }
    let color = lerp_color(theme::PANEL_HOVER, theme::ACCENT, t);
    painter.rect_stroke(slot.rect, theme::RADIUS, Stroke::new(1.0 + 2.5 * t, color));
    t
}

/* One line, ellipsis truncated. */
pub fn line(painter: &Painter, at: Pos2, text: Text) {
    let mut job = LayoutJob::single_section(
        text.text.to_owned(),
        TextFormat::simple(FontId::proportional(text.size), text.color),
    );
    job.wrap = TextWrapping::truncate_at_width(text.max_w);
    let galley = painter.layout_job(job);
    painter.galley(at, galley, text.color);
}
