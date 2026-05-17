use std::sync::Arc;

use parking_lot::Mutex;
use tokio::runtime::Handle;

use crate::api::Api;
use crate::player::bandwidth::{BandwidthProbe, BufferPolicy, ProbeProgress, ProbeResult};
use crate::player::mpv::{MpvPlayer, Track, TrackKind};

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

pub struct PlayerView {
    #[allow(dead_code)]
    pub gl: Arc<glow::Context>,
    pub rt: Handle,

    pub mpv: Option<Arc<MpvPlayer>>,
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

    pub auth_header: String,
    pub stream_url: String,

    pub show_controls_until: std::time::Instant,

    pub control_focus: usize,
    pub popup: Option<Popup>,
}

impl PlayerView {
    pub fn new(gl: Arc<glow::Context>, rt: Handle) -> Self {
        Self {
            gl,
            rt,
            mpv: None,
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
            auth_header: String::new(),
            stream_url: String::new(),
            show_controls_until: std::time::Instant::now()
                + std::time::Duration::from_secs(2),
            control_focus: 2,
            popup: None,
        }
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
    ) {
        self.shutdown();
        self.title = title.to_string();
        self.media_id = Some(id);
        self.origin = Some(origin);
        self.bitrate = bitrate;
        self.duration = duration;
        self.auth_header = api.auth_header().to_string();
        self.stream_url = api.stream_url(id);
        self.started_at = std::time::Instant::now();
        self.show_controls_until =
            std::time::Instant::now() + std::time::Duration::from_secs(5);
        *self.phase.lock() = Phase::Probing;
        self.probe.start(api, &self.rt, id, bitrate);
    }

    pub fn poll(&mut self) {
        let snap = self.probe.snapshot();
        let phase = *self.phase.lock();

        match phase {
            Phase::Probing => {
                if let Some(res) = &snap.result {
                    self.probe_result = Some(res.clone());
                    match self.init_mpv(res.cache_seconds) {
                        Ok(p) => {
                            let mpv = Arc::new(p);
                            if matches!(res.policy, BufferPolicy::InstantStart) {
                                mpv.set_pause(false);
                                self.mpv = Some(mpv);
                                *self.phase.lock() = Phase::Playing;
                            } else {
                                mpv.set_pause(true);
                                self.mpv = Some(mpv);
                                self.preload_started_at = Some(std::time::Instant::now());
                                *self.phase.lock() = Phase::Preloading;
                            }
                        }
                        Err(e) => {
                            *self.error.lock() = Some(e.to_string());
                            *self.phase.lock() = Phase::Error;
                        }
                    }
                }
            }
            Phase::Preloading => {
                if let (Some(mpv), Some(res), Some(t0)) =
                    (&self.mpv, &self.probe_result, self.preload_started_at)
                {
                    let elapsed = t0.elapsed().as_secs_f64();
                    let buf = mpv.cache_buffering().unwrap_or(0) as f64;
                    let ready = buf >= 99.0 || elapsed >= res.preload_seconds as f64;
                    if ready {
                        mpv.set_pause(false);
                        *self.phase.lock() = Phase::Playing;
                    }
                }
            }
            Phase::Playing | Phase::Error | Phase::Idle => {}
        }
    }

    fn init_mpv(&self, cache_secs: u32) -> anyhow::Result<MpvPlayer> {
        MpvPlayer::new(&self.stream_url, &self.auth_header, cache_secs)
    }

    pub fn shutdown(&mut self) {
        if let Some(mpv) = self.mpv.take() {
            mpv.stop();
        }
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
