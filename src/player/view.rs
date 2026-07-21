use std::sync::Arc;

use parking_lot::Mutex;
use tokio::runtime::Handle;

use crate::api::Api;
use crate::player::bandwidth::{BandwidthProbe, BufferPolicy, ProbeProgress, ProbeResult};
use crate::player::mpv::{MpvPlayer, Track, TrackKind};
use crate::player::wl_subsurface::SubsurfaceVideo;

#[derive(Clone, Debug)]
pub enum Origin {
    Movie(i64),
    Episode {
        show_key: String,
        season: i32,
        episode_idx: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Stop,
    SkipBack,
    PlayPause,
    SkipFwd,
    Subs,
    Audio,
}

pub const CONTROLS: [Control; 6] = [
    Control::Stop,
    Control::SkipBack,
    Control::PlayPause,
    Control::SkipFwd,
    Control::Subs,
    Control::Audio,
];

pub struct Popup {
    pub kind: TrackKind,
    pub tracks: Vec<Track>,
    pub idx: usize,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Idle,
    Probing,
    Preloading,
    Playing,
    Error,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SeekDir {
    Back,
    Forward,
}

pub struct SeekHold {
    pub dir: SeekDir,
    pub started_at: std::time::Instant,
    pub last_tick: std::time::Instant,
}

pub struct PlayerView {
    #[allow(dead_code)]
    pub gl: Arc<glow::Context>,
    pub rt: Handle,

    pub mpv: Option<Arc<MpvPlayer>>,
    pub subsurface: Option<Arc<SubsurfaceVideo>>,
    pub phase: Arc<Mutex<Phase>>,
    pub error: Arc<Mutex<Option<String>>>,

    pub probe: BandwidthProbe,
    pub probe_result: Option<ProbeResult>,
    pub started_at: std::time::Instant,
    pub preload_started_at: Option<std::time::Instant>,

    pub title: String,
    pub media_id: Option<i64>,
    pub origin: Option<Origin>,
    pub bitrate: i64,
    pub duration: i32,
    /* Live IPTV (start_url) vs VOD (start). Styrer mpv cache-pause: VOD
     * rebuffrer ved underløp, live holder cache-pause av for å ikke krølle
     * stall-deteksjon på 1x-realtime TS. */
    pub is_live: bool,

    pub auth_header: String,
    pub stream_url: String,

    pub show_controls_until: std::time::Instant,
    /* Forrige frames synlighet av kontroll-baren. Brukes til å detektere
     * skjult→synlig-overgang og resette fokus til play/pause (index 2)
     * ved hver visning. Uten dette ville fokus stå igjen på siste
     * brukte knapp og forvirre brukeren ved neste oppdukking. */
    pub controls_were_visible: bool,

    pub control_focus: usize,
    pub popup: Option<Popup>,

    /* Aktiv seek-hold fra skulderknapp. Some(..) mens knappen holdes inne.
     * tick_seek_hold() i poll() utsteder gjentatte keyframe-seeks med
     * akselererende step så lenge holdet varer. */
    pub seek_hold: Option<SeekHold>,

    /* Resume-posisjon i sekunder. Settes av start(), brukes i init_mpv
     * for å sette "start" property før loadfile. */
    pub resume_pos: f64,

    /* Auto-next state: Some(t0) når <=50 s gjenstår og countdown ruller.
     * Når t0.elapsed() >= 10s skal neste episode startes. */
    pub auto_next_started_at: Option<std::time::Instant>,
    /* Sist gang vi skrev watched-entry til disk. Throttler flush. */
    pub last_watched_save: std::time::Instant,

    /* Settes av start()/start_url() for å be App rive ned og bygge
     * subsurface på nytt FØR neste mpv. Hver play får dermed en ren
     * EGL/render-context istf gjenbrukt surface — gjenbruk akkumulerte
     * compositor-ressurser og lot CUDA-GL-teardown race mot ny context
     * (freeze på video 2/3, crash ved oppstart). Reconciles i
     * App::ensure_subsurface_for_player. */
    pub wants_fresh_surface: bool,

    /* Loop-vakt for watchdog-utløst auto-restart (frossen pipeline). Teller
     * restarter i et vindu; over taket gir vi opp med feilmelding istf å
     * restarte i evig løkke på en kilde som alltid fryser. */
    pub restart_count: u32,
    pub last_restart: std::time::Instant,
}

impl PlayerView {
    pub fn new(gl: Arc<glow::Context>, rt: Handle) -> Self {
        Self {
            gl,
            rt,
            mpv: None,
            subsurface: None,
            phase: Arc::new(Mutex::new(Phase::Idle)),
            error: Arc::new(Mutex::new(None)),
            probe: BandwidthProbe::new(),
            probe_result: None,
            started_at: std::time::Instant::now(),
            preload_started_at: None,
            title: String::new(),
            media_id: None,
            origin: None,
            bitrate: 0,
            duration: 0,
            is_live: false,
            auth_header: String::new(),
            stream_url: String::new(),
            show_controls_until: std::time::Instant::now()
                + std::time::Duration::from_secs(2),
            controls_were_visible: false,
            control_focus: 2,
            popup: None,
            seek_hold: None,
            resume_pos: 0.0,
            auto_next_started_at: None,
            last_watched_save: std::time::Instant::now(),
            wants_fresh_surface: false,
            restart_count: 0,
            last_restart: std::time::Instant::now()
                - std::time::Duration::from_secs(600),
        }
    }

    /* Watchdog-utløst full restart: river gammel spiller og bygger fersk
     * (ny subsurface + mpv), resume ved pos. Loop-vakt: > 3 restarter på
     * 120 s = kilden fryser konsekvent → gi opp med feil istf evig løkke. */
    pub fn restart_current(&mut self, api: &Api, resume_pos: f64) {
        if self.last_restart.elapsed() < std::time::Duration::from_secs(120) {
            self.restart_count += 1;
        } else {
            self.restart_count = 1;
        }
        self.last_restart = std::time::Instant::now();
        if self.restart_count > 3 {
            crate::nlog!("auto-restart: > 3 på 120 s — gir opp");
            self.shutdown();
            *self.error.lock() = Some("Avspilling frøs gjentatte ganger".into());
            *self.phase.lock() = Phase::Error;
            return;
        }
        crate::nlog!(
            "auto-restart #{} (frossen pipeline): resume {resume_pos:.1}s",
            self.restart_count
        );
        if self.is_live {
            let url = self.stream_url.clone();
            let title = self.title.clone();
            self.start_url(&url, &title);
        } else if let (Some(id), Some(origin)) = (self.media_id, self.origin.clone()) {
            let title = self.title.clone();
            let bitrate = self.bitrate;
            let duration = self.duration;
            self.start(api, id, &title, bitrate, duration, origin, resume_pos);
        }
    }

    /* Idx 2 i CONTROLS = PlayPause. Brukes som default-fokus både ved init
     * og ved hver synlighet-transition så brukeren alltid kan trykke
     * Confirm uten å først navigere. */
    pub const PLAY_PAUSE_IDX: usize = 2;

    /* Detekter hidden→visible-overgang og resett control_focus til
     * PlayPause. Kalles fra poll() før draw. nudge_controls fra
     * focus_left/right kalles ETTER fokusendring, så denne metoden
     * trigger ikke når brukeren navigerer mens baren allerede er synlig. */
    pub fn sync_controls_visibility(&mut self) {
        let visible = self.controls_visible();
        if visible && !self.controls_were_visible {
            self.control_focus = Self::PLAY_PAUSE_IDX;
        }
        self.controls_were_visible = visible;
    }

    pub fn focused_control(&self) -> Control {
        CONTROLS[self.control_focus.min(CONTROLS.len() - 1)]
    }

    pub fn focus_left(&mut self) {
        if self.control_focus > 0 {
            self.control_focus -= 1;
        }
        self.nudge_controls();
    }

    pub fn focus_right(&mut self) {
        if self.control_focus + 1 < CONTROLS.len() {
            self.control_focus += 1;
        }
        self.nudge_controls();
    }

    pub fn open_popup(&mut self, kind: TrackKind) {
        let Some(mpv) = &self.mpv else { return };
        let tracks = mpv.tracks(kind);
        let current = match kind {
            TrackKind::Sub => mpv.current_sid(),
            TrackKind::Audio => mpv.current_aid(),
        };
        let idx = tracks
            .iter()
            .position(|t| Some(t.id) == current)
            .unwrap_or(0);
        self.popup = Some(Popup { kind, tracks, idx });
        self.nudge_controls();
    }

    pub fn close_popup(&mut self) {
        self.popup = None;
    }

    pub fn popup_up(&mut self) {
        if let Some(p) = &mut self.popup {
            if p.idx > 0 {
                p.idx -= 1;
            }
        }
        self.nudge_controls();
    }

    pub fn popup_down(&mut self) {
        if let Some(p) = &mut self.popup {
            if p.idx + 1 < p.tracks.len() {
                p.idx += 1;
            }
        }
        self.nudge_controls();
    }

    pub fn popup_confirm(&mut self) {
        let Some(p) = self.popup.take() else { return };
        let Some(t) = p.tracks.get(p.idx).cloned() else { return };
        let Some(mpv) = &self.mpv else { return };
        match p.kind {
            TrackKind::Sub => mpv.set_sub_id(t.id),
            TrackKind::Audio => mpv.set_audio_id(t.id),
        }
        self.nudge_controls();
    }

    pub fn start(
        &mut self,
        api: &Api,
        id: i64,
        title: &str,
        bitrate: i64,
        duration: i32,
        origin: Origin,
        resume_pos: f64,
    ) {
        self.shutdown();
        /* Ikke gjenbruk gammel subsurface — be App lage en fersk. shutdown()
         * har allerede joinet render/watchdog-trådene, så gammel EGL/CUDA-GL-
         * context er revet ned før den nye bygges. */
        self.wants_fresh_surface = true;
        self.title = title.to_string();
        self.media_id = Some(id);
        self.origin = Some(origin);
        self.bitrate = bitrate;
        self.duration = duration;
        self.is_live = false;
        self.resume_pos = resume_pos;
        self.auto_next_started_at = None;
        self.auth_header = api.auth_header().to_string();
        self.stream_url = api.stream_url(id);
        self.started_at = std::time::Instant::now();
        self.show_controls_until =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        *self.phase.lock() = Phase::Probing;
        self.probe
            .start(api, &self.rt, id, bitrate, duration as f64, resume_pos);
    }

    pub fn start_url(&mut self, url: &str, title: &str) {
        self.shutdown();
        self.wants_fresh_surface = true;
        self.title = title.to_string();
        self.media_id = None;
        self.origin = None;
        self.bitrate = 0;
        self.duration = 0;
        self.is_live = true;
        self.resume_pos = 0.0;
        self.auto_next_started_at = None;
        self.auth_header = String::new();
        self.stream_url = url.to_string();
        self.started_at = std::time::Instant::now();
        self.show_controls_until =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        self.probe.set_instant(60);
        *self.phase.lock() = Phase::Probing;
    }

    pub fn set_subsurface(&mut self, sub: Arc<SubsurfaceVideo>) {
        self.subsurface = Some(sub);
    }

    /* Slipp PlayerViews Arc-klon av subsurface. App slipper sin og bygger
     * en fersk (se ensure_subsurface_for_player). */
    pub fn clear_subsurface(&mut self) {
        self.subsurface = None;
    }

    pub fn poll(&mut self) {
        let snap = self.probe.snapshot();
        let phase = *self.phase.lock();

        match phase {
            Phase::Probing => {
                if self.subsurface.is_none() {
                    return;
                }
                /* Start mpv (loadfile → demux + decoder-init + første GOP)
                 * UMIDDELBART, parallelt med båndbredde-proben, i stedet for å
                 * vente på probe-resultatet. Dekoder-init + første-frame-decode
                 * er den dyre delen av oppstart; ved å la den kjøre mens proben
                 * måler, er unpause praktisk talt instant på LAN. mpv holdes
                 * paused til vi vet policy. cache-secs dimensjoneres fra
                 * varighet; disk_cache=false (RAM 2 GiB-cap holder — treg linje
                 * som egentlig ville disk-backe degraderer til mer rebuffering,
                 * ikke OOM). */
                if self.mpv.is_none() {
                    let remaining = if self.duration > 0 {
                        (self.duration as f64 - self.resume_pos).max(60.0)
                    } else {
                        900.0
                    };
                    let cache_secs = (remaining as u32).saturating_add(120).max(900);
                    match self.init_mpv(cache_secs, false) {
                        Ok(mpv) => {
                            mpv.set_pause(true);
                            self.mpv = Some(mpv);
                        }
                        Err(e) => {
                            *self.error.lock() = Some(e.to_string());
                            *self.phase.lock() = Phase::Error;
                            return;
                        }
                    }
                }
                if let Some(res) = &snap.result {
                    self.probe_result = Some(res.clone());
                    let Some(mpv) = self.mpv.clone() else { return };
                    /* Bytt konservativt oppstart-estimat mot policyens
                     * autoritative readahead-mål nå som proben er ferdig. */
                    mpv.set_cache_secs(res.cache_seconds);
                    if res.disk_cache {
                        crate::nlog!(
                            "probe ville disk-backe cache, men mpv er alt startet med RAM-cache \
                             — degraderer til mer rebuffering (treg linje)"
                        );
                    }
                    if matches!(res.policy, BufferPolicy::InstantStart) {
                        mpv.set_pause(false);
                        *self.phase.lock() = Phase::Playing;
                    } else {
                        self.preload_started_at = Some(std::time::Instant::now());
                        *self.phase.lock() = Phase::Preloading;
                    }
                }
            }
            Phase::Preloading => {
                if let (Some(mpv), Some(res), Some(t0)) =
                    (&self.mpv, &self.probe_result, self.preload_started_at)
                {
                    let elapsed = t0.elapsed().as_secs_f64();
                    /* Mål i INNHOLDS-sekunder (beregnet av compute_policy ut
                     * fra linjehastighet vs reell bitrate). Er resume nær
                     * slutten kan hele resten av filen være mindre enn målet
                     * — da holder det å ha buffret resten. */
                    let target = res.preload_seconds as f64;
                    let rest = if self.duration > 0 {
                        (self.duration as f64 - self.resume_pos - 2.0).max(0.0)
                    } else {
                        f64::INFINITY
                    };
                    let buffered = mpv.demuxer_cache_duration().unwrap_or(0.0);
                    /* Hard-cap mot evig venting på linje som døde etter
                     * proben. Starter vi for tidlig fanger mpv cache-pause
                     * underløpet og rebuffrer istf å fryse. */
                    let hard_cap = (res.expected_wall_secs * 3.0).max(120.0);
                    /* Demuxer idle = cache full (byte-cap; VBR kan treffe
                     * 2 GiB RAM-cap før sekund-målet) eller EOF (metadata-
                     * varighet lengre enn reell fil). Mer data kommer ikke
                     * — start. 2s-vakt mot idle-glimt rett etter loadfile. */
                    let stalled_full = mpv.demuxer_cache_idle() && elapsed > 2.0;
                    let ready = buffered + 0.5 >= target.min(rest)
                        || stalled_full
                        || elapsed >= hard_cap;
                    if ready {
                        crate::nlog!(
                            "preload done: {buffered:.1}s buffret (mål {target:.0}s) etter {elapsed:.0}s"
                        );
                        mpv.set_pause(false);
                        *self.phase.lock() = Phase::Playing;
                    }
                }
            }
            Phase::Playing | Phase::Error | Phase::Idle => {}
        }
    }

    fn init_mpv(&self, cache_secs: u32, disk_cache: bool) -> anyhow::Result<Arc<MpvPlayer>> {
        let sub = self
            .subsurface
            .clone()
            .ok_or_else(|| anyhow::anyhow!("subsurface not initialized"))?;
        MpvPlayer::new(
            &self.stream_url,
            &self.auth_header,
            cache_secs,
            sub,
            self.resume_pos,
            self.is_live,
            disk_cache,
        )
    }

    pub fn shutdown(&mut self) {
        if let Some(mpv) = self.mpv.take() {
            mpv.stop();
            mpv.shutdown();
            /* Vent til render-tråden har frigjort EGL/CUDA-context FØR vi
             * returnerer. start() gjenbruker samme subsurface for neste
             * stream; uten denne barrieren kjører gammel teardown og ny
             * render-context samtidig mot surface → krasj ved episode-bytte. */
            mpv.join();
        }
        self.subsurface = None;
        *self.phase.lock() = Phase::Idle;
        self.probe_result = None;
        self.preload_started_at = None;
    }

    pub fn toggle_pause(&self) {
        if let Some(mpv) = &self.mpv {
            mpv.set_pause(!mpv.paused());
        }
    }

    pub fn seek(&self, secs: f64) {
        if let Some(mpv) = &self.mpv {
            mpv.seek(secs);
        }
    }

    /* Start hold-spoling. Stopper evt. tidligere hold (sikrer single dir),
     * setter mpv på pause så lyd/video ikke krøller seg under raske
     * keyframe-hopp, og initialiserer tick-state. Tick utstedes i poll()
     * mens self.seek_hold er Some. */
    pub fn start_seek_hold(&mut self, dir: SeekDir) {
        if !matches!(*self.phase.lock(), Phase::Playing) {
            return;
        }
        if let Some(mpv) = &self.mpv {
            mpv.set_pause(true);
            /* Første tick umiddelbart — bruker forventer feedback med
             * en gang knappen presses, ikke etter 100 ms tick-intervall. */
            let step = 2.0;
            let secs = match dir {
                SeekDir::Back => -step,
                SeekDir::Forward => step,
            };
            mpv.seek_keyframe(secs);
        }
        let now = std::time::Instant::now();
        self.seek_hold = Some(SeekHold {
            dir,
            started_at: now,
            last_tick: now,
        });
        self.nudge_controls();
    }

    /* Stopp hold-spoling og resume playback umiddelbart. */
    pub fn stop_seek_hold(&mut self) {
        if self.seek_hold.take().is_some() {
            if let Some(mpv) = &self.mpv {
                mpv.set_pause(false);
            }
        }
        self.nudge_controls();
    }

    /* Per-frame tick. Utsteder nytt keyframe-seek hvert ~120 ms med
     * akselererende step: 2 s først, 10 s etter 1 s hold, 30 s etter 3 s.
     * Keyframes-modus = mpv hopper til nærmeste I-frame uten å dekode
     * mellomliggende — orders of magnitude raskere enn eksakt seek. */
    pub fn tick_seek_hold(&mut self) {
        let Some(hold) = self.seek_hold.as_mut() else { return };
        let now = std::time::Instant::now();
        if now.duration_since(hold.last_tick).as_millis() < 120 {
            return;
        }
        hold.last_tick = now;
        let held = now.duration_since(hold.started_at).as_secs_f64();
        let step = if held < 1.0 {
            2.0
        } else if held < 3.0 {
            10.0
        } else {
            30.0
        };
        let secs = match hold.dir {
            SeekDir::Back => -step,
            SeekDir::Forward => step,
        };
        if let Some(mpv) = &self.mpv {
            mpv.seek_keyframe(secs);
        }
        self.nudge_controls();
    }

    pub fn is_seeking(&self) -> bool {
        self.seek_hold.is_some()
    }

    pub fn nudge_controls(&mut self) {
        self.show_controls_until =
            std::time::Instant::now() + std::time::Duration::from_secs(2);
    }

    pub fn controls_visible(&self) -> bool {
        std::time::Instant::now() < self.show_controls_until
            || matches!(*self.phase.lock(), Phase::Probing | Phase::Preloading | Phase::Error)
    }

    pub fn progress_snapshot(&self) -> ProbeProgress {
        self.probe.snapshot()
    }

    pub fn phase(&self) -> Phase {
        *self.phase.lock()
    }
}
