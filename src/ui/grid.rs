use egui::{pos2, scroll_area::ScrollBarVisibility, vec2, Align, Id, Rect, RichText, ScrollArea, Sense, Ui};

use crate::app::{App, Screen};
use crate::library::{self, GridItem};
use crate::ui::card::{self, CardData};
use crate::ui::loader;
use crate::ui::nav;
use crate::ui::theme;

const GAP: f32 = 38.0;
const TOP_PAD: f32 = 18.0;
const SIDE_PAD: f32 = 20.0;
const POS_ANIM: f32 = 0.42;
const CULL_PAD: f32 = 200.0;

fn arrow_nav(app: &mut App, ui: &Ui, focus_field: FocusField, total: usize, cols: usize) -> (bool, bool) {
    if total == 0 {
        let _ = nav::read(app, ui);
        return (false, false);
    }
    let n = nav::read(app, ui);
    let focus = focus_get(app, focus_field);
    let f = focus.min(total - 1);
    let mut new_f = f;
    if n.left && f > 0 {
        new_f = f - 1;
    }
    if n.right && f + 1 < total {
        new_f = f + 1;
    }
    if n.up && f >= cols {
        new_f = f - cols;
    }
    if n.down && f + cols < total {
        new_f = f + cols;
    }
    let changed = new_f != f;
    focus_set(app, focus_field, new_f);
    (n.enter, changed)
}

#[derive(Copy, Clone)]
enum FocusField {
    Grid,
}

fn focus_get(app: &App, f: FocusField) -> usize {
    match f {
        FocusField::Grid => app.grid_focus,
    }
}

fn focus_set(app: &mut App, f: FocusField, v: usize) {
    match f {
        FocusField::Grid => app.grid_focus = v,
    }
}

fn layout(ui: &Ui) -> (usize, f32, f32) {
    let screen = ui.ctx().screen_rect();
    let aspect = screen.width() / screen.height().max(1.0);
    let cols = if aspect >= 2.4 {
        7
    } else if aspect >= 2.0 {
        6
    } else if aspect >= 1.6 {
        5
    } else if aspect >= 1.3 {
        4
    } else {
        3
    };
    let avail = ui.available_width();
    let usable = (avail - 2.0 * SIDE_PAD).max(160.0);
    let card_w = ((usable - (cols as f32 - 1.0) * GAP) / cols as f32).max(160.0);
    let card_h = card_w * 1.62;
    (cols, card_w, card_h)
}

fn animated_rect(
    ui: &Ui,
    salt: &str,
    card_id: &str,
    origin: egui::Pos2,
    content_pos: egui::Vec2,
    size: egui::Vec2,
) -> Rect {
    let base = Id::new(salt).with(card_id);
    let ctx = ui.ctx();
    let cx = ctx.animate_value_with_time(base.with("cx"), content_pos.x, POS_ANIM);
    let cy = ctx.animate_value_with_time(base.with("cy"), content_pos.y, POS_ANIM);
    Rect::from_min_size(pos2(origin.x + cx, origin.y + cy), size)
}

pub fn draw_movies(app: &mut App, ui: &mut Ui, active: bool) {
    ui.label(
        RichText::new("Movies")
            .size(28.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(10.0);

    if app.movies_grid.is_empty() {
        if let Some(err) = &app.movies_err {
            ui.colored_label(theme::BAD, format!("Feil: {err}"));
        } else if app.movies_loaded {
            ui.label(RichText::new("Ingen filmer i biblioteket").color(theme::TEXT_DIM));
        } else {
            loader::draw(ui, "Henter filmbibliotek…");
        }
        return;
    }

    let items = app.movies_grid.clone();
    let (cols, _, _) = layout(ui);
    if active {
        let (enter, changed) = arrow_nav(app, ui, FocusField::Grid, items.len(), cols);
        if changed {
            app.grid_scroll_pending = true;
        }
        if enter {
            if let Some(it) = items.get(app.grid_focus) {
                match it {
                    GridItem::Movie(m) => {
                        let id = m.id;
                        app.navigate(Screen::MovieDetail(id));
                        if !app.details.contains_key(&id) {
                            app.fetch_detail(id);
                        }
                    }
                    GridItem::Collection(c) => {
                        app.navigate(Screen::CollectionView(c.key.clone()));
                        app.grid_focus = 0;
                        app.grid_scroll_pending = true;
                    }
                }
                return;
            }
        }
    }

    let scroll_pending = active && std::mem::take(&mut app.grid_scroll_pending);
    ScrollArea::vertical()
        .id_salt("movies_scroll")
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            let (cols, card_w, card_h) = layout(ui);
            let row_h = card_h + GAP;
            let total_rows = (items.len() + cols - 1) / cols;
            let total_h = TOP_PAD + total_rows as f32 * row_h;
            let avail_w = ui.available_width();
            let (area, _) =
                ui.allocate_exact_size(vec2(avail_w, total_h), Sense::hover());
            let origin = area.left_top();
            let clip = ui.clip_rect();

            for (idx, it) in items.iter().enumerate() {
                let col = idx % cols;
                let row = idx / cols;
                let content_pos = vec2(
                    SIDE_PAD + col as f32 * (card_w + GAP),
                    TOP_PAD + row as f32 * row_h,
                );
                let card_id = library::grid_card_id(it);
                let rect = animated_rect(
                    ui,
                    "movies_grid",
                    &card_id,
                    origin,
                    content_pos,
                    vec2(card_w, card_h),
                );
                let focused = active && idx == app.grid_focus;
                if !focused
                    && (rect.bottom() < clip.top() - CULL_PAD
                        || rect.top() > clip.bottom() + CULL_PAD)
                {
                    continue;
                }
                let insert_age = app
                    .new_card_at
                    .get(&card_id)
                    .map(|t| t.elapsed().as_secs_f32());
                let resp = match it {
                    GridItem::Movie(m) => {
                        let tex = m.poster.as_deref().and_then(|p| app.images.get(p));
                        card::draw(
                            ui,
                            rect,
                            CardData {
                                title: m.display_title().to_string(),
                                rating: m.rating,
                                badge: (m.year > 0).then(|| format!("{}", m.year)),
                                collection: false,
                                count: 0,
                            },
                            tex,
                            focused,
                            insert_age,
                        )
                    }
                    GridItem::Collection(c) => {
                        let tex = c.poster.as_deref().and_then(|p| app.images.get(p));
                        card::draw(
                            ui,
                            rect,
                            CardData {
                                title: c.title.clone(),
                                rating: 0.0,
                                badge: Some(format!("{} filmer", c.movies.len())),
                                collection: true,
                                count: c.movies.len(),
                            },
                            tex,
                            focused,
                            insert_age,
                        )
                    }
                };
                if focused && scroll_pending {
                    resp.scroll_to_me(Some(Align::Center));
                }
            }
        });
}

pub fn draw_collection(app: &mut App, ui: &mut Ui, key: String) {
    let coll = match app.collections.get(&key).cloned() {
        Some(c) => c,
        None => {
            ui.label(RichText::new("Collection ikke funnet").color(theme::TEXT_DIM));
            return;
        }
    };

    ui.label(
        RichText::new(format!("Collection · {}", coll.title))
            .size(28.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(10.0);

    let movies = coll.movies.clone();
    let (cols, _, _) = layout(ui);
    let (enter, changed) = arrow_nav(app, ui, FocusField::Grid, movies.len(), cols);
    if changed {
        app.grid_scroll_pending = true;
    }
    if enter {
        if let Some(m) = movies.get(app.grid_focus) {
            let id = m.id;
            app.navigate(Screen::MovieDetail(id));
            if !app.details.contains_key(&id) {
                app.fetch_detail(id);
            }
            return;
        }
    }

    let scroll_pending = std::mem::take(&mut app.grid_scroll_pending);
    let salt = format!("coll_grid_{key}");
    ScrollArea::vertical()
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            let (cols, card_w, card_h) = layout(ui);
            let row_h = card_h + GAP;
            let total_rows = (movies.len() + cols - 1) / cols;
            let total_h = TOP_PAD + total_rows as f32 * row_h;
            let avail_w = ui.available_width();
            let (area, _) =
                ui.allocate_exact_size(vec2(avail_w, total_h), Sense::hover());
            let origin = area.left_top();
            let clip = ui.clip_rect();

            for (idx, m) in movies.iter().enumerate() {
                let col = idx % cols;
                let row = idx / cols;
                let content_pos = vec2(
                    SIDE_PAD + col as f32 * (card_w + GAP),
                    TOP_PAD + row as f32 * row_h,
                );
                let card_id = format!("m:{}", m.id);
                let rect = animated_rect(
                    ui,
                    &salt,
                    &card_id,
                    origin,
                    content_pos,
                    vec2(card_w, card_h),
                );
                let focused = idx == app.grid_focus;
                if !focused
                    && (rect.bottom() < clip.top() - CULL_PAD
                        || rect.top() > clip.bottom() + CULL_PAD)
                {
                    continue;
                }
                let tex = m.poster.as_deref().and_then(|p| app.images.get(p));
                let resp = card::draw(
                    ui,
                    rect,
                    CardData {
                        title: m.display_title().to_string(),
                        rating: m.rating,
                        badge: (m.year > 0).then(|| format!("{}", m.year)),
                        collection: false,
                        count: 0,
                    },
                    tex,
                    focused,
                    None,
                );
                if focused && scroll_pending {
                    resp.scroll_to_me(Some(Align::Center));
                }
            }
        });
}

pub fn draw_iptv(app: &mut App, ui: &mut Ui, active: bool) {
    ui.label(
        RichText::new("IPTV")
            .size(28.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(10.0);

    if !app.channels_loaded {
        loader::draw(ui, "Henter IPTV-kanaler…");
        return;
    }
    if let Some(err) = &app.channels_err {
        ui.colored_label(theme::BAD, format!("Feil: {err}"));
        return;
    }
    if app.channels.is_empty() {
        ui.label(RichText::new("Ingen kanaler tilgjengelig").color(theme::TEXT_DIM));
        return;
    }
    if !app.epg_loaded {
        loader::draw(ui, "Henter EPG…");
        return;
    }
    if app.iptv_visible.is_empty() {
        ui.label(RichText::new("Ingen kanaler med aktivt program nå").color(theme::TEXT_DIM));
        return;
    }

    let visible: Vec<usize> = app.iptv_visible.clone();
    let now = crate::iptv::epg::now_unix();
    let (cols, _, _) = layout(ui);
    if active {
        let (enter, changed) = arrow_nav(app, ui, FocusField::Grid, visible.len(), cols);
        if changed {
            app.grid_scroll_pending = true;
        }
        if enter {
            if let Some(&ch_idx) = visible.get(app.grid_focus) {
                if let Some(ch) = app.channels.get(ch_idx) {
                    let url = ch.url.clone();
                    let name = ch.name.clone();
                    app.player.start_url(&url, &name);
                    app.navigate(Screen::Player);
                    return;
                }
            }
        }
    }

    let scroll_pending = active && std::mem::take(&mut app.grid_scroll_pending);
    ScrollArea::vertical()
        .id_salt("iptv_scroll")
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            let (cols, card_w, card_h) = layout(ui);
            let row_h = card_h + GAP;
            let total_rows = (visible.len() + cols - 1) / cols;
            let total_h = TOP_PAD + total_rows as f32 * row_h;
            let avail_w = ui.available_width();
            let (area, _) =
                ui.allocate_exact_size(vec2(avail_w, total_h), Sense::hover());
            let origin = area.left_top();
            let clip = ui.clip_rect();

            for (idx, &ch_idx) in visible.iter().enumerate() {
                let Some(ch) = app.channels.get(ch_idx) else { continue };
                let col = idx % cols;
                let row = idx / cols;
                let content_pos = vec2(
                    SIDE_PAD + col as f32 * (card_w + GAP),
                    TOP_PAD + row as f32 * row_h,
                );
                let card_id = ch.epg_id.clone().unwrap_or_else(|| format!("iptv:{ch_idx}"));
                let rect = animated_rect(
                    ui,
                    "iptv_grid",
                    &card_id,
                    origin,
                    content_pos,
                    vec2(card_w, card_h),
                );
                let focused = active && idx == app.grid_focus;
                if !focused
                    && (rect.bottom() < clip.top() - CULL_PAD
                        || rect.top() > clip.bottom() + CULL_PAD)
                {
                    continue;
                }
                let tex = ch.logo.as_deref().and_then(|p| app.images.get(p));
                let badge = ch
                    .epg_id
                    .as_deref()
                    .and_then(|id| app.epg.current(id, now))
                    .map(|p| p.title.clone())
                    .or_else(|| ch.group.clone());
                let resp = card::draw(
                    ui,
                    rect,
                    CardData {
                        title: ch.name.clone(),
                        rating: 0.0,
                        badge,
                        collection: false,
                        count: 0,
                    },
                    tex,
                    focused,
                    None,
                );
                if focused && scroll_pending {
                    resp.scroll_to_me(Some(Align::Center));
                }
            }
        });
}

pub fn draw_tvshows(app: &mut App, ui: &mut Ui, active: bool) {
    ui.label(
        RichText::new("TV-shows")
            .size(28.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(10.0);

    if app.tvshows.is_empty() {
        if let Some(err) = &app.tvshows_err {
            ui.colored_label(theme::BAD, format!("Feil: {err}"));
        } else if app.tvshows_loaded {
            ui.label(RichText::new("Ingen serier i biblioteket").color(theme::TEXT_DIM));
        } else {
            loader::draw(ui, "Henter seriebibliotek…");
        }
        return;
    }

    let shows = app.tvshows.clone();
    let (cols, _, _) = layout(ui);
    if active {
        let (enter, changed) = arrow_nav(app, ui, FocusField::Grid, shows.len(), cols);
        if changed {
            app.grid_scroll_pending = true;
        }
        if enter {
            if let Some(s) = shows.get(app.grid_focus) {
                let key = s.key().to_string();
                let id = s.id;
                app.navigate(Screen::TvShowDetail(key.clone()));
                if !app.seasons.contains_key(&key) {
                    app.fetch_seasons(&key);
                }
                if !app.details.contains_key(&id) {
                    app.fetch_detail(id);
                }
                return;
            }
        }
    }

    let scroll_pending = active && std::mem::take(&mut app.grid_scroll_pending);
    ScrollArea::vertical()
        .id_salt("tvshows_scroll")
        .scroll_bar_visibility(ScrollBarVisibility::AlwaysHidden)
        .show(ui, |ui| {
            let (cols, card_w, card_h) = layout(ui);
            let row_h = card_h + GAP;
            let total_rows = (shows.len() + cols - 1) / cols;
            let total_h = TOP_PAD + total_rows as f32 * row_h;
            let avail_w = ui.available_width();
            let (area, _) =
                ui.allocate_exact_size(vec2(avail_w, total_h), Sense::hover());
            let origin = area.left_top();
            let clip = ui.clip_rect();

            for (idx, s) in shows.iter().enumerate() {
                let col = idx % cols;
                let row = idx / cols;
                let content_pos = vec2(
                    SIDE_PAD + col as f32 * (card_w + GAP),
                    TOP_PAD + row as f32 * row_h,
                );
                let card_id = format!("t:{}", s.key());
                let rect = animated_rect(
                    ui,
                    "tv_grid",
                    &card_id,
                    origin,
                    content_pos,
                    vec2(card_w, card_h),
                );
                let focused = active && idx == app.grid_focus;
                if !focused
                    && (rect.bottom() < clip.top() - CULL_PAD
                        || rect.top() > clip.bottom() + CULL_PAD)
                {
                    continue;
                }
                let insert_age = app
                    .new_card_at
                    .get(&card_id)
                    .map(|t| t.elapsed().as_secs_f32());
                let tex = s.poster.as_deref().and_then(|p| app.images.get(p));
                let badge = (s.episode_count > 0).then(|| format!("{} ep", s.episode_count));
                let resp = card::draw(
                    ui,
                    rect,
                    CardData {
                        title: s.display_title().to_string(),
                        rating: s.rating,
                        badge,
                        collection: false,
                        count: 0,
                    },
                    tex,
                    focused,
                    insert_age,
                );
                if focused && scroll_pending {
                    resp.scroll_to_me(Some(Align::Center));
                }
            }
        });
}
