use std::collections::HashMap;

use crate::api::types::{Episode, Movie, TvShow};

#[derive(Clone)]
pub enum GridItem {
    Movie(Movie),
    Collection(CollectionGroup),
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct CollectionGroup {
    pub key: String,
    pub title: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub movies: Vec<Movie>,
}

pub fn dedupe_movies(movies: &[Movie]) -> Vec<Movie> {
    let mut by_key: HashMap<String, Movie> = HashMap::new();
    for m in movies {
        let k = movie_dedup_key(m);
        let keep = match by_key.get(&k) {
            None => true,
            Some(existing) => movie_score(m) > movie_score(existing),
        };
        if keep {
            by_key.insert(k, m.clone());
        }
    }
    let mut v: Vec<Movie> = by_key.into_values().collect();
    v.sort_by(|a, b| {
        a.display_title()
            .to_lowercase()
            .cmp(&b.display_title().to_lowercase())
    });
    v
}

pub fn dedupe_tvshows(shows: &[TvShow]) -> Vec<TvShow> {
    let mut by_key: HashMap<String, TvShow> = HashMap::new();
    for s in shows {
        let k = if s.tmdb_id != 0 {
            format!("t:{}", s.tmdb_id)
        } else {
            format!("n:{}", normalize(s.display_title()))
        };
        let keep = match by_key.get(&k) {
            None => true,
            Some(existing) => tvshow_score(s) > tvshow_score(existing),
        };
        if keep {
            by_key.insert(k, s.clone());
        }
    }
    let mut v: Vec<TvShow> = by_key.into_values().collect();
    v.sort_by(|a, b| {
        b.rating
            .partial_cmp(&a.rating)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.display_title()
                    .to_lowercase()
                    .cmp(&b.display_title().to_lowercase())
            })
    });
    v
}

pub fn dedupe_episodes(eps: &[Episode]) -> Vec<Episode> {
    let mut by_key: HashMap<(i32, i32), Episode> = HashMap::new();
    for e in eps {
        let k = (e.season, e.episode);
        let keep = match by_key.get(&k) {
            None => true,
            Some(existing) => episode_score(e) > episode_score(existing),
        };
        if keep {
            by_key.insert(k, e.clone());
        }
    }
    let mut v: Vec<Episode> = by_key.into_values().collect();
    v.sort_by_key(|e| (e.season, e.episode));
    v
}

fn movie_score(m: &Movie) -> (u8, i64, i32) {
    let complete = (m.duration > 0 && m.width > 0 && m.height > 0) as u8;
    (complete, m.size, m.width * m.height)
}

fn tvshow_score(s: &TvShow) -> (i32, i64) {
    (s.episode_count, s.size)
}

fn episode_score(e: &Episode) -> (u8, i64) {
    let complete = (e.duration > 0) as u8;
    (complete, e.size)
}

fn movie_dedup_key(m: &Movie) -> String {
    if m.tmdb_id != 0 {
        format!("t:{}", m.tmdb_id)
    } else {
        format!("n:{}::{}", normalize(m.display_title()), m.year)
    }
}

pub fn build_movie_grid(movies: &[Movie], collections: &[CollectionGroup]) -> Vec<GridItem> {
    let deduped = dedupe_movies(movies);

    let mut in_collection: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for c in collections {
        for m in &c.movies {
            in_collection.insert(m.id);
        }
    }

    let mut items: Vec<GridItem> = Vec::new();
    for c in collections {
        items.push(GridItem::Collection(c.clone()));
    }
    for m in &deduped {
        if !in_collection.contains(&m.id) {
            items.push(GridItem::Movie(m.clone()));
        }
    }
    items.sort_by(|a, b| {
        grid_rating(b)
            .partial_cmp(&grid_rating(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                grid_title(a)
                    .to_lowercase()
                    .cmp(&grid_title(b).to_lowercase())
            })
    });
    items
}

pub fn grid_title(g: &GridItem) -> &str {
    match g {
        GridItem::Movie(m) => m.display_title(),
        GridItem::Collection(c) => c.title.as_str(),
    }
}

pub fn grid_rating(g: &GridItem) -> f32 {
    match g {
        GridItem::Movie(m) => m.rating,
        GridItem::Collection(c) => c
            .movies
            .iter()
            .map(|m| m.rating)
            .fold(0.0_f32, f32::max),
    }
}

pub fn grid_card_id(g: &GridItem) -> String {
    match g {
        GridItem::Movie(m) => format!("m:{}", m.id),
        GridItem::Collection(c) => format!("c:{}", c.key),
    }
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
