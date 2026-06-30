use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::Instant;

use eframe::CreationContext;
use egui::Context;
use tokio::runtime::Handle;

use crate::api::types::{Episode, MediaDetail, Movie, SeasonInfo, ServerCollection,
    ServerCollectionDetail, Status, TvShow};
use crate::api::Api;
use crate::cache::images::ImageCache;
use crate::config;
use crate::input::gamepad::{Action, Gamepad};
use crate::library::{self, CollectionGroup, GridItem};
use crate::player::PlayerView;
use crate::ui;
use crate::watched::WatchedDb;

pub enum Screen {
    MoviesGrid,
    TvShowsGrid,
    IptvGrid,
    MovieDetail(i64),
    TvShowDetail(String),
    CollectionView(String),
    Player,
}

#[derive(Clone, Copy, PartialEq)]
pub enum TvColumn {
    Seasons,
    Episodes,
}

pub struct TvFocus {
    pub key: String,
    pub column: TvColumn,
    pub season_idx: usize,
    pub episode_idx: usize,
    pub scroll_pending: bool,
}

impl Default for TvFocus {
    fn default() -> Self {
        Self {
            key: String::new(),
            column: TvColumn::Seasons,
            season_idx: 0,
            episode_idx: 0,
            scroll_pending: true,
        }
    }
}

impl TvFocus {
    pub fn switch_show(&mut self, key: &str) {
        if self.key != key {
            self.key = key.to_string();
            self.column = TvColumn::Seasons;
            self.season_idx = 0;
            self.episode_idx = 0;
            self.scroll_pending = true;
        }
    }
}

pub enum Msg {
    Status(Result<Status, String>),
    Movies(Result<Vec<Movie>, String>),
    TvShows(Result<Vec<TvShow>, String>),
    Seasons(String, Result<Vec<SeasonInfo>, String>),
    Episodes(String, i32, Result<Vec<Episode>, String>),
    Detail(i64, Result<MediaDetail, String>),
    Collections(Result<Vec<ServerCollection>, String>),
    CollectionDetail(i64, Result<ServerCollectionDetail, String>),
    Channels(Result<Vec<crate::iptv::Channel>, String>),
    Epg(Result<crate::iptv::EpgIndex, String>),
}

pub struct App {
    pub api: Api,
    pub rt: Handle,
    pub images: ImageCache,
    pub screen: Screen,
    pub history: Vec<Screen>,

    pub status: Option<Status>,
    pub status_err: Option<String>,
    pub last_status_poll: std::time::Instant,

    pub movies: Vec<Movie>,
    pub movies_grid: Vec<GridItem>,
    pub collections: HashMap<String, CollectionGroup>,
    pub movies_loaded: bool,
    pub movies_err: Option<String>,

    pub tvshows: Vec<TvShow>,
    pub tvshows_loaded: bool,
    pub tvshows_err: Option<String>,

    pub seasons: HashMap<String, Vec<SeasonInfo>>,
    pub episodes: HashMap<(String, i32), Vec<Episode>>,
    pub selected_season: HashMap<String, i32>,
    pub details: HashMap<i64, MediaDetail>,
    pub tv_focus: TvFocus,
    pub grid_focus: usize,
    pub grid_scroll_pending: bool,

    pub tx: mpsc::Sender<Msg>,
    pub rx: mpsc::Receiver<Msg>,

    pub player: PlayerView,
    pub watched: WatchedDb,
    pub gamepad: Gamepad,
    pub nav_actions: Vec<Action>,
    pub fullscreen_sent: bool,

    pub known_movie_ids: HashSet<i64>,
    pub known_tvshow_keys: HashSet<String>,
    pub movies_seen_once: bool,
    pub tvshows_seen_once: bool,
    pub new_card_at: HashMap<String, Instant>,
    pub last_library_poll: Instant,
    pub last_disk_keepalive: Instant,
    pub toasts: Vec<crate::ui::toast::Toast>,

    pub rating_anim_key: Option<String>,
    pub rating_anim_start: Instant,

    pub server_collections: Vec<ServerCollection>,
    pub server_collection_members: HashMap<i64, Vec<i64>>,
    pub last_collections_poll: Instant,

    pub channels: Vec<crate::iptv::Channel>,
    pub channels_loaded: bool,
    pub channels_err: Option<String>,

    pub epg: crate::iptv::EpgIndex,
    pub epg_loaded: bool,
    pub last_epg_poll: Instant,
    pub iptv_visible: Vec<usize>,

    pub wl_display_ptr: Option<*mut std::ffi::c_void>,
    pub wl_parent_surface_ptr: Option<*mut std::ffi::c_void>,
    pub subsurface: Option<std::sync::Arc<crate::player::wl_subsurface::SubsurfaceVideo>>,
    pub color_mgr: Option<std::sync::Arc<crate::player::wl_color::ColorMgr>>,
    pub hdr_applied: bool,
    /* Markeres når build_image_description har feilet for nåværende stream
     * (manglende parametric-feature, compositor-Failed-event, eller timeout).
     * Hindrer per-frame retry som ellers blokkerer main-tråd 200 ms hver
     * gang. Nullstilles ved ny loadfile via clear_hdr_state(). */
    pub hdr_failed: bool,
    pub hdr_cm_surface: Option<
        wayland_protocols::wp::color_management::v1::client::wp_color_management_surface_v1::WpColorManagementSurfaceV1,
    >,
}

impl App {
    pub fn new(cc: &CreationContext<'_>, rt: Handle) -> Self {
        ui::theme::install(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let api = Api::new();
        let images = ImageCache::new(api.clone(), rt.clone());
        let (tx, rx) = mpsc::channel();
        let gl = cc.gl.clone().expect("glow GL context");
        let player = PlayerView::new(gl, rt.clone());
        let app = Self {
            api,
            rt,
            images,
            screen: Screen::MoviesGrid,
            history: Vec::new(),
            status: None,
            status_err: None,
            last_status_poll: std::time::Instant::now()
                - std::time::Duration::from_secs(config::STATUS_POLL_SECS + 1),
            movies: Vec::new(),
            movies_grid: Vec::new(),
            collections: HashMap::new(),
            movies_loaded: false,
            movies_err: None,
            tvshows: Vec::new(),
            tvshows_loaded: false,
            tvshows_err: None,
            seasons: HashMap::new(),
            episodes: HashMap::new(),
            selected_season: HashMap::new(),
            details: HashMap::new(),
            tv_focus: TvFocus::default(),
            grid_focus: 0,
            grid_scroll_pending: false,
            tx,
            rx,
            player,
            watched: WatchedDb::load(),
            gamepad: Gamepad::new(),
            nav_actions: Vec::new(),
            fullscreen_sent: false,
            known_movie_ids: HashSet::new(),
            known_tvshow_keys: HashSet::new(),
            movies_seen_once: false,
            tvshows_seen_once: false,
            new_card_at: HashMap::new(),
            last_library_poll: Instant::now()
                - std::time::Duration::from_secs(config::LIBRARY_POLL_SECS + 1),
            last_disk_keepalive: Instant::now(),
            toasts: Vec::new(),
            rating_anim_key: None,
            rating_anim_start: Instant::now(),
            server_collections: Vec::new(),
            server_collection_members: HashMap::new(),
            last_collections_poll: Instant::now()
                - std::time::Duration::from_secs(config::LIBRARY_POLL_SECS + 1),
            channels: Vec::new(),
            channels_loaded: false,
            channels_err: None,
            epg: crate::iptv::EpgIndex::default(),
            epg_loaded: false,
            last_epg_poll: Instant::now()
                - std::time::Duration::from_secs(config::EPG_REFRESH_SECS + 1),
            iptv_visible: Vec::new(),
            wl_display_ptr: None,
            wl_parent_surface_ptr: None,
            subsurface: None,
            color_mgr: None,
            hdr_applied: false,
            hdr_failed: false,
            hdr_cm_surface: None,
        };
        app.fetch_status();
        app.fetch_movies();
        app.fetch_tvshows();
        app.fetch_collections();
        app.fetch_channels();
        app.fetch_epg();
        app
    }

    pub fn navigate(&mut self, screen: Screen) {
        let entering_player = matches!(screen, Screen::Player);
        let prev = std::mem::replace(&mut self.screen, screen);
        self.history.push(prev);
        self.nav_actions.clear();
        /* Ny session — nullstill HDR-failure-flagget så neste forhandling
         * får ny sjanse. Stream kan ha annen color-profile enn forrige. */
        if entering_player {
            self.clear_hdr_state();
        }
    }

    pub fn switch_lib(&mut self, screen: Screen) {
        self.screen = screen;
        self.grid_focus = 0;
        self.grid_scroll_pending = true;
        self.nav_actions.clear();
    }

    pub fn go_back(&mut self) {
        if let Some(s) = self.history.pop() {
            let leaving = std::mem::replace(&mut self.screen, s);
            self.restore_grid_focus(&leaving);
        }
        self.nav_actions.clear()
    }

    fn restore_grid_focus(&mut self, leaving: &Screen) {
        match leaving {
            Screen::MovieDetail(id) => match &self.screen {
                Screen::MoviesGrid => {
                    if let Some(idx) = self.movies_grid.iter().position(|it| match it {
                        GridItem::Movie(m) => m.id == *id,
                        GridItem::Collection(c) => c.movies.iter().any(|m| m.id == *id),
                    }) {
                        self.grid_focus = idx;
                        self.grid_scroll_pending = true;
                    }
                }
                Screen::CollectionView(key) => {
                    if let Some(coll) = self.collections.get(key) {
                        if let Some(idx) = coll.movies.iter().position(|m| m.id == *id) {
                            self.grid_focus = idx;
                            self.grid_scroll_pending = true;
                        }
                    }
                }
                _ => {}
            },
            Screen::TvShowDetail(key) => {
                if matches!(self.screen, Screen::TvShowsGrid) {
                    if let Some(idx) = self.tvshows.iter().position(|s| s.key() == *key) {
                        self.grid_focus = idx;
                        self.grid_scroll_pending = true;
                    }
                }
            }
            Screen::CollectionView(key) => {
                if matches!(self.screen, Screen::MoviesGrid) {
                    if let Some(idx) =
                        self.movies_grid.iter().position(|it| matches!(it, GridItem::Collection(c) if c.key == *key))
                    {
                        self.grid_focus = idx;
                        self.grid_scroll_pending = true;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn fetch_status(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = api
                .get_json::<Status>("/api/status")
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Status(r));
        });
    }

    pub fn fetch_movies(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = api
                .get_json::<Vec<Movie>>("/api/movies")
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Movies(r));
        });
    }

    pub fn fetch_tvshows(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = api
                .get_json::<Vec<TvShow>>("/api/tvshows")
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::TvShows(r));
        });
    }

    pub fn fetch_seasons(&self, key: &str) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        let key = key.to_string();
        self.rt.spawn(async move {
            let path = format!("/api/show/{}/seasons", urlencoding::encode(&key));
            let r = api
                .get_json::<Vec<SeasonInfo>>(&path)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Seasons(key, r));
        });
    }

    pub fn fetch_episodes(&self, key: &str, season: i32) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        let key = key.to_string();
        self.rt.spawn(async move {
            let path = format!(
                "/api/show/{}/episodes/{}",
                urlencoding::encode(&key),
                season
            );
            let r = api
                .get_json::<Vec<Episode>>(&path)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Episodes(key, season, r));
        });
    }

    pub fn fetch_collections(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = api
                .get_json::<Vec<ServerCollection>>("/api/collections")
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Collections(r));
        });
    }

    pub fn fetch_channels(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = crate::iptv::fetch_channels(
                &api,
                crate::config::IPTV_BASE,
                crate::config::IPTV_USER,
                crate::config::IPTV_PASS,
            )
            .await
            .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Channels(r));
        });
    }

    pub fn fetch_epg(&self) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = crate::iptv::epg::fetch_epg(
                &api,
                crate::config::IPTV_BASE,
                crate::config::IPTV_USER,
                crate::config::IPTV_PASS,
            )
            .await
            .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Epg(r));
        });
    }

    pub fn recompute_iptv_visible(&mut self) {
        let epg_ready = !self.epg.is_empty();
        self.iptv_visible = self
            .channels
            .iter()
            .enumerate()
            .filter_map(|(i, ch)| {
                if !epg_ready {
                    return Some(i);
                }
                match ch.epg_id.as_deref() {
                    Some(id) if self.epg.is_active(id) => Some(i),
                    _ => None,
                }
            })
            .collect();
        self.prewarm_iptv_logos();
    }

    fn prewarm_iptv_logos(&self) {
        for &idx in &self.iptv_visible {
            if let Some(ch) = self.channels.get(idx) {
                if let Some(logo) = ch.logo.as_deref() {
                    let _ = self.images.get(logo);
                }
            }
        }
    }

    pub fn fetch_collection_detail(&self, id: i64) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = api
                .get_json::<ServerCollectionDetail>(&format!("/api/collections/{}", id))
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::CollectionDetail(id, r));
        });
    }

    pub fn fetch_detail(&self, id: i64) {
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let r = api
                .get_json::<MediaDetail>(&format!("/api/media/{}", id))
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Detail(id, r));
        });
    }

    fn drain_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Status(Ok(s)) => {
                    self.status = Some(s);
                    self.status_err = None;
                }
                Msg::Status(Err(e)) => {
                    self.status = None;
                    self.status_err = Some(e);
                }
                Msg::Movies(Ok(v)) => {
                    let now_ids: HashSet<i64> = v.iter().map(|m| m.id).collect();
                    if self.movies_seen_once {
                        let now = Instant::now();
                        let new_movies: Vec<(String, i32)> = library::dedupe_movies(&v)
                            .into_iter()
                            .filter(|m| !self.known_movie_ids.contains(&m.id))
                            .map(|m| {
                                self.new_card_at.insert(format!("m:{}", m.id), now);
                                (m.display_title().to_string(), m.year)
                            })
                            .collect();
                        for (title, year) in new_movies {
                            crate::ui::toast::push_movie(self, &title, year);
                        }
                    }
                    self.known_movie_ids = now_ids;
                    self.movies_seen_once = true;
                    self.movies = library::dedupe_movies(&v);
                    self.rebuild_movie_grid();
                    self.movies_loaded = true;
                    self.movies_err = None;
                }
                Msg::Movies(Err(e)) => {
                    self.movies_loaded = true;
                    self.movies_err = Some(e);
                }
                Msg::TvShows(Ok(v)) => {
                    let deduped = library::dedupe_tvshows(&v);
                    let now_keys: HashSet<String> =
                        deduped.iter().map(|s| s.key().to_string()).collect();
                    if self.tvshows_seen_once {
                        let now = Instant::now();
                        let mut new_shows: Vec<String> = Vec::new();
                        for s in &deduped {
                            let k = s.key().to_string();
                            if !self.known_tvshow_keys.contains(&k) {
                                self.new_card_at.insert(format!("t:{}", k), now);
                                new_shows.push(s.display_title().to_string());
                            }
                        }
                        for title in new_shows {
                            crate::ui::toast::push_tvshow(self, &title);
                        }
                    }
                    self.known_tvshow_keys = now_keys;
                    self.tvshows_seen_once = true;
                    self.tvshows = deduped;
                    self.tvshows_loaded = true;
                    self.tvshows_err = None;
                }
                Msg::TvShows(Err(e)) => {
                    self.tvshows_loaded = true;
                    self.tvshows_err = Some(e);
                }
                Msg::Seasons(k, Ok(v)) => {
                    if !v.is_empty() && !self.selected_season.contains_key(&k) {
                        self.selected_season.insert(k.clone(), v[0].season);
                        self.fetch_episodes(&k, v[0].season);
                    }
                    self.seasons.insert(k, v);
                }
                Msg::Seasons(_, Err(_)) => {}
                Msg::Episodes(k, s, Ok(v)) => {
                    let deduped = library::dedupe_episodes(&v);
                    self.episodes.insert((k, s), deduped);
                }
                Msg::Episodes(_, _, Err(_)) => {}
                Msg::Detail(id, Ok(d)) => {
                    self.details.insert(id, d);
                }
                Msg::Detail(_, Err(_)) => {}
                Msg::Collections(Ok(list)) => {
                    self.server_collections = list;
                    for c in &self.server_collections {
                        if !self.server_collection_members.contains_key(&c.id) {
                            self.fetch_collection_detail(c.id);
                        }
                    }
                    self.rebuild_movie_grid();
                }
                Msg::Collections(Err(_)) => {}
                Msg::CollectionDetail(id, Ok(d)) => {
                    let ids: Vec<i64> = d.items.iter().map(|m| m.id).collect();
                    self.server_collection_members.insert(id, ids);
                    self.rebuild_movie_grid();
                }
                Msg::CollectionDetail(_, Err(_)) => {}
                Msg::Channels(Ok(v)) => {
                    self.channels = v;
                    self.channels_loaded = true;
                    self.channels_err = None;
                    self.recompute_iptv_visible();
                }
                Msg::Channels(Err(e)) => {
                    self.channels_loaded = true;
                    self.channels_err = Some(e);
                }
                Msg::Epg(Ok(idx)) => {
                    self.epg = idx;
                    self.epg_loaded = true;
                    self.recompute_iptv_visible();
                }
                Msg::Epg(Err(e)) => {
                    crate::nlog!("epg fetch failed: {e}");
                    self.epg_loaded = true;
                }
            }
        }
    }

    fn rebuild_movie_grid(&mut self) {
        let mut groups: Vec<library::CollectionGroup> = Vec::new();
        let by_id: HashMap<i64, Movie> =
            self.movies.iter().map(|m| (m.id, m.clone())).collect();
        for c in &self.server_collections {
            let member_ids = match self.server_collection_members.get(&c.id) {
                Some(v) => v,
                None => continue,
            };
            let mut movies: Vec<Movie> = member_ids
                .iter()
                .filter_map(|id| by_id.get(id).cloned())
                .collect();
            if movies.is_empty() {
                continue;
            }
            movies.sort_by_key(|m| (m.year, m.display_title().to_lowercase()));
            let key = format!("srv:{}", c.id);
            let poster = c
                .poster
                .clone()
                .or_else(|| movies.iter().find_map(|m| m.poster.clone()));
            let backdrop = c
                .backdrop
                .clone()
                .or_else(|| movies.iter().find_map(|m| m.backdrop.clone()));
            groups.push(library::CollectionGroup {
                key,
                title: c.name.clone(),
                poster,
                backdrop,
                movies,
            });
        }
        self.movies_grid = library::build_movie_grid(&self.movies, &groups);
        self.collections.clear();
        for item in &self.movies_grid {
            if let library::GridItem::Collection(c) = item {
                self.collections.insert(c.key.clone(), c.clone());
            }
        }
    }

    fn update_wl_handles(&mut self, frame: &eframe::Frame) {
        use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
        if self.wl_display_ptr.is_none() {
            if let Ok(dh) = frame.display_handle() {
                if let RawDisplayHandle::Wayland(wd) = dh.as_raw() {
                    self.wl_display_ptr = Some(wd.display.as_ptr());
                }
            }
        }
        if self.wl_parent_surface_ptr.is_none() {
            if let Ok(wh) = frame.window_handle() {
                if let RawWindowHandle::Wayland(ww) = wh.as_raw() {
                    self.wl_parent_surface_ptr = Some(ww.surface.as_ptr());
                }
            }
        }
    }

    fn ensure_subsurface_for_player(&mut self, ctx: &Context) {
        if !matches!(self.screen, Screen::Player) {
            return;
        }
        let (Some(dp), Some(sp)) = (self.wl_display_ptr, self.wl_parent_surface_ptr) else {
            return;
        };
        let rect = ctx.screen_rect();
        let ppp = ctx.pixels_per_point();
        let w = (rect.width() * ppp).round() as i32;
        let h = (rect.height() * ppp).round() as i32;

        /* Fersk subsurface per play: start()/start_url() har joinet gammel
         * mpv og bedt om ny surface. Slipp begge Arc-klonene → SubsurfaceVideo
         * Drop frigjør EGL/render-context rent før neste bygges. Uten dette
         * gjenbrukes surface på tvers av plays → akkumulerte compositor-
         * ressurser (freeze) og CUDA-GL-teardown-race (crash). */
        if self.player.wants_fresh_surface {
            self.player.wants_fresh_surface = false;
            /* HDR-CM-surface er bundet til den GAMLE subsurfacen. Unbind
             * (unset+commit, riktig rekkefølge) FØR vi dropper den, ellers
             * river vi ned en surface med HDR fremdeles bundet →
             * VK_ERROR_DEVICE_LOST/freeze. Auto-next bytter stream uten å
             * forlate Player, så tick_hdr_state sin !in_player-teardown
             * fyrer aldri her — vi må gjøre det eksplisitt (samme som
             * stop_and_return gjør på vei ut). clear_hdr_state lar neste
             * stream reforhandle HDR fra scratch. */
            self.teardown_hdr_surface();
            self.clear_hdr_state();
            self.player.clear_subsurface();
            self.subsurface = None;
        }

        if self.subsurface.is_none() {
            match unsafe { crate::player::wl_subsurface::SubsurfaceVideo::new(dp, sp, w.max(1), h.max(1)) } {
                Ok(sv) => {
                    sv.set_position(0, 0);
                    let arc = std::sync::Arc::new(sv);
                    self.subsurface = Some(arc.clone());
                    self.player.set_subsurface(arc);
                    crate::nlog!("subsurface created {w}x{h}");
                }
                Err(e) => {
                    crate::nlog!("subsurface init failed: {e}");
                    return;
                }
            }
        }

        if self.color_mgr.is_none() {
            let backend = unsafe {
                wayland_backend::client::Backend::from_foreign_display(dp.cast())
            };
            let conn = wayland_client::Connection::from_backend(backend);
            match crate::player::wl_color::ColorMgr::new(conn) {
                Ok(m) => {
                    let has_pq = m.supports_pq();
                    crate::nlog!(
                        "wp_color_manager_v1: {}",
                        if m.manager().is_some() {
                            if has_pq {
                                "bound (PQ+BT2020 supported)"
                            } else {
                                "bound (PQ unsupported)"
                            }
                        } else {
                            "not advertised"
                        }
                    );
                    self.color_mgr = Some(std::sync::Arc::new(m));
                }
                Err(e) => crate::nlog!("color_mgr init: {e}"),
            }
        }

        if let Some(sub) = &self.subsurface {
            sub.resize(w.max(1), h.max(1));
            sub.pump();
        }
        if let Some(cm) = &self.color_mgr {
            cm.pump();
        }
    }

    /* Per-frame HDR state machine. Polls mpv's HDR meta (updated by the
     * render thread every ~500ms) and toggles PQ passthrough on the
     * wp_color_manager_v1 surface so HDR content reaches the display as
     * HDR instead of being tone-mapped to SDR by mpv.
     *
     * Transition order is deliberate:
     *  setup    → attach+commit FIRST, then flip mpv to PQ
     *  teardown → unset+commit FIRST, then flip mpv back to SDR
     * Both orders ensure the compositor's "what color space is this
     * surface" matches mpv's "what am I emitting" within one frame;
     * the alternative ordering blows out one frame at HDR brightness. */
    fn tick_hdr_state(&mut self) {
        let in_player = matches!(self.screen, Screen::Player);

        if !in_player {
            if self.hdr_applied {
                if let Some(mpv) = self.player.mpv.clone() {
                    self.teardown_hdr_surface();
                    mpv.set_passthrough_pq(false);
                } else {
                    self.teardown_hdr_surface();
                }
            }
            return;
        }

        let Some(mpv) = self.player.mpv.clone() else {
            if self.hdr_applied {
                self.teardown_hdr_surface();
            }
            /* Ny session forventes — nullstill failure-flagget så neste
             * loadfile får ny sjanse til å forhandle HDR. */
            self.hdr_failed = false;
            return;
        };

        let (Some(cm), Some(sub)) =
            (self.color_mgr.clone(), self.subsurface.clone())
        else {
            return;
        };

        if !cm.supports_pq() {
            if self.hdr_applied {
                self.teardown_hdr_surface();
                mpv.set_passthrough_pq(false);
            }
            return;
        }

        let meta = mpv.current_hdr();
        let want_hdr = meta.as_ref().map(|m| m.is_hdr()).unwrap_or(false);

        if want_hdr && !self.hdr_applied {
            /* hdr_failed = ikke prøv på nytt før neste stream. Hindrer
             * 200 ms blokkering per frame ved persistent feil. */
            if self.hdr_failed {
                return;
            }
            /* Vent til decoder + render-kjeden har produsert flere frames
             * før vi flipper color-mode. NVDEC for 4K HEVC HDR bruker
             * 200-500 ms på å initialisere; setter vi target-prim/trc
             * midt i den prosessen kan begge sider race og krasje. */
            const HDR_DEFER_FRAMES: i64 = 8;
            if mpv.frames_decoded() < HDR_DEFER_FRAMES {
                return;
            }
            let Some(meta) = meta else { return };

            /* Pause mpv før transition. Render-tråden slutter å pumpe ut
             * frames mens vi konfigurerer surface + property — eliminerer
             * 1-frame race der SDR-buffer henger igjen på PQ-tagget
             * surface (eller motsatt). Resume etter mpv-property er
             * applied. */
            let was_paused = mpv.paused();
            if !was_paused {
                mpv.set_pause(true);
            }

            let Some(desc) = cm.build_image_description(&meta) else {
                crate::nlog!("HDR: build_image_description failed — fallback til SDR for denne strømmen");
                self.hdr_failed = true;
                if !was_paused {
                    mpv.set_pause(false);
                }
                return;
            };

            /* Hold commit_lock gjennom attach + commit. Render-tråden
             * blokkeres uansett (mpv paused), men låsen hindrer at en
             * sen swap-callback slipper igjennom. */
            let cm_surf_opt = sub.with_commit_lock(|| {
                let cm_surf = cm.attach_to_surface(sub.child_wl_surface(), &desc)?;
                sub.child_surface.commit();
                let _ = sub.conn.flush();
                Some(cm_surf)
            });

            let Some(cm_surf) = cm_surf_opt else {
                crate::nlog!("HDR: attach_to_surface failed — fallback til SDR");
                desc.destroy();
                self.hdr_failed = true;
                if !was_paused {
                    mpv.set_pause(false);
                }
                return;
            };

            /* Vent kort på compositor før vi flipper mpv-output, så
             * surface-tag er ekte-applied når første PQ-frame committer.
             * En enkelt roundtrip på cm-køen er nok — ingen tight loop. */
            cm.flush_and_roundtrip();
            desc.destroy();

            mpv.set_passthrough_pq(true);
            self.hdr_cm_surface = Some(cm_surf);
            self.hdr_applied = true;
            crate::nlog!("HDR: PQ/BT.2020 passthrough engaged");

            if !was_paused {
                mpv.set_pause(false);
            }
        } else if !want_hdr && self.hdr_applied {
            let was_paused = mpv.paused();
            if !was_paused {
                mpv.set_pause(true);
            }
            self.teardown_hdr_surface();
            mpv.set_passthrough_pq(false);
            crate::nlog!("HDR: passthrough disengaged (SDR content)");
            if !was_paused {
                mpv.set_pause(false);
            }
        }
    }

    /* Kalles ved ny loadfile / stream-start så HDR-failure-flagget
     * resettes og ny HDR-forhandling kan prøves. */
    pub fn clear_hdr_state(&mut self) {
        self.hdr_failed = false;
    }

    /* Detach color description from subsurface and let compositor settle
     * before we destroy the surface. Without this, dropping the subsurface
     * with HDR still bound forces the compositor to do HDR-exit + redraw
     * in the same frame — under wlroots 0.20 + Vulkan this can trigger
     * VK_ERROR_DEVICE_LOST. */
    pub fn teardown_hdr_surface(&mut self) {
        if let Some(cm_surf) = self.hdr_cm_surface.take() {
            /* Unset + commit må være atomic mot render-tråd swap, og må
             * skje før destroy. Uten kommit får compositor aldri vite at
             * vi har droppet CM-state — under wlroots 0.20+Vulkan kan
             * destroy-mens-bundet trigge VK_ERROR_DEVICE_LOST. */
            if let Some(sub) = &self.subsurface {
                sub.with_commit_lock(|| {
                    cm_surf.unset_image_description();
                    sub.child_surface.commit();
                    let _ = sub.conn.flush();
                });
            }
            cm_surf.destroy();
            if let Some(cm) = &self.color_mgr {
                cm.flush_and_roundtrip();
            }
        }
        self.hdr_applied = false;
    }

    /* Hold media-disken på server våken. Mens en video faktisk spiller
     * holder selve streamen disken våken, så vi varmer kun når vi er idle
     * i menyene ELLER står på pause midt i avspilling (lang pause kan ellers
     * la HDD-en spinne ned → frys ved resume). Varmer den filen som spilles
     * når vi er i Player, ellers første film i biblioteket. */
    fn maybe_disk_keepalive(&mut self) {
        if self.last_disk_keepalive.elapsed().as_secs() < config::DISK_KEEPALIVE_SECS {
            return;
        }
        let in_player = matches!(self.screen, Screen::Player);
        let paused = self
            .player
            .mpv
            .as_ref()
            .map(|m| m.paused())
            .unwrap_or(false);
        if in_player && !paused {
            return;
        }
        let id = if in_player {
            self.player.media_id
        } else {
            self.movies.first().map(|m| m.id)
        };
        let Some(id) = id else { return };
        self.last_disk_keepalive = Instant::now();
        let api = self.api.clone();
        self.rt.spawn(async move {
            let _ = api.warm_disk(id).await;
        });
    }

    fn maybe_repoll_status(&mut self) {
        if matches!(self.screen, Screen::Player) {
            return;
        }
        if self.last_status_poll.elapsed().as_secs() >= config::STATUS_POLL_SECS {
            self.last_status_poll = Instant::now();
            self.fetch_status();
        }
        if self.last_library_poll.elapsed().as_secs() >= config::LIBRARY_POLL_SECS {
            self.last_library_poll = Instant::now();
            self.fetch_movies();
            self.fetch_tvshows();
        }
        if self.last_collections_poll.elapsed().as_secs() >= config::LIBRARY_POLL_SECS {
            self.last_collections_poll = Instant::now();
            self.fetch_collections();
        }
        if self.last_epg_poll.elapsed().as_secs() >= config::EPG_REFRESH_SECS {
            self.last_epg_poll = Instant::now();
            self.fetch_epg();
        }
        self.new_card_at
            .retain(|_, t| t.elapsed().as_millis() < 800);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, frame: &mut eframe::Frame) {
        apply_global_scale(ctx);

        self.update_wl_handles(frame);
        self.ensure_subsurface_for_player(ctx);
        self.tick_hdr_state();

        self.drain_messages();
        self.maybe_repoll_status();
        self.maybe_disk_keepalive();
        self.images.pump(ctx);
        self.gamepad.attach_ctx(ctx);
        self.gamepad.poll();
        crate::input::gamepad::route(self);

        ctx.set_cursor_icon(egui::CursorIcon::None);
        if !self.fullscreen_sent {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.fullscreen_sent = true;
        }

        ui::root::draw(self, ctx);

        if matches!(self.screen, Screen::Player) {
            if self.player.controls_visible() || self.player.popup.is_some() {
                ctx.request_repaint_after(std::time::Duration::from_millis(33));
            }
        } else {
            ctx.request_repaint();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        if matches!(self.screen, Screen::Player) && self.subsurface.is_some() {
            [0.0, 0.0, 0.0, 0.0]
        } else if matches!(self.screen, Screen::Player) {
            [0.0, 0.0, 0.0, 1.0]
        } else {
            let c = ui::theme::BG;
            let f = |b: u8| b as f32 / 255.0;
            [f(c.r()), f(c.g()), f(c.b()), 1.0]
        }
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.teardown_hdr_surface();
        self.player.shutdown();
        self.subsurface = None;
        self.watched.flush();
    }
}

const DESIGN_HEIGHT: f32 = 1080.0;

fn apply_global_scale(ctx: &Context) {
    let cur_ppp = ctx.pixels_per_point();
    let physical_h = ctx.screen_rect().height() * cur_ppp;
    if physical_h < 1.0 {
        return;
    }
    let target = (physical_h / DESIGN_HEIGHT).clamp(0.5, 4.0);
    if (cur_ppp - target).abs() > 0.01 {
        ctx.set_pixels_per_point(target);
    }
}
