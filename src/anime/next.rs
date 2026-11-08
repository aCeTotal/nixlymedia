use std::collections::HashMap;

use super::model::Episode;
use super::state::{Load, SeriesPage};
use super::tracker::Tracked;

type Episodes = HashMap<String, Load<Vec<Episode>>>;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Start,
    Resume,
    Next,
}

pub enum Resolve {
    /* Season episodes not loaded. */
    Needs(String),
    Play {
        season: usize,
        episode: usize,
        kind: Kind,
    },
}

/* Episode a series click plays. */
pub fn resolve(
    page: &SeriesPage,
    episodes: &Episodes,
    tracked: Option<&Tracked>,
) -> Option<Resolve> {
    let Some(t) = tracked else {
        return first_of(page, episodes, 0);
    };
    let season = page
        .seasons
        .iter()
        .position(|s| s.id == t.season_id)
        .unwrap_or(0);
    let id = &page.seasons.get(season)?.id;
    let Some(Load::Ready(list)) = episodes.get(id) else {
        return Some(Resolve::Needs(id.clone()));
    };
    let at = list.iter().position(|e| e.id == t.episode_id).unwrap_or(0);
    if !t.finished() || (at + 1 >= list.len() && season + 1 >= page.seasons.len()) {
        return Some(Resolve::Play {
            season,
            episode: at,
            kind: Kind::Resume,
        });
    }
    if at + 1 < list.len() {
        return Some(Resolve::Play {
            season,
            episode: at + 1,
            kind: Kind::Next,
        });
    }
    first_of(page, episodes, season + 1)
}

/* Only an unstarted series opens on season one. */
fn first_of(page: &SeriesPage, episodes: &Episodes, season: usize) -> Option<Resolve> {
    let kind = if season == 0 { Kind::Start } else { Kind::Next };
    let id = &page.seasons.get(season)?.id;
    match episodes.get(id) {
        Some(Load::Ready(list)) if list.is_empty() => None,
        Some(Load::Ready(_)) => Some(Resolve::Play {
            season,
            episode: 0,
            kind,
        }),
        _ => Some(Resolve::Needs(id.clone())),
    }
}

/* Viewer position as (season, episode). */
pub fn position(page: &SeriesPage, episodes: &Episodes, t: &Tracked) -> Option<(usize, usize)> {
    let season = page.seasons.iter().position(|s| s.id == t.season_id)?;
    let Some(Load::Ready(list)) = episodes.get(&t.season_id) else {
        return Some((season, 0));
    };
    Some((
        season,
        list.iter().position(|e| e.id == t.episode_id).unwrap_or(0),
    ))
}
