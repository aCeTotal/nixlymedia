use egui::{
    pos2, scroll_area::ScrollBarVisibility, vec2, Align, Color32, FontId, Frame, Layout, Margin,
    Rect, RichText, Rounding, ScrollArea, Sense, Stroke, Ui,
};

use crate::app::{App, Screen, TvColumn};
use crate::ui::nav;
use crate::ui::theme;

const ROW_H: f32 = 44.0;
const EP_ROW_H: f32 = 84.0;
const COL_GAP: f32 = 12.0;
const BOX_PAD: f32 = 10.0;

pub fn draw(app: &mut App, ui: &mut Ui, key: &str) {
    app.tv_focus.switch_show(key);

    let seasons_list = match app.seasons.get(key).cloned() {
        Some(v) => v,
        None => {
            ui.spinner();
            return;
        }
    };
    if seasons_list.is_empty() {
        ui.label(RichText::new("Ingen sesonger").color(theme::TEXT_DIM));
        return;
    }

    if !app.tv_focus.auto_selected {
        auto_select_next(app, key, &seasons_list);
    }

    app.tv_focus.season_idx = app.tv_focus.season_idx.min(seasons_list.len() - 1);
    let active_season_n = seasons_list[app.tv_focus.season_idx].season;

    if app.selected_season.get(key) != Some(&active_season_n) {
        app.selected_season.insert(key.to_string(), active_season_n);
        if !app.episodes.contains_key(&(key.to_string(), active_season_n)) {
            app.fetch_episodes(key, active_season_n);
        }
    }

    handle_keys(app, ui, &seasons_list);

    let avail = ui.available_size();
    let screen_h = ui.ctx().screen_rect().height();
    let remaining = avail.y.max(0.0);
    let total_h = remaining.clamp(360.0, screen_h - 40.0);
    let col_w = ((avail.x - COL_GAP) / 2.0).max(200.0);

    if app.tv_focus.scroll_pending {
        let anchor = Rect::from_min_size(ui.cursor().min, vec2(10.0, total_h + 20.0));
        ui.scroll_to_rect(anchor, Some(Align::Min));
    }

    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            vec2(col_w, total_h),
            Layout::top_down(Align::Min),
            |ui| {
                box_frame().show(ui, |ui| {
                    ui.set_min_size(vec2(col_w - BOX_PAD * 2.0, total_h - BOX_PAD * 2.0));
                    draw_season_col(app, ui, key, &seasons_list);
                });
            },
        );
        ui.add_space(COL_GAP);
        ui.allocate_ui_with_layout(
            vec2(col_w, total_h),
            Layout::top_down(Align::Min),
            |ui| {
                box_frame().show(ui, |ui| {
                    ui.set_min_size(vec2(col_w - BOX_PAD * 2.0, total_h - BOX_PAD * 2.0));
                    draw_episode_col(app, ui, key, active_season_n);
                });
            },
        );
    });
    app.tv_focus.scroll_pending = false;
}

fn box_frame() -> Frame {
    Frame::none()
        .fill(theme::PANEL)
        .rounding(Rounding::same(theme::RADIUS))
        .stroke(Stroke::new(1.0, theme::PANEL_HOVER))
        .inner_margin(Margin::same(BOX_PAD))
}

fn handle_keys(app: &mut App, ui: &Ui, seasons: &[crate::api::types::SeasonInfo]) {
    let n = nav::read(app, ui);
    let ep_count = app
        .episodes
        .get(&(app.tv_focus.key.clone(), seasons[app.tv_focus.season_idx].season))
        .map(|v| v.len())
        .unwrap_or(0);

    if n.up || n.down || n.left || n.right {
        app.tv_focus.scroll_pending = true;
    }
    match app.tv_focus.column {
        TvColumn::Seasons => {
            if n.up && app.tv_focus.season_idx > 0 {
                app.tv_focus.season_idx -= 1;
            }
            if n.down && app.tv_focus.season_idx + 1 < seasons.len() {
                app.tv_focus.season_idx += 1;
            }
            if n.right || n.enter {
                app.tv_focus.column = TvColumn::Episodes;
                app.tv_focus.episode_idx = 0;
            }
        }
        TvColumn::Episodes => {
            if n.up && app.tv_focus.episode_idx > 0 {
                app.tv_focus.episode_idx -= 1;
            }
            if n.down && app.tv_focus.episode_idx + 1 < ep_count {
                app.tv_focus.episode_idx += 1;
            }
            if n.left {
                app.tv_focus.column = TvColumn::Seasons;
            }
            if n.enter && ep_count > 0 {
                play_focused(app);
            }
        }
    }
}

fn play_focused(app: &mut App) {
    let key = app.tv_focus.key.clone();
    let season = app
        .seasons
        .get(&key)
        .and_then(|v| v.get(app.tv_focus.season_idx))
        .map(|s| s.season);
    let Some(season) = season else { return };
    let eps = app.episodes.get(&(key.clone(), season)).cloned();
    let Some(eps) = eps else { return };
    let ep_idx = app.tv_focus.episode_idx;
    let Some(ep) = eps.get(ep_idx).cloned() else { return };
    let title = ep
        .episode_title
        .clone()
        .or(ep.tmdb_title.clone())
        .unwrap_or_else(|| ep.title.clone());
    let origin = crate::player::view::Origin::Episode {
        show_key: key,
        season,
        episode_idx: ep_idx,
    };
    let resume = app
        .watched
        .get(ep.id)
        .filter(|e| !e.completed)
        .map(|e| e.position)
        .unwrap_or(0.0);
    app.player
        .start(&app.api, ep.id, &title, 0, ep.duration, origin, resume);
    app.navigate(Screen::Player);
}

/* Velg sesong + marker neste usette episode ut fra sist sette. "Sist sett" =
 * siste episode i sesong/episode-rekkefølge som har en watched-entry. Er den
 * ferdigsett → neste episode; er den kun delvis sett → den selv (resume).
 * Krever at alle sesongers episoder er lastet; venter ellers til de er det. */
fn auto_select_next(app: &mut App, key: &str, seasons: &[crate::api::types::SeasonInfo]) {
    for s in seasons {
        if !app.episodes.contains_key(&(key.to_string(), s.season)) {
            return;
        }
    }
    let mut flat: Vec<(usize, usize, i64)> = Vec::new();
    for (si, s) in seasons.iter().enumerate() {
        if let Some(eps) = app.episodes.get(&(key.to_string(), s.season)) {
            for (ei, ep) in eps.iter().enumerate() {
                flat.push((si, ei, ep.id));
            }
        }
    }
    app.tv_focus.auto_selected = true;
    if flat.is_empty() {
        return;
    }
    let last_watched = flat
        .iter()
        .rposition(|(_, _, id)| app.watched.get(*id).is_some());
    let target = match last_watched {
        None => 0,
        Some(p) => {
            let (_, _, id) = flat[p];
            if app.watched.get(id).map(|e| e.completed).unwrap_or(false) {
                (p + 1).min(flat.len() - 1)
            } else {
                p
            }
        }
    };
    let (si, ei, _) = flat[target];
    app.tv_focus.season_idx = si;
    app.tv_focus.episode_idx = ei;
    app.tv_focus.column = TvColumn::Episodes;
    app.tv_focus.scroll_pending = true;
}

fn draw_season_col(
    app: &mut App,
    ui: &mut Ui,
    _key: &str,
    seasons: &[crate::api::types::SeasonInfo],
) {
    ui.label(
        RichText::new("Sesonger")
            .size(14.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(4.0);

    let column_focused = matches!(app.tv_focus.column, TvColumn::Seasons);
    let mut switch_to: Option<usize> = None;

    ScrollArea::vertical()
        .id_salt("seasons_col")
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            for (i, s) in seasons.iter().enumerate() {
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let painter = ui.painter();
                let highlighted = i == app.tv_focus.season_idx;
                let bg = if highlighted && column_focused {
                    theme::ACCENT_SOFT
                } else if highlighted {
                    theme::PANEL_ACTIVE
                } else if resp.hovered() {
                    theme::PANEL_ACTIVE
                } else {
                    theme::PANEL_HOVER
                };
                painter.rect_filled(rect, theme::RADIUS, bg);
                if highlighted {
                    painter.rect_stroke(
                        rect,
                        theme::RADIUS,
                        Stroke::new(
                            if column_focused { 2.5 } else { 1.0 },
                            theme::ACCENT,
                        ),
                    );
                }
                let txt_color = if highlighted && column_focused {
                    theme::BG
                } else {
                    theme::TEXT
                };
                painter.text(
                    rect.left_center() + vec2(16.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("Sesong {}", s.season),
                    FontId::proportional(16.0),
                    txt_color,
                );
                painter.text(
                    rect.right_center() + vec2(-16.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    format!("{} ep", s.episode_count),
                    FontId::proportional(13.0),
                    if highlighted && column_focused {
                        Color32::from_black_alpha(180)
                    } else {
                        theme::TEXT_DIM
                    },
                );
                if highlighted && column_focused && app.tv_focus.scroll_pending {
                    resp.scroll_to_me(Some(Align::Center));
                }
                if resp.hovered()
                    && ui.ctx().input(|inp| inp.pointer.delta() != egui::Vec2::ZERO)
                {
                    app.tv_focus.season_idx = i;
                }
                if resp.clicked() {
                    switch_to = Some(i);
                }
                ui.add_space(6.0);
            }
        });

    if let Some(i) = switch_to {
        app.tv_focus.season_idx = i;
        app.tv_focus.column = TvColumn::Episodes;
        app.tv_focus.episode_idx = 0;
        app.tv_focus.scroll_pending = true;
    }
}

fn draw_episode_col(app: &mut App, ui: &mut Ui, key: &str, season: i32) {
    ui.label(
        RichText::new(format!("Episoder · Sesong {season}"))
            .size(14.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(4.0);

    let episodes = app
        .episodes
        .get(&(key.to_string(), season))
        .cloned()
        .unwrap_or_default();

    if episodes.is_empty() {
        ui.spinner();
        return;
    }

    let column_focused = matches!(app.tv_focus.column, TvColumn::Episodes);
    app.tv_focus.episode_idx = app.tv_focus.episode_idx.min(episodes.len() - 1);

    let mut to_play_idx: Option<usize> = None;

    ScrollArea::vertical()
        .id_salt(format!("episodes_col_{key}_{season}"))
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            for (i, ep) in episodes.iter().enumerate() {
                let still_tex = ep.still.as_deref().and_then(|s| app.images.get(s));
                let (rect, resp) = ui.allocate_exact_size(
                    vec2(ui.available_width(), EP_ROW_H),
                    Sense::click(),
                );
                let painter = ui.painter();
                let highlighted = i == app.tv_focus.episode_idx;
                let bg = if highlighted && column_focused {
                    theme::PANEL_ACTIVE
                } else if resp.hovered() {
                    theme::PANEL_ACTIVE
                } else {
                    theme::PANEL_HOVER
                };
                painter.rect_filled(rect, theme::RADIUS, bg);
                painter.rect_stroke(
                    rect,
                    theme::RADIUS,
                    Stroke::new(
                        if highlighted && column_focused {
                            2.5
                        } else if highlighted || resp.hovered() {
                            1.5
                        } else {
                            1.0
                        },
                        if highlighted && column_focused {
                            theme::ACCENT
                        } else {
                            theme::PANEL_HOVER
                        },
                    ),
                );

                let thumb_h = EP_ROW_H - 12.0;
                let thumb_w = thumb_h * 16.0 / 9.0;
                let thumb_rect = Rect::from_min_size(
                    rect.left_top() + vec2(6.0, 6.0),
                    vec2(thumb_w, thumb_h),
                );
                painter.rect_filled(thumb_rect, theme::RADIUS_SM, theme::BG);
                if let Some(t) = still_tex {
                    painter.image(
                        t.id(),
                        thumb_rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                if app.watched.is_completed(ep.id) {
                    let c = thumb_rect.right_top() + vec2(-12.0, 12.0);
                    painter.circle_filled(c, 11.0, Color32::from_rgb(40, 180, 80));
                    painter.circle_stroke(c, 11.0, Stroke::new(1.5, Color32::from_rgb(20, 110, 50)));
                    painter.text(
                        c,
                        egui::Align2::CENTER_CENTER,
                        "✓",
                        FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                }

                let title = ep
                    .episode_title
                    .clone()
                    .or(ep.tmdb_title.clone())
                    .unwrap_or_else(|| ep.title.clone());
                painter.text(
                    thumb_rect.right_top() + vec2(10.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    format!("E{:02} · {}", ep.episode, title),
                    FontId::proportional(14.0),
                    theme::TEXT,
                );
                let sub = ep
                    .episode_overview
                    .as_deref()
                    .or(ep.overview.as_deref())
                    .unwrap_or("");
                painter.text(
                    thumb_rect.right_top() + vec2(10.0, 22.0),
                    egui::Align2::LEFT_TOP,
                    truncate(sub, 140),
                    FontId::proportional(11.5),
                    theme::TEXT_DIM,
                );
                if ep.duration > 0 {
                    painter.text(
                        rect.right_top() + vec2(-10.0, 6.0),
                        egui::Align2::RIGHT_TOP,
                        format!("{} min", ep.duration / 60),
                        FontId::proportional(11.0),
                        theme::TEXT_DIM,
                    );
                }

                if highlighted && column_focused && app.tv_focus.scroll_pending {
                    resp.scroll_to_me(Some(Align::Center));
                }
                if resp.hovered()
                    && ui.ctx().input(|inp| inp.pointer.delta() != egui::Vec2::ZERO)
                {
                    app.tv_focus.episode_idx = i;
                    app.tv_focus.column = TvColumn::Episodes;
                }
                if resp.clicked() {
                    to_play_idx = Some(i);
                }
                ui.add_space(6.0);
            }
        });

    if let Some(i) = to_play_idx {
        app.tv_focus.episode_idx = i;
        play_focused(app);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}
