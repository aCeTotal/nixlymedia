use egui::{pos2, vec2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};

use crate::app::{App, Screen};
use crate::ui::nav;
use crate::ui::theme;

pub fn draw(app: &mut App, ui: &mut Ui) {
    let online = app.status.as_ref().map(|s| !s.status.is_empty()).unwrap_or(false);
    handle_keys(app, ui);

    ui.add_space(20.0);
    let banner_color = if online { theme::GOOD } else { theme::BAD };
    let banner_text = if online {
        format!(
            "Mediaserver Online — {}",
            app.status.as_ref().map(|s| s.server_name.as_str()).unwrap_or("")
        )
    } else if app.status_err.is_some() {
        "Mediaserver Offline".to_string()
    } else {
        "Kobler til mediaserver…".to_string()
    };

    egui::Frame::none()
        .fill(theme::PANEL)
        .rounding(theme::RADIUS)
        .stroke(Stroke::new(2.0, banner_color))
        .inner_margin(egui::Margin::symmetric(28.0, 22.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("●").color(banner_color).size(28.0));
                ui.add_space(10.0);
                ui.label(RichText::new(banner_text).color(theme::TEXT).size(22.0).strong());
            });
        });

    ui.add_space(40.0);

    if !online {
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Venter på server…")
                    .color(theme::TEXT_DIM)
                    .size(16.0),
            );
        });
        return;
    }

    let avail = ui.available_size();
    let card_w = (avail.x - 32.0) / 2.0;
    let card_h = (avail.y - 60.0).min(420.0).max(240.0);

    ui.horizontal(|ui| {
        if hero_card(
            ui,
            vec2(card_w, card_h),
            "Filmer",
            "Utforsk filmbiblioteket",
            theme::ACCENT,
            Color32::from_rgb(0, 220, 255),
            app.home_focus == 0,
        ) {
            app.home_focus = 0;
            app.grid_focus = 0;
            app.navigate(Screen::MoviesGrid);
            if !app.movies_loaded {
                app.fetch_movies();
            }
        }
        ui.add_space(20.0);
        if hero_card(
            ui,
            vec2(card_w, card_h),
            "Serier",
            "Utforsk seriebiblioteket",
            Color32::from_rgb(200, 100, 255),
            Color32::from_rgb(255, 90, 180),
            app.home_focus == 1,
        ) {
            app.home_focus = 1;
            app.grid_focus = 0;
            app.navigate(Screen::TvShowsGrid);
            if !app.tvshows_loaded {
                app.fetch_tvshows();
            }
        }
    });
}

fn handle_keys(app: &mut App, ui: &Ui) {
    let n = nav::read(app, ui);
    app.home_focus = app.home_focus.min(1);
    if n.left && app.home_focus > 0 {
        app.home_focus -= 1;
    }
    if n.right && app.home_focus < 1 {
        app.home_focus += 1;
    }
    if n.enter {
        app.grid_focus = 0;
        if app.home_focus == 0 {
            app.navigate(Screen::MoviesGrid);
            if !app.movies_loaded {
                app.fetch_movies();
            }
        } else {
            app.navigate(Screen::TvShowsGrid);
            if !app.tvshows_loaded {
                app.fetch_tvshows();
            }
        }
    }
}

fn hero_card(
    ui: &mut Ui,
    size: Vec2,
    title: &str,
    subtitle: &str,
    accent: Color32,
    accent2: Color32,
    focused: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let hovered = resp.hovered() || focused;
    let pressed = resp.is_pointer_button_down_on();

    let pop = if pressed { 0.0 } else if hovered { 8.0 } else { 0.0 };
    let inflated = Rect::from_center_size(rect.center(), rect.size() + Vec2::splat(pop));

    if hovered {
        for i in 1..=6 {
            let r = Rect::from_center_size(
                inflated.center(),
                inflated.size() + Vec2::splat(i as f32 * 4.0),
            );
            let a = (40 - i * 6).max(0) as u8;
            painter.rect_stroke(
                r,
                theme::RADIUS + i as f32,
                Stroke::new(1.0, accent.linear_multiply(a as f32 / 255.0)),
            );
        }
    }

    painter.rect_filled(inflated, theme::RADIUS, theme::PANEL);

    let strips = 48;
    for i in 0..strips {
        let t0 = i as f32 / strips as f32;
        let t1 = (i + 1) as f32 / strips as f32;
        let y0 = inflated.top() + t0 * inflated.height();
        let y1 = inflated.top() + t1 * inflated.height();
        let strip = Rect::from_min_max(pos2(inflated.left(), y0), pos2(inflated.right(), y1 + 0.5));
        let c = lerp_color(accent, accent2, t0);
        let alpha = ((1.0 - t0) * if hovered { 90.0 } else { 55.0 }) as u8;
        painter.rect_filled(
            strip,
            0.0,
            Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha),
        );
    }

    for i in 0..6 {
        let off = i as f32 * 60.0;
        let x0 = inflated.right() - 220.0 + off;
        let p0 = pos2(x0, inflated.bottom());
        let p1 = pos2(x0 + 80.0, inflated.top());
        painter.line_segment(
            [p0, p1],
            Stroke::new(1.5, Color32::from_rgba_unmultiplied(255, 255, 255, 10)),
        );
    }

    let stroke = if hovered {
        Stroke::new(if focused { 3.0 } else { 2.0 }, accent)
    } else {
        Stroke::new(1.0, theme::PANEL_HOVER)
    };
    painter.rect_stroke(inflated, theme::RADIUS, stroke);

    let bar = Rect::from_min_max(
        inflated.left_top(),
        pos2(inflated.right(), inflated.top() + 6.0),
    );
    painter.rect_filled(bar, theme::RADIUS, accent);
    let bar2 = Rect::from_min_max(
        pos2(inflated.left(), inflated.bottom() - 6.0),
        inflated.right_bottom(),
    );
    painter.rect_filled(bar2, theme::RADIUS, accent2);

    let title_pos = inflated.center() - vec2(0.0, 30.0);
    painter.text(
        title_pos + vec2(2.0, 3.0),
        egui::Align2::CENTER_CENTER,
        title,
        FontId::proportional(72.0),
        Color32::from_black_alpha(160),
    );
    painter.text(
        title_pos,
        egui::Align2::CENTER_CENTER,
        title,
        FontId::proportional(72.0),
        theme::TEXT,
    );
    painter.text(
        inflated.center() + vec2(0.0, 32.0),
        egui::Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(16.0),
        Color32::from_rgba_unmultiplied(232, 236, 244, 200),
    );

    let arrow_pos = pos2(inflated.right() - 28.0, inflated.bottom() - 24.0);
    painter.text(
        arrow_pos,
        egui::Align2::RIGHT_BOTTOM,
        if hovered { "→" } else { "›" },
        FontId::proportional(if hovered { 32.0 } else { 26.0 }),
        accent,
    );

    resp.clicked()
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |x: u8, y: u8| -> u8 {
        (x as f32 * (1.0 - t) + y as f32 * t).round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}
