use egui::{
    pos2, scroll_area::ScrollBarVisibility, vec2, Align, Align2, Color32, FontId, Pos2, Rect,
    ScrollArea, Sense, Shape, Stroke, TextureHandle, Ui,
};

use crate::api::types::{CastMember, MediaDetail, TvShow};
use crate::app::{App, Screen};
use crate::ui::{nav, seasons, theme};

const SIDE_PAD: f32 = 48.0;
const RATING_ANIM_SECS: f32 = 0.9;

fn animated_rating(app: &mut App, key: &str, target: f32) -> f32 {
    if app.rating_anim_key.as_deref() != Some(key) {
        app.rating_anim_key = Some(key.to_string());
        app.rating_anim_start = std::time::Instant::now();
    }
    let t = (app.rating_anim_start.elapsed().as_secs_f32() / RATING_ANIM_SECS).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    target * eased
}

pub fn draw_movie(app: &mut App, ui: &mut Ui, id: i64) {
    let Some(d) = app.details.get(&id).cloned() else {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| ui.spinner());
        return;
    };

    let backdrop = d.backdrop.as_deref().and_then(|b| app.images.get(b));
    let poster = d.poster.as_deref().and_then(|p| app.images.get(p));

    let anim_rating = animated_rating(app, &format!("m:{}", id), d.rating);

    ScrollArea::vertical()
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            let n = nav::read(app, ui);
            let title = d
                .tmdb_title
                .clone()
                .unwrap_or_else(|| d.title.clone());

            let info = HeroInfo {
                title: title.clone(),
                year: d.year,
                rating: anim_rating,
                meta_chips: movie_meta_chips(&d),
                genres: parse_genres(d.genres.as_deref()),
                tagline: None,
                show_play: true,
                overview: d.overview.clone(),
                sidebar: movie_sidebar(&d),
            };

            let screen_h = ui.ctx().screen_rect().height();
            let hero_h = (screen_h * 0.96).max(640.0);
            let hero_resp = draw_hero(ui, backdrop.as_ref(), poster.as_ref(), &info, hero_h);

            if !d.cast.is_empty() {
                let cast_top = hero_resp.sidebar_bottom + 22.0;
                let cast_left = hero_resp.info_left;
                let cast_w = hero_resp.info_right - hero_resp.info_left;
                let cast_h = cast_grid_height(cast_w, d.cast.len());
                let cast_rect = Rect::from_min_size(
                    pos2(cast_left, cast_top),
                    vec2(cast_w, cast_h),
                );
                ui.allocate_new_ui(egui::UiBuilder::new().max_rect(cast_rect), |ui| {
                    draw_cast_grid(app, ui, &d.cast, cast_w);
                });
                ui.add_space(28.0);
            }

            if hero_resp.play_clicked || n.enter {
                app.player.start(
                    &app.api,
                    d.id,
                    &title,
                    d.bitrate,
                    d.duration,
                    crate::player::view::Origin::Movie(d.id),
                );
                app.navigate(Screen::Player);
            }
        });
}

pub fn draw_tvshow(app: &mut App, ui: &mut Ui, key: String) {
    let show = app.tvshows.iter().find(|s| s.key() == key).cloned();
    let Some(show) = show else {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| ui.spinner());
        return;
    };

    let backdrop = show.backdrop.as_deref().and_then(|b| app.images.get(b));
    let poster = show.poster.as_deref().and_then(|p| app.images.get(p));

    let anim_rating = animated_rating(app, &format!("t:{}", key), show.rating);

    ScrollArea::vertical()
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            let title = show.display_title().to_string();
            let info = HeroInfo {
                title,
                year: show.year,
                rating: anim_rating,
                meta_chips: tvshow_meta_chips(&show),
                genres: parse_genres(show.genres.as_deref()),
                tagline: None,
                show_play: false,
                overview: show.overview.clone(),
                sidebar: tvshow_sidebar(&show),
            };

            let hero_resp = draw_hero(ui, backdrop.as_ref(), poster.as_ref(), &info, 880.0);

            let screen_rect = ui.ctx().screen_rect();
            let seasons_top = hero_resp.sidebar_bottom + 22.0;
            let seasons_w = hero_resp.info_right - hero_resp.info_left;
            let seasons_h = (screen_rect.bottom() - seasons_top - 32.0).max(260.0);
            let seasons_rect = Rect::from_min_size(
                pos2(hero_resp.info_left, seasons_top),
                vec2(seasons_w, seasons_h),
            );
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(seasons_rect), |ui| {
                seasons::draw(app, ui, &key);
            });
        });
}

struct HeroInfo {
    title: String,
    year: i32,
    rating: f32,
    meta_chips: Vec<String>,
    genres: Vec<String>,
    tagline: Option<String>,
    show_play: bool,
    overview: Option<String>,
    sidebar: Vec<(String, String)>,
}

#[allow(dead_code)]
struct HeroResp {
    play_clicked: bool,
    poster_bottom: f32,
    hero_bottom: f32,
    sidebar_bottom: f32,
    info_left: f32,
    info_right: f32,
}

fn draw_hero(
    ui: &mut Ui,
    backdrop: Option<&TextureHandle>,
    poster: Option<&TextureHandle>,
    info: &HeroInfo,
    hero_h_min: f32,
) -> HeroResp {
    let avail_w = ui.available_width();
    let origin = ui.cursor().min;
    let pad = SIDE_PAD;

    let poster_h = (hero_h_min * 0.72).clamp(360.0, 680.0);
    let poster_w = poster_h / 1.5;
    let poster_top = origin.y + (hero_h_min - poster_h).max(0.0) * 0.32;
    let poster_rect = Rect::from_min_size(
        pos2(origin.x + pad, poster_top),
        vec2(poster_w, poster_h),
    );

    let info_x = poster_rect.right() + 36.0;
    let info_right = origin.x + avail_w - pad;
    let info_w = (info_right - info_x).max(280.0);

    let mut y = poster_rect.top();
    let title_font = FontId::proportional(46.0);
    let title_galley = ui.fonts(|f| {
        f.layout(info.title.clone(), title_font.clone(), theme::TEXT, info_w)
    });
    let title_pos = pos2(info_x, y);
    y += title_galley.size().y + 4.0;

    let year_pos = pos2(info_x, y);
    if info.year > 0 {
        y += 28.0;
    }

    let meta_pos = pos2(info_x, y);
    if !info.meta_chips.is_empty() {
        y += 22.0;
    }

    let chip_font = FontId::proportional(12.5);
    let mut chip_rects: Vec<(Rect, String)> = Vec::new();
    if !info.genres.is_empty() {
        y += 6.0;
        let mut cx = info_x;
        for g in &info.genres {
            let galley = ui.fonts(|f| {
                f.layout_no_wrap(g.clone(), chip_font.clone(), theme::TEXT)
            });
            let cw = galley.size().x + 18.0;
            if cx + cw > info_right {
                cx = info_x;
                y += 28.0;
            }
            chip_rects.push((
                Rect::from_min_size(pos2(cx, y), vec2(cw, 24.0)),
                g.clone(),
            ));
            cx += cw + 8.0;
        }
        y += 30.0;
    } else {
        y += 6.0;
    }

    let rating_r = 38.0;
    let rating_center = pos2(info_x + rating_r, y + rating_r);
    let score_x = rating_center.x + rating_r + 14.0;

    let btn_w = 168.0;
    let btn_h = 44.0;
    let btn_rect = Rect::from_min_size(
        pos2(info_right - btn_w, rating_center.y - btn_h / 2.0),
        vec2(btn_w, btn_h),
    );

    y = rating_center.y + rating_r + 18.0;

    let tagline_paint = info
        .tagline
        .as_ref()
        .filter(|s| !s.is_empty())
        .map(|tag| {
            let p = pos2(info_x, y);
            y += 24.0;
            (p, tag.clone())
        });

    let overview_top = y;
    let ov_text = info
        .overview
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Ingen sammendrag tilgjengelig.");
    let ov_font = FontId::proportional(17.5);
    let ov_w = (info_w * 0.82).max(280.0);
    let ov_galley = ui.fonts(|f| {
        f.layout(
            ov_text.to_string(),
            ov_font.clone(),
            Color32::from_rgba_premultiplied(245, 248, 255, 250),
            ov_w,
        )
    });
    let ov_h = ov_galley.size().y;
    let ov_rect = Rect::from_min_size(pos2(info_x, overview_top), vec2(ov_w, ov_h));

    let sidebar_h_val = sidebar_height(&info.sidebar);
    let sidebar_panel = if !info.sidebar.is_empty() {
        let sy = overview_top + ov_h + 30.0;
        Some(Rect::from_min_size(
            pos2(info_x, sy),
            vec2(info_right - info_x, sidebar_h_val + 16.0),
        ))
    } else {
        None
    };

    let mut content_bottom = poster_rect.bottom().max(ov_rect.bottom());
    if let Some(p) = sidebar_panel {
        content_bottom = content_bottom.max(p.bottom());
    }
    let final_hero_h = content_bottom - origin.y;

    let (hero, _) = ui.allocate_exact_size(vec2(avail_w, final_hero_h), Sense::hover());
    let painter = ui.painter().clone();

    if let Some(t) = backdrop {
        paint_cover(&painter, t, hero);
    } else {
        painter.rect_filled(hero, 0.0, theme::PANEL);
    }
    paint_vertical_shade(&painter, hero);
    paint_left_shade(&painter, hero);

    paint_poster(&painter, poster, poster_rect);

    painter.galley(title_pos, title_galley, theme::TEXT);

    if info.year > 0 {
        let s = format!("({})", info.year);
        let f = FontId::proportional(20.0);
        painter.text(
            year_pos + vec2(1.0, 1.0),
            Align2::LEFT_TOP,
            &s,
            f.clone(),
            Color32::from_black_alpha(200),
        );
        painter.text(
            year_pos,
            Align2::LEFT_TOP,
            s,
            f,
            Color32::from_rgb(230, 236, 248),
        );
    }

    if !info.meta_chips.is_empty() {
        let s = info.meta_chips.join("  •  ");
        let f = FontId::proportional(14.0);
        painter.text(
            meta_pos + vec2(1.0, 1.0),
            Align2::LEFT_TOP,
            &s,
            f.clone(),
            Color32::from_black_alpha(200),
        );
        painter.text(
            meta_pos,
            Align2::LEFT_TOP,
            s,
            f,
            Color32::from_rgb(220, 226, 238),
        );
    }

    for (r, g) in &chip_rects {
        painter.rect_filled(*r, 12.0, Color32::from_rgba_premultiplied(10, 14, 22, 200));
        painter.rect_stroke(
            *r,
            12.0,
            Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 60)),
        );
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            g,
            chip_font.clone(),
            theme::TEXT,
        );
    }

    draw_rating_ring(&painter, rating_center, rating_r, info.rating);
    painter.text(
        pos2(score_x, rating_center.y - 6.0),
        Align2::LEFT_BOTTOM,
        "Brukerscore",
        FontId::proportional(12.0),
        theme::TEXT_DIM,
    );
    painter.text(
        pos2(score_x, rating_center.y + 2.0),
        Align2::LEFT_TOP,
        if info.rating > 0.0 {
            format!("{:.1} / 10", info.rating)
        } else {
            "Ingen rating".to_string()
        },
        FontId::proportional(14.5),
        theme::TEXT,
    );

    let mut play_clicked = false;
    if info.show_play {
        let resp = ui.interact(btn_rect, ui.id().with("hero_play_btn"), Sense::click());
        let hovered = resp.hovered();
        let fill = if hovered { theme::ACCENT } else { theme::ACCENT_SOFT };
        painter.rect_filled(btn_rect, 10.0, fill);
        painter.text(
            btn_rect.center(),
            Align2::CENTER_CENTER,
            "▶  Spill av",
            FontId::proportional(17.0),
            theme::BG,
        );
        if resp.clicked() {
            play_clicked = true;
        }
    }

    if let Some((tp, tag)) = tagline_paint {
        painter.text(
            tp,
            Align2::LEFT_TOP,
            format!("\u{201C}{}\u{201D}", tag),
            FontId::proportional(15.0),
            Color32::from_rgba_premultiplied(220, 224, 232, 200),
        );
    }

    paint_clipped_galley(&painter, ov_rect, ov_galley);

    if let Some(panel) = sidebar_panel {
        let bg = Color32::from_rgba_premultiplied(8, 12, 20, 170);
        painter.rect_filled(panel, theme::RADIUS, bg);
        painter.rect_stroke(
            panel,
            theme::RADIUS,
            Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 22)),
        );
        paint_sidebar_row(&painter, ui, &info.sidebar, panel.shrink2(vec2(18.0, 8.0)));
    }

    HeroResp {
        play_clicked,
        poster_bottom: poster_rect.bottom(),
        hero_bottom: hero.bottom(),
        sidebar_bottom: sidebar_panel.map(|p| p.bottom()).unwrap_or(ov_rect.bottom()),
        info_left: info_x,
        info_right,
    }
}

fn paint_clipped_galley(
    painter: &egui::Painter,
    rect: Rect,
    galley: std::sync::Arc<egui::Galley>,
) {
    let prev = painter.clip_rect();
    let mut clip = prev;
    clip = clip.intersect(rect);
    let sub = painter.with_clip_rect(clip);
    sub.galley(rect.left_top(), galley, theme::TEXT);
}

fn sidebar_height(items: &[(String, String)]) -> f32 {
    if items.is_empty() {
        return 0.0;
    }
    let pairs = items.iter().filter(|(_, v)| !v.trim().is_empty()).count();
    let cols = 4usize;
    let rows = ((pairs + cols - 1) / cols).max(1);
    let row_h = 38.0;
    row_h * rows as f32
}

fn paint_sidebar_row(
    painter: &egui::Painter,
    ui: &Ui,
    items: &[(String, String)],
    rect: Rect,
) {
    let visible: Vec<&(String, String)> = items
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .collect();
    if visible.is_empty() {
        return;
    }
    let cols = 4usize;
    let rows = ((visible.len() + cols - 1) / cols).max(1);
    let cell_w = rect.width() / cols as f32;
    let cell_h = rect.height() / rows as f32;
    let key_font = FontId::proportional(11.5);
    let val_font = FontId::proportional(14.0);
    for (i, (k, v)) in visible.iter().enumerate() {
        let r = i / cols;
        let c = i % cols;
        let cell_x = rect.left() + c as f32 * cell_w;
        let cell_y = rect.top() + r as f32 * cell_h;
        let key_pos = pos2(cell_x, cell_y);
        painter.text(
            key_pos,
            Align2::LEFT_TOP,
            k.to_uppercase(),
            key_font.clone(),
            theme::TEXT_DIM,
        );
        let val_galley = ui.fonts(|f| {
            f.layout(
                v.to_string(),
                val_font.clone(),
                theme::TEXT,
                cell_w - 14.0,
            )
        });
        let val_rect = Rect::from_min_size(
            pos2(cell_x, cell_y + 16.0),
            vec2(cell_w - 14.0, cell_h - 16.0),
        );
        paint_clipped_galley(painter, val_rect, val_galley);
    }
}

fn paint_cover(painter: &egui::Painter, t: &TextureHandle, rect: Rect) {
    let size = t.size_vec2();
    let img_aspect = size.x / size.y;
    let dst_aspect = rect.width() / rect.height().max(1.0);
    let uv = if img_aspect > dst_aspect {
        let off = (1.0 - dst_aspect / img_aspect) / 2.0;
        Rect::from_min_max(pos2(off, 0.0), pos2(1.0 - off, 1.0))
    } else {
        let off = (1.0 - img_aspect / dst_aspect) / 2.0;
        Rect::from_min_max(pos2(0.0, off), pos2(1.0, 1.0 - off))
    };
    painter.image(t.id(), rect, uv, Color32::WHITE);
}

fn paint_vertical_shade(painter: &egui::Painter, hero: Rect) {
    let strips = 80;
    let bg = theme::BG;
    for i in 0..strips {
        let t = i as f32 / strips as f32;
        let y0 = hero.top() + t * hero.height();
        let y1 = y0 + hero.height() / strips as f32 + 1.0;
        let strip = Rect::from_min_max(
            pos2(hero.left(), y0),
            pos2(hero.right(), y1),
        );
        let top_alpha = ((1.0 - t).powf(1.5) * 70.0) as u8;
        let bot_alpha = (t.powf(2.0) * 245.0) as u8;
        let alpha = top_alpha.max(bot_alpha);
        painter.rect_filled(
            strip,
            0.0,
            Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), alpha),
        );
    }
}

fn paint_left_shade(painter: &egui::Painter, hero: Rect) {
    let strips = 80;
    let w = hero.width() * 0.62;
    for i in 0..strips {
        let t = i as f32 / strips as f32;
        let x0 = hero.left() + t * w;
        let x1 = x0 + w / strips as f32 + 1.0;
        let strip = Rect::from_min_max(
            pos2(x0, hero.top()),
            pos2(x1, hero.bottom()),
        );
        let alpha = ((1.0 - t).powf(1.6) * 195.0) as u8;
        painter.rect_filled(
            strip,
            0.0,
            Color32::from_rgba_unmultiplied(0, 0, 0, alpha),
        );
    }
}

fn paint_poster(painter: &egui::Painter, tex: Option<&TextureHandle>, rect: Rect) {
    for i in 1..=8 {
        let r = rect.expand(i as f32 * 1.6);
        let a = ((60 - i * 7).max(0)) as u8;
        painter.rect_filled(
            r,
            theme::RADIUS + i as f32 * 0.6,
            Color32::from_rgba_premultiplied(0, 0, 0, a),
        );
    }
    painter.rect_filled(rect, theme::RADIUS, theme::PANEL);
    if let Some(t) = tex {
        let size = t.size_vec2();
        let img_aspect = size.x / size.y;
        let dst_aspect = rect.width() / rect.height();
        let uv = if img_aspect > dst_aspect {
            let off = (1.0 - dst_aspect / img_aspect) / 2.0;
            Rect::from_min_max(pos2(off, 0.0), pos2(1.0 - off, 1.0))
        } else {
            let off = (1.0 - img_aspect / dst_aspect) / 2.0;
            Rect::from_min_max(pos2(0.0, off), pos2(1.0, 1.0 - off))
        };
        painter.image(t.id(), rect, uv, Color32::WHITE);
    }
    painter.rect_stroke(
        rect,
        theme::RADIUS,
        Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 40)),
    );
}

fn draw_rating_ring(painter: &egui::Painter, center: Pos2, radius: f32, rating: f32) {
    let bg = Color32::from_rgb(15, 22, 30);
    let track = Color32::from_rgba_premultiplied(60, 70, 90, 200);
    painter.circle_filled(center, radius, bg);
    let stroke_w = 6.0;

    let segs = 96;
    let tau = std::f32::consts::TAU;
    let start = -std::f32::consts::FRAC_PI_2;
    let mut track_points: Vec<Pos2> = Vec::with_capacity(segs + 1);
    for i in 0..=segs {
        let a = start + (i as f32 / segs as f32) * tau;
        track_points.push(pos2(
            center.x + a.cos() * (radius - stroke_w / 2.0),
            center.y + a.sin() * (radius - stroke_w / 2.0),
        ));
    }
    painter.add(Shape::line(track_points, Stroke::new(stroke_w, track)));

    if rating > 0.0 {
        let frac = (rating / 10.0).clamp(0.0, 1.0);
        let color = rating_arc_color(rating);
        let n = (segs as f32 * frac).ceil() as usize;
        let mut pts: Vec<Pos2> = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let prog = (i as f32 / segs as f32).min(frac);
            let a = start + prog * tau;
            pts.push(pos2(
                center.x + a.cos() * (radius - stroke_w / 2.0),
                center.y + a.sin() * (radius - stroke_w / 2.0),
            ));
        }
        painter.add(Shape::line(pts, Stroke::new(stroke_w, color)));
    }

    let text = if rating > 0.0 {
        format!("{:.0}%", (rating * 10.0).round())
    } else {
        "NR".to_string()
    };
    painter.text(
        center,
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(if rating > 0.0 { 16.0 } else { 13.0 }),
        theme::TEXT,
    );
}

fn rating_arc_color(rating: f32) -> Color32 {
    if rating >= 7.0 {
        Color32::from_rgb(33, 208, 122)
    } else if rating >= 4.0 {
        Color32::from_rgb(210, 213, 79)
    } else {
        Color32::from_rgb(219, 80, 80)
    }
}

fn movie_meta_chips(d: &MediaDetail) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(rd) = d.release_date.as_deref().filter(|s| !s.is_empty()) {
        v.push(rd.to_string());
    } else if d.year > 0 {
        v.push(d.year.to_string());
    }
    if d.duration > 0 {
        v.push(format_runtime(d.duration));
    }
    if d.width > 0 && d.height > 0 {
        v.push(resolution_label(d.width, d.height));
    }
    if let Some(h) = d.hdr_format.as_deref().filter(|s| !s.is_empty()) {
        v.push(h.to_string());
    }
    if let Some(lay) = d.audio_layout.as_deref().filter(|s| !s.is_empty()) {
        v.push(lay.to_string());
    }
    if let Some(feat) = d.audio_features.as_deref().filter(|s| !s.is_empty()) {
        for f in feat.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            v.push(audio_feature_label(f));
        }
    }
    v
}

fn audio_feature_label(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "atmos" => "Atmos".into(),
        "joc" => "JOC".into(),
        "dts-x" => "DTS:X".into(),
        "dts-hd-ma" => "DTS-HD MA".into(),
        other => other.to_uppercase(),
    }
}

fn movie_sidebar(d: &MediaDetail) -> Vec<(String, String)> {
    let mut v = Vec::new();
    v.push((
        "Status".to_string(),
        if d.release_date.is_some() || d.year > 0 {
            "Released".to_string()
        } else {
            "Unknown".to_string()
        },
    ));
    if let Some(rd) = d.release_date.as_deref().filter(|s| !s.is_empty()) {
        v.push(("Premiere".to_string(), rd.to_string()));
    }
    if d.duration > 0 {
        v.push(("Spilletid".to_string(), format_runtime(d.duration)));
    }
    if d.width > 0 && d.height > 0 {
        v.push((
            "Oppløsning".to_string(),
            format!("{}×{}", d.width, d.height),
        ));
    }
    if let Some(cv) = d.codec_video.as_deref().filter(|s| !s.is_empty()) {
        v.push(("Video codec".to_string(), cv.to_uppercase()));
    }
    if let Some(ca) = d.codec_audio.as_deref().filter(|s| !s.is_empty()) {
        v.push(("Audio codec".to_string(), ca.to_uppercase()));
    }
    if let Some(lay) = d.audio_layout.as_deref().filter(|s| !s.is_empty()) {
        let mut s = lay.to_string();
        if let Some(feat) = d.audio_features.as_deref().filter(|s| !s.is_empty()) {
            let labels: Vec<String> = feat
                .split(',')
                .map(|x| x.trim())
                .filter(|x| !x.is_empty())
                .map(audio_feature_label)
                .collect();
            if !labels.is_empty() {
                s = format!("{} · {}", s, labels.join(" · "));
            }
        }
        v.push(("Lyd".to_string(), s));
    }
    if let Some(h) = d.hdr_format.as_deref().filter(|s| !s.is_empty()) {
        let val = if d.bit_depth > 0 {
            format!("{} ({}-bit)", h, d.bit_depth)
        } else {
            h.to_string()
        };
        v.push(("HDR".to_string(), val));
    }
    if d.bitrate > 0 {
        v.push((
            "Bitrate".to_string(),
            format!("{} Mbps", (d.bitrate as f64 / 1_000_000.0).round() as i64),
        ));
    }
    if d.size > 0 {
        v.push(("Filstørrelse".to_string(), format_bytes(d.size)));
    }
    v
}

fn tvshow_meta_chips(s: &TvShow) -> Vec<String> {
    let mut v = Vec::new();
    if s.year > 0 {
        v.push(s.year.to_string());
    }
    if s.tmdb_total_seasons > 0 {
        v.push(format!("{} sesonger", s.tmdb_total_seasons));
    }
    if s.tmdb_episode_runtime > 0 {
        v.push(format!("{} min/ep", s.tmdb_episode_runtime));
    }
    if let Some(st) = s.tmdb_status.as_deref().filter(|s| !s.is_empty()) {
        v.push(st.to_string());
    }
    v
}

fn tvshow_sidebar(s: &TvShow) -> Vec<(String, String)> {
    let mut v = Vec::new();
    v.push((
        "Status".to_string(),
        s.tmdb_status
            .clone()
            .filter(|x| !x.is_empty())
            .unwrap_or_else(|| "Unknown".to_string()),
    ));
    if s.tmdb_total_seasons > 0 {
        v.push(("Sesonger".to_string(), s.tmdb_total_seasons.to_string()));
    }
    let ep_text = if s.tmdb_total_episodes > 0 {
        format!("{} / {}", s.episode_count, s.tmdb_total_episodes)
    } else {
        s.episode_count.to_string()
    };
    v.push(("Episoder".to_string(), ep_text));
    if let Some(next) = s.tmdb_next_episode.as_deref().filter(|s| !s.is_empty()) {
        v.push(("Neste episode".to_string(), next.to_string()));
    }
    if s.tmdb_episode_runtime > 0 {
        v.push((
            "Lengde / ep".to_string(),
            format!("{} min", s.tmdb_episode_runtime),
        ));
    }
    if s.size > 0 {
        v.push(("Filstørrelse".to_string(), format_bytes(s.size)));
    }
    v
}

fn parse_genres(s: Option<&str>) -> Vec<String> {
    s.unwrap_or("")
        .split(|c: char| c == ',' || c == '|' || c == ';')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

fn format_runtime(secs: i32) -> String {
    let total_min = secs / 60;
    let h = total_min / 60;
    let m = total_min % 60;
    if h > 0 {
        format!("{}t {}m", h, m)
    } else {
        format!("{} min", m)
    }
}

fn resolution_label(w: i32, h: i32) -> String {
    let p = h.max(w * 9 / 16);
    if p >= 2000 {
        "4K".to_string()
    } else if p >= 1000 {
        "1080p".to_string()
    } else if p >= 700 {
        "720p".to_string()
    } else {
        format!("{}×{}", w, h)
    }
}

fn format_bytes(n: i64) -> String {
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    let nf = n as f64;
    if nf >= GB {
        format!("{:.1} GB", nf / GB)
    } else {
        format!("{:.0} MB", nf / MB)
    }
}

const CAST_CARD_W: f32 = 140.0;
const CAST_PHOTO_H: f32 = 210.0;
const CAST_CARD_H: f32 = CAST_PHOTO_H + 68.0;
const CAST_GAP: f32 = 14.0;

const CAST_LIMIT: usize = 9;

fn cast_grid_height(_width: f32, n: usize) -> f32 {
    if n == 0 { return 0.0; }
    let header = 38.0;
    header + CAST_CARD_H + 4.0
}

fn draw_cast_grid(app: &mut App, ui: &mut Ui, cast: &[CastMember], _width: f32) {
    ui.label(
        egui::RichText::new("Skuespillere")
            .size(20.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(10.0);

    ui.horizontal(|ui| {
        for c in cast.iter().take(CAST_LIMIT) {
            draw_cast_card(app, ui, c);
            ui.add_space(CAST_GAP);
        }
    });
}

fn draw_cast_card(app: &mut App, ui: &mut Ui, c: &CastMember) {
    let (rect, _) = ui.allocate_exact_size(vec2(CAST_CARD_W, CAST_CARD_H), Sense::hover());
    let painter = ui.painter().clone();

    let photo_rect = Rect::from_min_size(rect.left_top(), vec2(CAST_CARD_W, CAST_PHOTO_H));
    painter.rect_filled(photo_rect, theme::RADIUS, theme::PANEL);

    let tex = c.profile.as_deref().and_then(|p| app.images.get(p));
    if let Some(t) = tex {
        let size = t.size_vec2();
        let img_aspect = size.x / size.y;
        let dst_aspect = photo_rect.width() / photo_rect.height();
        let uv = if img_aspect > dst_aspect {
            let off = (1.0 - dst_aspect / img_aspect) / 2.0;
            Rect::from_min_max(pos2(off, 0.0), pos2(1.0 - off, 1.0))
        } else {
            let off = (1.0 - img_aspect / dst_aspect) / 2.0;
            Rect::from_min_max(pos2(0.0, off), pos2(1.0, 1.0 - off))
        };
        painter.image(t.id(), photo_rect, uv, Color32::WHITE);
    } else {
        painter.text(
            photo_rect.center(),
            Align2::CENTER_CENTER,
            initials(&c.name),
            FontId::proportional(36.0),
            theme::TEXT_DIM,
        );
    }
    painter.rect_stroke(
        photo_rect,
        theme::RADIUS,
        Stroke::new(1.0, Color32::from_rgba_premultiplied(255, 255, 255, 24)),
    );

    let name_font = FontId::proportional(13.5);
    let name_galley = ui.fonts(|f| {
        f.layout(c.name.clone(), name_font.clone(), theme::TEXT, CAST_CARD_W - 4.0)
    });
    let name_pos = pos2(rect.left() + 2.0, photo_rect.bottom() + 8.0);
    let name_h = name_galley.size().y.min(34.0);
    let name_rect = Rect::from_min_size(name_pos, vec2(CAST_CARD_W - 4.0, name_h));
    paint_clipped_galley(&painter, name_rect, name_galley);

    let ch_font = FontId::proportional(12.0);
    let ch_galley = ui.fonts(|f| {
        f.layout(
            c.character.clone(),
            ch_font.clone(),
            theme::TEXT_DIM,
            CAST_CARD_W - 4.0,
        )
    });
    let ch_pos = pos2(rect.left() + 2.0, photo_rect.bottom() + 8.0 + name_h + 4.0);
    let ch_rect = Rect::from_min_size(ch_pos, vec2(CAST_CARD_W - 4.0, 28.0));
    paint_clipped_galley(&painter, ch_rect, ch_galley);
    let _ = Align::Min;
}

fn initials(s: &str) -> String {
    s.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}
