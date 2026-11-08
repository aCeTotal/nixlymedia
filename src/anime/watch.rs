use std::sync::mpsc::channel;

use super::model::Episode;
use super::session::Session;
use super::state::{AnimeState, Load};
use super::tracker::Tracked;
use crate::input::gamepad::Gamepad;
use crate::ui::anime::row::WIDE_PX;

const WATCH_HOST: &str = "https://www.crunchyroll.com/watch";

pub struct Target {
    pub series_id: String,
    pub series_title: String,
    pub season_id: String,
    pub episode: Episode,
}

pub struct Active {
    session: Session,
    target: Target,
}

pub fn play(state: &mut AnimeState, pad: &Gamepad, target: Target) {
    if state.watch.is_some() {
        return;
    }
    let ep = &target.episode;
    let url = format!("{WATCH_HOST}/{}/{}", ep.id, ep.slug_title);
    let (tx, rx) = channel();
    pad.redirect(Some(tx));
    match Session::start(&url, rx) {
        Ok(session) => state.watch = Some(Active { session, target }),
        Err(e) => {
            pad.redirect(None);
            crate::nlog!("crunchyroll: kunne ikke starte nettleser: {e}");
        }
    }
}

/* Browse episodes carry series context. */
pub fn play_listed(state: &mut AnimeState, pad: &Gamepad, ep: &Episode) {
    let Some(m) = &ep.episode_metadata else {
        return;
    };
    let target = Target {
        series_id: m.series_id.clone(),
        series_title: m.series_title.clone(),
        season_id: m.season_id.clone(),
        episode: ep.clone(),
    };
    play(state, pad, target);
}

/* Records progress after browser exit. */
pub fn finish_if_ended(state: &mut AnimeState, pad: &Gamepad) {
    if !state.watch.as_ref().is_some_and(|a| a.session.ended()) {
        return;
    }
    let Some(active) = state.watch.take() else {
        return;
    };
    pad.redirect(None);
    let snap = active.session.snapshot();
    /* No video means nothing watched. */
    if snap.duration <= 0.0 {
        return;
    }
    let t = &active.target;
    let watched_to = snap.episode_id.as_deref().unwrap_or(&t.episode.id);
    let episode = season_episode(state, &t.season_id, watched_to).unwrap_or(&t.episode);
    let (position, duration) = (snap.position as u64, snap.duration as u64);
    let entry = Tracked {
        series_id: t.series_id.clone(),
        series_title: t.series_title.clone(),
        season_id: t.season_id.clone(),
        image: episode.images.wide(WIDE_PX).map(str::to_owned),
        episode_id: episode.id.clone(),
        episode_label: format!("Episode {}", episode.label()),
        episode_title: episode.title.clone(),
        position_secs: position,
        duration_secs: duration,
        updated: 0,
    };
    state.tracker.record(entry);
}

fn season_episode<'a>(
    state: &'a AnimeState,
    season_id: &str,
    episode_id: &str,
) -> Option<&'a Episode> {
    match state.episodes.get(season_id) {
        Some(Load::Ready(list)) => list.iter().find(|e| e.id == episode_id),
        _ => None,
    }
}
