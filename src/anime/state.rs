use std::collections::HashMap;
use std::future::Future;
use std::sync::mpsc::{channel, Receiver, Sender};

use tokio::runtime::Handle;

use super::api::CrApi;
use super::model::{Category, Episode, Season, Series};
use super::tracker::Tracker;
use super::watch::Active;

const ROW_LEN: u32 = 40;
const GENRE_LEN: u32 = 120;

pub enum Load<T> {
    Pending,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
}

pub struct SeriesPage {
    pub series: Series,
    pub seasons: Vec<Season>,
}

enum Msg {
    Categories(Load<Vec<Category>>),
    NewEpisodes(Load<Vec<Episode>>),
    Popular(Load<Vec<Series>>),
    Genre(String, Load<Vec<Series>>),
    Series(String, Box<Load<SeriesPage>>),
    Episodes(String, Load<Vec<Episode>>),
}

/* Crunchyroll catalogue plus local tracking. */
pub struct AnimeState {
    api: CrApi,
    rt: Handle,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub categories: Option<Load<Vec<Category>>>,
    pub new_episodes: Option<Load<Vec<Episode>>>,
    pub popular: Option<Load<Vec<Series>>>,
    pub genres: HashMap<String, Load<Vec<Series>>>,
    pub series: HashMap<String, Load<SeriesPage>>,
    pub episodes: HashMap<String, Load<Vec<Episode>>>,
    pub tracker: Tracker,
    pub watch: Option<Active>,
}

impl AnimeState {
    pub fn new(rt: Handle) -> Self {
        let (tx, rx) = channel();
        Self {
            api: CrApi::new(),
            rt,
            tx,
            rx,
            categories: None,
            new_episodes: None,
            popular: None,
            genres: HashMap::new(),
            series: HashMap::new(),
            episodes: HashMap::new(),
            tracker: Tracker::load(),
            watch: None,
        }
    }

    pub fn pump(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Categories(v) => self.categories = Some(v),
                Msg::NewEpisodes(v) => self.new_episodes = Some(v),
                Msg::Popular(v) => self.popular = Some(v),
                Msg::Genre(id, v) => {
                    self.genres.insert(id, v);
                }
                Msg::Series(id, v) => {
                    self.series.insert(id, *v);
                }
                Msg::Episodes(id, v) => {
                    self.episodes.insert(id, v);
                }
            }
        }
    }

    fn spawn<T, F>(&self, fut: F, wrap: impl FnOnce(Load<T>) -> Msg + Send + 'static)
    where
        T: Send + 'static,
        F: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let load = match fut.await {
                Ok(v) => Load::Ready(v),
                Err(e) => {
                    crate::nlog!("crunchyroll: {e}");
                    Load::Failed(e.to_string())
                }
            };
            let _ = tx.send(wrap(load));
        });
    }

    pub fn ensure_home(&mut self) {
        if self.categories.is_none() {
            self.categories = Some(Load::Pending);
            let api = self.api.clone();
            self.spawn(async move { api.categories().await }, Msg::Categories);
        }
        if self.new_episodes.is_none() {
            self.new_episodes = Some(Load::Pending);
            let api = self.api.clone();
            self.spawn(
                async move { api.new_episodes(ROW_LEN).await },
                Msg::NewEpisodes,
            );
        }
        if self.popular.is_none() {
            self.popular = Some(Load::Pending);
            let api = self.api.clone();
            self.spawn(
                async move { api.popular(None, ROW_LEN).await },
                Msg::Popular,
            );
        }
    }

    pub fn ensure_genre(&mut self, id: &str) {
        if self.genres.contains_key(id) {
            return;
        }
        self.genres.insert(id.to_string(), Load::Pending);
        let (api, key) = (self.api.clone(), id.to_string());
        let wrap_key = key.clone();
        self.spawn(
            async move { api.popular(Some(&key), GENRE_LEN).await },
            move |v| Msg::Genre(wrap_key, v),
        );
    }

    pub fn ensure_series(&mut self, id: &str) {
        if self.series.contains_key(id) {
            return;
        }
        self.series.insert(id.to_string(), Load::Pending);
        let (api, key) = (self.api.clone(), id.to_string());
        let wrap_key = key.clone();
        self.spawn(
            async move {
                let series = api.series(&key).await?;
                let seasons = api.seasons(&key).await?;
                Ok(SeriesPage { series, seasons })
            },
            move |v| Msg::Series(wrap_key, Box::new(v)),
        );
    }

    pub fn ensure_episodes(&mut self, season_id: &str) {
        if self.episodes.contains_key(season_id) {
            return;
        }
        self.episodes.insert(season_id.to_string(), Load::Pending);
        let (api, key) = (self.api.clone(), season_id.to_string());
        let wrap_key = key.clone();
        self.spawn(async move { api.episodes(&key).await }, move |v| {
            Msg::Episodes(wrap_key, v)
        });
    }
}
