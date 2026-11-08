use egui::{pos2, vec2, Align2, FontId, Id, Rect, Ui};

use crate::anime::model::Series;
use crate::anime::state::Load;
use crate::app::App;
use crate::ui::card::{self, CardData};
use crate::ui::theme;

use super::rail::{self, Rail};
use super::tile::{self, Art, Slot};

#[derive(Clone, Copy, PartialEq)]
pub enum Row {
    Continue,
    NewEpisodes,
    Popular,
    Genres,
}

pub const COUNT: usize = 4;
pub const ALL: [Row; COUNT] = [Row::Continue, Row::NewEpisodes, Row::Popular, Row::Genres];

const GAP: f32 = 28.0;
const TITLE_H: f32 = 46.0;
const WIDE_W: f32 = 384.0;
const WIDE_H: f32 = WIDE_W * 9.0 / 16.0 + 64.0;
const TALL_W: f32 = 236.0;
const TALL_H: f32 = TALL_W * 1.62;
const GENRE_W: f32 = 384.0;
const GENRE_H: f32 = GENRE_W * 9.0 / 16.0;
/* Fetch widths, roughly double cards. */
pub const TALL_PX: u32 = 480;
pub const WIDE_PX: u32 = 640;

/* One tile within a row. */
struct Cell {
    row: Row,
    index: usize,
    slot: Slot,
}

/* A row placed on screen. */
pub struct Placed {
    pub row: Row,
    pub rect: Rect,
    pub active: bool,
}

pub fn height(row: Row) -> f32 {
    TITLE_H
        + match row {
            Row::Continue | Row::NewEpisodes => WIDE_H,
            Row::Popular => TALL_H,
            Row::Genres => GENRE_H,
        }
}

fn stride(row: Row) -> f32 {
    GAP + match row {
        Row::Continue | Row::NewEpisodes => WIDE_W,
        Row::Popular => TALL_W,
        Row::Genres => GENRE_W,
    }
}

fn title(row: Row) -> &'static str {
    match row {
        Row::Continue => "Fortsett å se",
        Row::NewEpisodes => "Nye episoder",
        Row::Popular => "Populært",
        Row::Genres => "Sjangre",
    }
}

pub fn len(app: &App, row: Row) -> usize {
    let a = &app.anime;
    match row {
        Row::Continue => a.tracker.len(),
        Row::NewEpisodes => a
            .new_episodes
            .as_ref()
            .and_then(Load::ready)
            .map_or(0, Vec::len),
        Row::Popular => a.popular.as_ref().and_then(Load::ready).map_or(0, Vec::len),
        Row::Genres => a
            .categories
            .as_ref()
            .and_then(Load::ready)
            .map_or(0, Vec::len),
    }
}

pub fn draw(app: &App, ui: &mut Ui, placed: Placed) {
    let Placed { row, rect, active } = placed;
    let color = if active { theme::TEXT } else { theme::TEXT_DIM };
    ui.painter().text(
        rect.left_top(),
        Align2::LEFT_TOP,
        title(row),
        FontId::proportional(24.0),
        color,
    );
    let count = len(app, row);
    let focus = app.anime_ui.cols[row as usize].min(count.saturating_sub(1));
    let stride = stride(row);
    let id = Id::new(("anime_row", row as usize));
    let h_off = rail::offset(
        ui,
        Rail {
            id,
            focus,
            count,
            stride,
            view: rect.width(),
        },
    );
    let size = vec2(stride - GAP, height(row) - TITLE_H);
    for i in 0..count {
        let x = rect.left() + i as f32 * stride - h_off;
        if x > rect.right() || x + stride < rect.left() {
            continue;
        }
        let slot = Slot {
            rect: Rect::from_min_size(pos2(x, rect.top() + TITLE_H), size),
            focused: active && i == focus,
            id: Id::new(("anime_cell", row as usize, i)),
        };
        cell(
            app,
            ui,
            Cell {
                row,
                index: i,
                slot,
            },
        );
    }
}

fn cell(app: &App, ui: &mut Ui, cell: Cell) {
    let Cell {
        row,
        index: i,
        slot,
    } = cell;
    let a = &app.anime;
    let image = |url: Option<&str>| url.and_then(|u| app.images.get(u));
    match row {
        Row::Continue => {
            let Some(t) = a.tracker.recent().get(i) else {
                return;
            };
            let subtitle = format!("{} · {}", t.episode_label, t.episode_title);
            let progress = Some(if t.finished() { 1.0 } else { t.progress() });
            let tex = image(t.image.as_deref());
            tile::wide(
                ui,
                slot,
                Art {
                    title: &t.series_title,
                    subtitle: &subtitle,
                    progress,
                    tex,
                },
            );
        }
        Row::NewEpisodes => {
            let Some(ep) = a
                .new_episodes
                .as_ref()
                .and_then(Load::ready)
                .and_then(|v| v.get(i))
            else {
                return;
            };
            let series = ep
                .episode_metadata
                .as_ref()
                .map_or("", |m| m.series_title.as_str());
            let subtitle = format!("Episode {} · {}", ep.label(), ep.title);
            let tex = image(ep.images.wide(WIDE_PX));
            tile::wide(
                ui,
                slot,
                Art {
                    title: series,
                    subtitle: &subtitle,
                    progress: None,
                    tex,
                },
            );
        }
        Row::Popular => {
            let Some(s) = a
                .popular
                .as_ref()
                .and_then(Load::ready)
                .and_then(|v| v.get(i))
            else {
                return;
            };
            let tex = image(s.images.tall(TALL_PX));
            card::draw(ui, slot.rect, poster_card(s), tex, slot.focused, None);
        }
        Row::Genres => {
            let Some(c) = a
                .categories
                .as_ref()
                .and_then(Load::ready)
                .and_then(|v| v.get(i))
            else {
                return;
            };
            let tex = image(c.background(WIDE_PX));
            tile::genre(
                ui,
                slot,
                Art {
                    title: &c.localization.title,
                    subtitle: "",
                    progress: None,
                    tex,
                },
            );
        }
    }
}

pub fn poster_card(s: &Series) -> CardData {
    CardData {
        title: s.title.clone(),
        rating: 0.0,
        badge: s.meta().series_launch_year.map(|y| y.to_string()),
        collection: false,
        count: 0,
    }
}
