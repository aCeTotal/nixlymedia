use egui::{pos2, vec2, Id, Rect, RichText, Ui, UiBuilder};

use crate::anime::state::Load;
use crate::app::App;
use crate::ui::{card, loader, nav, theme};

use super::home::open_series;
use super::rail::{self, Rail};
use super::row::{poster_card, TALL_PX};

const CARD_W: f32 = 236.0;
const CARD_H: f32 = CARD_W * 1.62;
const GAP: f32 = 32.0;

/* A genre's poster grid. */
struct Grid<'a> {
    id: &'a str,
    count: usize,
    cols: usize,
}

pub fn draw(app: &mut App, ui: &mut Ui, id: &str) {
    app.anime.ensure_genre(id);
    let title = app
        .anime
        .categories
        .as_ref()
        .and_then(Load::ready)
        .and_then(|cs| cs.iter().find(|c| c.id == id))
        .map_or_else(|| id.to_string(), |c| c.localization.title.clone());
    ui.label(
        RichText::new(format!("Anime · {title}"))
            .size(28.0)
            .color(theme::TEXT)
            .strong(),
    );
    ui.add_space(14.0);

    let count = match app.anime.genres.get(id) {
        Some(Load::Ready(list)) => list.len(),
        Some(Load::Failed(e)) => {
            ui.colored_label(theme::BAD, format!("Feil: {e}"));
            return;
        }
        _ => {
            loader::draw(ui, "Henter serier…");
            return;
        }
    };
    let area = ui.available_rect_before_wrap();
    let cols = (((area.width() + GAP) / (CARD_W + GAP)) as usize).max(1);
    if handle_nav(app, ui, Grid { id, count, cols }) {
        return;
    }

    let focus = app.anime_ui.genre.min(count.saturating_sub(1));
    let stride = CARD_H + GAP;
    let rail = Rail {
        id: Id::new(("anime_genre", id)),
        focus: focus / cols,
        count: count.div_ceil(cols),
        stride,
        view: area.height(),
    };
    let v_off = rail::offset(ui, rail);
    let mut child = ui.new_child(UiBuilder::new().max_rect(area));
    child.set_clip_rect(ui.clip_rect().intersect(area));
    let Some(Load::Ready(list)) = app.anime.genres.get(id) else {
        return;
    };
    for (i, s) in list.iter().enumerate() {
        let pos = pos2(
            area.left() + (i % cols) as f32 * (CARD_W + GAP),
            area.top() + (i / cols) as f32 * stride - v_off,
        );
        let rect = Rect::from_min_size(pos, vec2(CARD_W, CARD_H));
        if rect.bottom() < area.top() || rect.top() > area.bottom() {
            continue;
        }
        let tex = s.images.tall(TALL_PX).and_then(|u| app.images.get(u));
        card::draw(&mut child, rect, poster_card(s), tex, i == focus, None);
    }
}

/* True when screen changed. */
fn handle_nav(app: &mut App, ui: &Ui, grid: Grid) -> bool {
    let n = nav::read(app, ui);
    let f = app.anime_ui.genre.min(grid.count.saturating_sub(1));
    let mut next = rail::step(f, grid.count, rail::delta(n.left, n.right));
    if n.up && f >= grid.cols {
        next = f - grid.cols;
    }
    if n.down && f + grid.cols < grid.count {
        next = f + grid.cols;
    }
    app.anime_ui.genre = next;
    if !n.enter {
        return false;
    }
    let series_id = match app.anime.genres.get(grid.id) {
        Some(Load::Ready(list)) => list.get(next).map(|s| s.id.clone()),
        _ => None,
    };
    let Some(series_id) = series_id else {
        return false;
    };
    app.anime_ui.series = super::series::Focus::default();
    open_series(app, &series_id);
    true
}
