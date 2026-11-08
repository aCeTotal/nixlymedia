use egui::{pos2, vec2, Id, Rect, RichText, Ui, UiBuilder};

use crate::anime::state::Load;
use crate::anime::watch;
use crate::app::{App, Screen};
use crate::ui::{loader, nav, theme};

use super::rail;
use super::row::{self, Placed, Row};

const ROW_GAP: f32 = 30.0;
const V_SCROLL_ANIM: f32 = 0.34;

pub fn draw(app: &mut App, ui: &mut Ui, active: bool) {
    app.anime.ensure_home();
    ui.label(
        RichText::new("Anime")
            .size(28.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(10.0);

    let rows: Vec<Row> = row::ALL
        .into_iter()
        .filter(|r| row::len(app, *r) > 0)
        .collect();
    let Some(&last) = rows.last() else {
        draw_empty(app, ui);
        return;
    };
    app.anime_ui.row = app.anime_ui.row.min(rows.len() - 1);
    if active && handle_nav(app, ui, &rows) {
        return;
    }

    let area = ui.available_rect_before_wrap();
    let tops: Vec<f32> = rows
        .iter()
        .scan(0.0, |y, r| {
            let top = *y;
            *y += row::height(*r) + ROW_GAP;
            Some(top)
        })
        .collect();
    let content_h = tops[rows.len() - 1] + row::height(last);
    let target = tops[app.anime_ui.row].min((content_h - area.height()).max(0.0));
    let v_off = ui
        .ctx()
        .animate_value_with_time(Id::new("anime_home_v"), target, V_SCROLL_ANIM);

    let mut child = ui.new_child(UiBuilder::new().max_rect(area));
    child.set_clip_rect(ui.clip_rect().intersect(area));
    for (i, r) in rows.iter().enumerate() {
        let top = area.top() + tops[i] - v_off;
        let rect = Rect::from_min_size(pos2(area.left(), top), vec2(area.width(), row::height(*r)));
        if rect.bottom() < area.top() || rect.top() > area.bottom() {
            continue;
        }
        let active = active && i == app.anime_ui.row;
        row::draw(
            app,
            &mut child,
            Placed {
                row: *r,
                rect,
                active,
            },
        );
    }
}

fn draw_empty(app: &App, ui: &mut Ui) {
    if !matches!(app.anime.popular, Some(Load::Failed(_))) {
        loader::draw(ui, "Henter anime fra Crunchyroll…");
        return;
    }
    ui.colored_label(theme::BAD, "Fikk ikke kontakt med Crunchyroll.");
}

/* True when screen changed. */
fn handle_nav(app: &mut App, ui: &Ui, rows: &[Row]) -> bool {
    let n = nav::read(app, ui);
    app.anime_ui.row = rail::step(app.anime_ui.row, rows.len(), rail::delta(n.up, n.down));
    let r = rows[app.anime_ui.row];
    let len = row::len(app, r);
    let col = &mut app.anime_ui.cols[r as usize];
    *col = rail::step(*col, len, rail::delta(n.left, n.right));
    if !n.enter {
        return false;
    }
    activate(app, r, app.anime_ui.cols[r as usize]);
    true
}

fn activate(app: &mut App, r: Row, col: usize) {
    let a = &app.anime;
    match r {
        Row::Continue => {
            let Some(id) = a.tracker.recent().get(col).map(|t| t.series_id.clone()) else {
                return;
            };
            app.anime_ui.series = super::series::Focus::autoplay();
            open_series(app, &id);
        }
        Row::NewEpisodes => {
            let ep = a
                .new_episodes
                .as_ref()
                .and_then(Load::ready)
                .and_then(|v| v.get(col))
                .cloned();
            if let Some(ep) = ep {
                watch::play_listed(&mut app.anime, &app.gamepad, &ep);
            }
        }
        Row::Popular => {
            let id = a
                .popular
                .as_ref()
                .and_then(Load::ready)
                .and_then(|v| v.get(col))
                .map(|s| s.id.clone());
            if let Some(id) = id {
                app.anime_ui.series = super::series::Focus::default();
                open_series(app, &id);
            }
        }
        Row::Genres => {
            let id = a
                .categories
                .as_ref()
                .and_then(Load::ready)
                .and_then(|v| v.get(col))
                .map(|c| c.id.clone());
            if let Some(id) = id {
                app.anime.ensure_genre(&id);
                app.anime_ui.genre = 0;
                app.navigate(Screen::AnimeGenre(id));
            }
        }
    }
}

pub fn open_series(app: &mut App, id: &str) {
    app.anime.ensure_series(id);
    app.navigate(Screen::AnimeSeries(id.to_string()));
}
