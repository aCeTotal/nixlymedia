use std::ffi::{c_void, CString};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use libmpv2::Mpv;
use parking_lot::Mutex;

use crate::config;
use crate::player::gl_loader;

pub struct MpvPlayer {
    pub render: Arc<Mutex<RenderContext>>,
    pub wake: Arc<Mutex<Option<egui::Context>>>,
    pub mpv: Mpv,
}

unsafe impl Send for MpvPlayer {}
unsafe impl Sync for MpvPlayer {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrackKind {
    Sub,
    Audio,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: i64,
    pub lang: String,
    pub title: String,
    pub selected: bool,
}

impl Track {
    pub fn display(&self) -> String {
        let mut s = format!("#{}", self.id);
        if !self.lang.is_empty() {
            s.push_str(&format!(" · {}", self.lang));
        }
        if !self.title.is_empty() {
            s.push_str(&format!(" · {}", self.title));
        }
        s
    }
}

#[derive(Clone)]
pub struct GlCtx;

fn proc_loader(_ctx: &GlCtx, name: &str) -> *mut c_void {
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return std::ptr::null_mut(),
    };
    unsafe { gl_loader::get_proc(&cname) }
}

impl MpvPlayer {
    pub fn new(stream_url: &str, auth_header: &str, cache_secs: u32) -> Result<Self> {
        let mpv = Mpv::with_initializer(|init| {
            let cache_secs = cache_secs.max(900);
            init.set_property("log-file", "/tmp/nixlymedia-mpv.log")?;
            init.set_property("msg-level", "all=v")?;
            init.set_property("vo", "libmpv")?;
            init.set_property("gpu-api", "opengl")?;
            init.set_property("gpu-context", "auto")?;
            init.set_property("hwdec", "auto-safe")?;
            init.set_property("hwdec-codecs", "all")?;
            init.set_property("hwdec-extra-frames", 8_i64)?;
            init.set_property("vd-lavc-dr", "yes")?;
            init.set_property("vd-lavc-fast", "yes")?;
            init.set_property("vd-lavc-threads", 0_i64)?;
            init.set_property("vd-lavc-skiploopfilter", "nonref")?;
            init.set_property("vd-lavc-skipframe", "default")?;
            init.set_property("vd-lavc-skipidct", "default")?;
            init.set_property("vd-lavc-software-fallback", "yes")?;
            init.set_property("video-sync", "display-resample")?;
            init.set_property("video-sync-max-video-change", 10.0)?;
            init.set_property("video-sync-max-audio-change", 0.5)?;
            init.set_property("interpolation", "yes")?;
            init.set_property("interpolation-preserve", "yes")?;
            init.set_property("interpolation-threshold", 0.0001)?;
            init.set_property("tscale", "mitchell")?;
            init.set_property("tscale-clamp", 0.0)?;
            init.set_property("audio-pitch-correction", "yes")?;
            init.set_property("audio-buffer", 0.3)?;
            init.set_property("video-latency-hacks", "no")?;
            init.set_property("framedrop", "no")?;
            init.set_property("opengl-pbo", "yes")?;
            init.set_property("opengl-swapinterval", 1_i64)?;
            init.set_property("swapchain-depth", 3_i64)?;
            init.set_property("vd-queue-enable", "yes")?;
            init.set_property("vd-queue-max-bytes", 1024_i64 * 1024 * 1024)?;
            init.set_property("vd-queue-max-secs", 8.0)?;
            init.set_property("ad-queue-enable", "yes")?;
            init.set_property("ad-queue-max-secs", 12.0)?;
            init.set_property("scale", "bilinear")?;
            init.set_property("dscale", "bilinear")?;
            init.set_property("cscale", "bilinear")?;
            init.set_property("dither", "no")?;
            init.set_property("keepaspect", "yes")?;
            init.set_property("slang", config::SUB_LANG_PREFS)?;
            init.set_property("alang", config::AUDIO_LANG_PREFS)?;
            init.set_property("sub-auto", "fuzzy")?;
            init.set_property("cache", "yes")?;
            init.set_property("cache-secs", cache_secs as i64)?;
            init.set_property("cache-pause", "no")?;
            init.set_property("cache-pause-initial", "no")?;
            init.set_property("cache-pause-wait", 1.0)?;
            init.set_property("demuxer-max-bytes", 16_i64 * 1024 * 1024 * 1024)?;
            init.set_property("demuxer-max-back-bytes", 4_i64 * 1024 * 1024 * 1024)?;
            init.set_property("demuxer-readahead-secs", cache_secs as i64)?;
            init.set_property("demuxer-thread", "yes")?;
            init.set_property("demuxer-termination-timeout", 1.0)?;
            init.set_property("demuxer-lavf-analyzeduration", 0.5)?;
            init.set_property("demuxer-lavf-probesize", 1_048_576_i64)?;
            init.set_property("stream-buffer-size", 256_i64 * 1024 * 1024)?;
            init.set_property("network-timeout", 60_i64)?;
            init.set_property("prefetch-playlist", "yes")?;
            init.set_property("force-seekable", "yes")?;
            init.set_property("keep-open", "yes")?;
            init.set_property("idle", "yes")?;
            init.set_property("input-default-bindings", "no")?;
            init.set_property("input-vo-keyboard", "no")?;
            init.set_property("osc", "no")?;
            init.set_property("osd-bar", "no")?;
            init.set_property("user-agent", "nixlymedia/0.1")?;
            init.set_property(
                "http-header-fields",
                format!("Authorization: {auth_header}"),
            )?;
            Ok(())
        })
        .map_err(|e| anyhow!("mpv init: {e}"))?;

        let mut mpv = mpv;
        let mut render = RenderContext::new(
            unsafe { mpv.ctx.as_mut() },
            [
                RenderParam::ApiType(RenderParamApiType::OpenGl),
                RenderParam::InitParams(OpenGLInitParams {
                    get_proc_address: proc_loader,
                    ctx: GlCtx,
                }),
            ],
        )
        .map_err(|e| anyhow!("mpv render context: {e}"))?;

        let wake = Arc::new(Mutex::new(None::<egui::Context>));
        let wake_for_cb = wake.clone();
        render.set_update_callback(move || {
            if let Some(ctx) = wake_for_cb.lock().as_ref() {
                ctx.request_repaint();
            }
        });

        mpv.command("loadfile", &[stream_url])
            .map_err(|e| anyhow!("mpv loadfile: {e}"))?;

        Ok(Self {
            render: Arc::new(Mutex::new(render)),
            wake,
            mpv,
        })
    }

    pub fn attach_repaint(&self, ctx: &egui::Context) {
        *self.wake.lock() = Some(ctx.clone());
    }

    pub fn render_to_fbo(&self, fbo: i32, w: i32, h: i32, flip_y: bool) -> Result<()> {
        self.render
            .lock()
            .render::<GlCtx>(fbo, w, h, flip_y)
            .map_err(|e| anyhow!("mpv render: {e}"))
    }

    pub fn set_pause(&self, paused: bool) {
        let _ = self.mpv.set_property("pause", paused);
    }

    pub fn seek(&self, secs: f64) {
        let s = format!("{secs:.2}");
        let _ = self.mpv.command("seek", &[&s, "relative"]);
    }

    pub fn set_sub_id(&self, id: i64) {
        let _ = self.mpv.set_property("sid", id);
    }

    pub fn set_audio_id(&self, id: i64) {
        let _ = self.mpv.set_property("aid", id);
    }

    pub fn current_sid(&self) -> Option<i64> {
        self.mpv.get_property::<i64>("sid").ok()
    }

    pub fn current_aid(&self) -> Option<i64> {
        self.mpv.get_property::<i64>("aid").ok()
    }

    pub fn tracks(&self, kind: TrackKind) -> Vec<Track> {
        let count = self
            .mpv
            .get_property::<i64>("track-list/count")
            .unwrap_or(0);
        let kind_str = match kind {
            TrackKind::Sub => "sub",
            TrackKind::Audio => "audio",
        };
        let mut out = Vec::new();
        for i in 0..count {
            let t = self
                .mpv
                .get_property::<String>(&format!("track-list/{i}/type"))
                .unwrap_or_default();
            if t != kind_str {
                continue;
            }
            let id = self
                .mpv
                .get_property::<i64>(&format!("track-list/{i}/id"))
                .unwrap_or(0);
            let lang = self
                .mpv
                .get_property::<String>(&format!("track-list/{i}/lang"))
                .unwrap_or_default();
            let title = self
                .mpv
                .get_property::<String>(&format!("track-list/{i}/title"))
                .unwrap_or_default();
            let selected = self
                .mpv
                .get_property::<bool>(&format!("track-list/{i}/selected"))
                .unwrap_or(false);
            out.push(Track {
                id,
                lang,
                title,
                selected,
            });
        }
        out
    }

    pub fn time_pos(&self) -> Option<f64> {
        self.mpv.get_property::<f64>("time-pos").ok()
    }

    pub fn duration(&self) -> Option<f64> {
        self.mpv.get_property::<f64>("duration").ok()
    }

    pub fn paused(&self) -> bool {
        self.mpv.get_property::<bool>("pause").unwrap_or(false)
    }

    pub fn cache_buffering(&self) -> Option<i64> {
        self.mpv.get_property::<i64>("cache-buffering-state").ok()
    }

    pub fn stop(&self) {
        let _ = self.mpv.command("stop", &[]);
    }

}
