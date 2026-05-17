use std::time::Instant;

use egui::{pos2, vec2, Align2, Color32, Context, FontId, Id, LayerId, Order, Rect, Stroke};

use crate::app::App;
use crate::ui::theme;

const VISIBLE_SECS: f32 = 3.0;
const SLIDE_IN: f32 = 0.25;
const FADE_OUT: f32 = 0.35;
const W: f32 = 420.0;
const H: f32 = 60.0;
const MARGIN: f32 = 22.0;
const GAP: f32 = 10.0;
const MAX_TEXT: usize = 48;

pub struct Toast {
    pub text: String,
    pub born: Instant,
}

pub fn push_movie(app: &mut App, title: &str, year: i32) {
    let label = if year > 0 {
        format!("Ny film: {} ({}) lagt til!", elide(title, MAX_TEXT), year)
    } else {
        format!("Ny film: {} lagt til!", elide(title, MAX_TEXT))
    };
    push(app, label);
}

pub fn push_tvshow(app: &mut App, title: &str) {
    push(app, format!("Ny serie: {} lagt til!", elide(title, MAX_TEXT)));
}

fn push(app: &mut App, text: String) {
    app.toasts.push(Toast {
        text,
        born: Instant::now(),
    });
    if app.toasts.len() > 8 {
        app.toasts.drain(0..app.toasts.len() - 8);
    }
}

pub fn draw(app: &mut App, ctx: &Context) {
    let total_life = VISIBLE_SECS + FADE_OUT;
    app.toasts
        .retain(|t| t.born.elapsed().as_secs_f32() < total_life);
    if app.toasts.is_empty() {
        return;
    }
    ctx.request_repaint();

    let screen = ctx.screen_rect();
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("toasts")));

    for (i, t) in app.toasts.iter().enumerate() {
        let age = t.born.elapsed().as_secs_f32();
        let slide = (age / SLIDE_IN).clamp(0.0, 1.0);
        let slide_eased = 1.0 - (1.0 - slide).powi(3);
        let fade = if age > VISIBLE_SECS {
            1.0 - ((age - VISIBLE_SECS) / FADE_OUT).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let alpha = slide_eased * fade;
        if alpha <= 0.01 {
            continue;
        }

        let x_off = (1.0 - slide_eased) * (W + MARGIN);
        let y = screen.top() + MARGIN + i as f32 * (H + GAP);
        let rect = Rect::from_min_size(
            pos2(screen.right() - MARGIN - W + x_off, y),
            vec2(W, H),
        );

        let bg = theme::PANEL.linear_multiply(alpha);
        let accent = theme::ACCENT.linear_multiply(alpha);
        let text_c = theme::TEXT.linear_multiply(alpha);
        let shadow = Color32::from_black_alpha((140.0 * alpha) as u8);

        let shadow_rect = rect.translate(vec2(0.0, 4.0));
        painter.rect_filled(shadow_rect, theme::RADIUS, shadow);
        painter.rect_filled(rect, theme::RADIUS, bg);
        painter.rect_stroke(rect, theme::RADIUS, Stroke::new(2.0, accent));

        let strip = Rect::from_min_size(rect.left_top(), vec2(6.0, H));
        painter.rect_filled(strip, theme::RADIUS_SM, accent);

        let dot_c = pos2(rect.left() + 26.0, rect.center().y);
        painter.circle_filled(dot_c, 6.0, accent);

        painter.text(
            pos2(rect.left() + 44.0, rect.center().y),
            Align2::LEFT_CENTER,
            &t.text,
            FontId::proportional(15.5),
            text_c,
        );
    }
}

fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}
