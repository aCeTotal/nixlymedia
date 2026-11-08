use egui::Ui;

use crate::anime::next::{self, Resolve};
use crate::anime::state::Load;
use crate::anime::watch;
use crate::app::App;
use crate::ui::{loader, nav, theme};

use super::{rail, series_view};

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Zone {
    #[default]
    Play,
    Seasons,
    Episodes,
}

/* Episode position as (season, episode). */
#[derive(Clone, Copy)]
pub struct Spot {
    pub season: usize,
    pub episode: usize,
}

#[derive(Default)]
pub struct Focus {
    pub zone: Zone,
    pub season: usize,
    pub episode: usize,
    autoplay: bool,
    placed: bool,
}

impl Focus {
    /* Continue-row open plays immediately. */
    pub fn autoplay() -> Self {
        Self {
            autoplay: true,
            ..Self::default()
        }
    }
}

/* What navigation needs from the page. */
struct Scope<'a> {
    id: &'a str,
    seasons: usize,
    next: Option<Spot>,
}

pub fn draw(app: &mut App, ui: &mut Ui, id: &str) {
    app.anime.ensure_series(id);
    let season_ids: Vec<String> = match app.anime.series.get(id) {
        Some(Load::Ready(p)) => p.seasons.iter().map(|s| s.id.clone()).collect(),
        Some(Load::Failed(e)) => {
            ui.colored_label(theme::BAD, format!("Feil: {e}"));
            return;
        }
        _ => {
            loader::draw(ui, "Henter serie…");
            return;
        }
    };
    let focus = &mut app.anime_ui.series;
    focus.season = focus.season.min(season_ids.len().saturating_sub(1));
    if let Some(sid) = season_ids.get(focus.season) {
        app.anime.ensure_episodes(sid);
    }

    let next = match resolve(app, id) {
        Some(Resolve::Needs(sid)) => {
            app.anime.ensure_episodes(&sid);
            None
        }
        Some(Resolve::Play {
            season, episode, ..
        }) => Some(Spot { season, episode }),
        None => None,
    };
    if let Some(spot) = next {
        let focus = &mut app.anime_ui.series;
        if !std::mem::replace(&mut focus.placed, true) {
            focus.season = spot.season;
            focus.episode = spot.episode;
        }
        if std::mem::take(&mut focus.autoplay) {
            play(app, id, spot);
            return;
        }
    }

    if handle_nav(
        app,
        ui,
        Scope {
            id,
            seasons: season_ids.len(),
            next,
        },
    ) {
        return;
    }
    series_view::draw(app, ui, id);
}

pub fn resolve(app: &App, id: &str) -> Option<Resolve> {
    let Some(Load::Ready(page)) = app.anime.series.get(id) else {
        return None;
    };
    next::resolve(page, &app.anime.episodes, app.anime.tracker.get(id))
}

fn episode_count(app: &App, id: &str) -> usize {
    let Some(Load::Ready(page)) = app.anime.series.get(id) else {
        return 0;
    };
    page.seasons
        .get(app.anime_ui.series.season)
        .and_then(|s| app.anime.episodes.get(&s.id))
        .and_then(Load::ready)
        .map_or(0, Vec::len)
}

/* True when playback started. */
fn handle_nav(app: &mut App, ui: &Ui, scope: Scope) -> bool {
    let n = nav::read(app, ui);
    let multi = scope.seasons > 1;
    let episodes = episode_count(app, scope.id);
    let f = &mut app.anime_ui.series;
    match f.zone {
        Zone::Play if n.down => f.zone = if multi { Zone::Seasons } else { Zone::Episodes },
        Zone::Play => {}
        Zone::Seasons => {
            let season = rail::step(f.season, scope.seasons, rail::delta(n.left, n.right));
            if season != f.season {
                f.season = season;
                f.episode = 0;
            }
            if n.up {
                f.zone = Zone::Play;
            }
            if n.down {
                f.zone = Zone::Episodes;
            }
        }
        Zone::Episodes => {
            f.episode = rail::step(f.episode, episodes, rail::delta(n.left, n.right));
            if n.up {
                f.zone = if multi { Zone::Seasons } else { Zone::Play };
            }
        }
    }
    if !n.enter {
        return false;
    }
    let spot = match (f.zone, scope.next) {
        (Zone::Play, Some(spot)) => spot,
        (Zone::Episodes, _) => Spot {
            season: f.season,
            episode: f.episode,
        },
        _ => return false,
    };
    play(app, scope.id, spot);
    true
}

fn play(app: &mut App, id: &str, spot: Spot) {
    let Some(Load::Ready(page)) = app.anime.series.get(id) else {
        return;
    };
    let Some(season) = page.seasons.get(spot.season) else {
        return;
    };
    let Some(Load::Ready(list)) = app.anime.episodes.get(&season.id) else {
        return;
    };
    let Some(ep) = list.get(spot.episode) else {
        return;
    };
    let target = watch::Target {
        series_id: page.series.id.clone(),
        series_title: page.series.title.clone(),
        season_id: season.id.clone(),
        episode: ep.clone(),
    };
    watch::play(&mut app.anime, &app.gamepad, target);
}
