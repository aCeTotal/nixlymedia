use std::ffi::{c_void, CString};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, Result};
use libmpv2::render::{OpenGLInitParams, RenderParam, RenderParamApiType};
use libmpv2::Mpv;
use parking_lot::{Condvar, Mutex};

use crate::config;
use crate::player::hdr;
use crate::player::wl_subsurface::SubsurfaceVideo;

#[allow(dead_code)]
pub struct MpvPlayer {
    pub wake: Arc<Mutex<Option<egui::Context>>>,
    pub mpv: Mpv,
    pub subsurface: Arc<SubsurfaceVideo>,
    pub render_wake: Arc<(Mutex<bool>, Condvar)>,
    pub render_alive: Arc<Mutex<bool>>,
    pub render_thread: Option<thread::JoinHandle<()>>,
    pub hdr_meta: Arc<Mutex<Option<hdr::HdrMeta>>>,
    pub hdr_applied: Arc<Mutex<bool>>,
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
pub struct GlCtx {
    sub: Arc<SubsurfaceVideo>,
}

fn proc_loader(ctx: &GlCtx, name: &str) -> *mut c_void {
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return std::ptr::null_mut(),
    };
    let p = ctx.sub.get_proc(cname.to_str().unwrap_or(""));
    if !p.is_null() {
        return p;
    }
    unsafe { crate::player::gl_loader::get_proc(&cname) }
}

fn shader_cache_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::cache_dir())?;
    Some(base.join("nixlymedia").join("shaders"))
}

impl MpvPlayer {
    pub fn new(
        stream_url: &str,
        auth_header: &str,
        cache_secs: u32,
        subsurface: Arc<SubsurfaceVideo>,
    ) -> Result<Arc<Self>> {
        let log_on = crate::log::enabled();
        let mpv = Mpv::with_initializer(|init| {
            let cache_secs = cache_secs.max(900);
            if log_on {
                /* Skriv all mpv-output (decoder, vo, ao, statusline,
                 * frame-timing) til delt loggfil. Verbose nivå. */
                init.set_property("log-file", crate::log::path().as_str())?;
                init.set_property("msg-level", "all=v")?;
            } else {
                init.set_property("msg-level", "all=warn")?;
            }
            init.set_property("vo", "libmpv")?;
            init.set_property("gpu-api", "opengl")?;
            /* Eksplisitt Wayland EGL — unngår at "auto" tilfeldigvis
             * plukker x11egl eller waylandvk på rare driver-oppsett. */
            init.set_property("gpu-context", "wayland")?;
            let hwdec = crate::player::hwdec::detect();
            crate::nlog!("hwdec selected: {hwdec}");
            init.set_property("hwdec", hwdec)?;
            init.set_property("hwdec-codecs", "all")?;
            /* mpv default er 6. 4 kan stalle decoder pipeline på høy-
             * bitrate 4K HEVC når GL render trekker langsommere enn
             * NVDEC produserer. */
            init.set_property("hwdec-extra-frames", 6_i64)?;
            init.set_property("vd-lavc-dr", "yes")?;
            init.set_property("vd-lavc-fast", "yes")?;
            init.set_property("vd-lavc-threads", 0_i64)?;
            init.set_property("vd-lavc-software-fallback", "yes")?;
            init.set_property("opengl-pbo", "yes")?;
            /* Cache kompilerte GLSL shaders mellom oppstarter — sparer
             * 1-3s startup. ${XDG_CACHE_HOME:-~/.cache}/nixlymedia/shaders. */
            if let Some(cache) = shader_cache_dir() {
                let _ = std::fs::create_dir_all(&cache);
                if let Some(s) = cache.to_str() {
                    init.set_property("gpu-shader-cache-dir", s)?;
                }
            }
            /* Predictable audio backend rekkefølge. Default "auto" kan
             * havne på jack/oss på rare systemer. */
            init.set_property("ao", "pipewire,pulse,alsa")?;
            /* La mpv auto-forhandle channel layout med audio sink (PipeWire/
             * PulseAudio). Hardkodet 7.1 tvinger upmix/downmix selv på
             * stereo-content og kan gi feil mapping på 5.1-sinks. */
            init.set_property("audio-samplerate", 48000_i64)?;
            /* 1.0s buffer absorberer demuxer-spikes (observed swap_us=1220ms
             * stall) og TrueHD 8ch dekoderlast uten audible delay. */
            init.set_property("audio-buffer", 1.0)?;
            let igpu = crate::player::hwdec::is_intel_igpu_active();
            if igpu {
                init.set_property("scale", "spline36")?;
                init.set_property("dscale", "mitchell")?;
                init.set_property("cscale", "bilinear")?;
            } else {
                init.set_property("scale", "spline36")?;
                init.set_property("dscale", "mitchell")?;
                init.set_property("cscale", "spline36")?;
            }
            init.set_property("dither", "fruit")?;
            init.set_property("dither-depth", "auto")?;
            init.set_property("keepaspect", "yes")?;
            init.set_property("slang", config::SUB_LANG_PREFS)?;
            init.set_property("alang", config::AUDIO_LANG_PREFS)?;
            init.set_property("sub-auto", "fuzzy")?;
            init.set_property("cache", "yes")?;
            init.set_property("cache-secs", cache_secs as i64)?;
            init.set_property("cache-pause", "no")?;
            init.set_property("cache-pause-initial", "no")?;
            init.set_property("demuxer-max-bytes", 16_i64 * 1024 * 1024 * 1024)?;
            init.set_property("demuxer-max-back-bytes", 4_i64 * 1024 * 1024 * 1024)?;
            init.set_property("demuxer-readahead-secs", 300_i64)?;
            init.set_property("demuxer-thread", "yes")?;
            init.set_property("demuxer-termination-timeout", 1.0)?;
            init.set_property("stream-buffer-size", 256_i64 * 1024 * 1024)?;
            init.set_property("network-timeout", 600_i64)?;
            init.set_property("prefetch-playlist", "yes")?;
            init.set_property("force-seekable", "yes")?;
            init.set_property("keep-open", "yes")?;
            init.set_property("idle", "yes")?;
            init.set_property("input-default-bindings", "no")?;
            init.set_property("input-vo-keyboard", "no")?;
            init.set_property("osc", "no")?;
            init.set_property("osd-bar", "no")?;
            init.set_property("user-agent", "nixlymedia/0.1")?;
            init.set_property("target-prim", "auto")?;
            init.set_property("target-trc", "auto")?;
            init.set_property("target-peak", "auto")?;
            init.set_property("target-colorspace-hint", "yes")?;
            /* SDR-fallback default: BT.2390 gives smooth HDR→SDR rolloff
             * without crushed blacks or blown highlights. Flipped to "clip"
             * by set_passthrough_pq(true) when compositor accepts PQ. */
            init.set_property("tone-mapping", "bt.2390")?;
            /* Dynamisk peak — for SDR-fallback når source mangler MaxCLL.
             * Ignoreres når PQ passthrough er aktiv (tone-mapping=clip). */
            init.set_property("hdr-compute-peak", "yes")?;
            init.set_property("gamut-mapping-mode", "clip")?;
            /* Force full-range RGB out of mpv. Compositor handles final
             * monitor signalling; "auto" on Nvidia/HDMI can pick limited
             * range and crush blacks to grey. */
            init.set_property("video-output-levels", "full")?;
            /* Display-locked pacing. mpv måler fps fra report_swap()
             * intervaller (kall etter swap_buffers i render_thread). */
            init.set_property("video-sync", "display-resample")?;
            init.set_property("interpolation", "yes")?;
            init.set_property("tscale", "oversample")?;
            /* Debanding for dark gradients (common in HDR-sourced content).
             * 2 iterations is the mpv-default quality without GPU cost. */
            init.set_property("deband", "yes")?;
            init.set_property("deband-iterations", 2_i64)?;
            /* Antiringing on spline36 — removes halos without softening. */
            init.set_property("scale-antiring", 0.7)?;
            init.set_property("cscale-antiring", 0.7)?;
            init.set_property(
                "http-header-fields",
                format!("Authorization: {auth_header}"),
            )?;
            Ok(())
        })
        .map_err(|e| anyhow!("mpv init: {e}"))?;

        let wake = Arc::new(Mutex::new(None::<egui::Context>));
        let render_wake = Arc::new((Mutex::new(false), Condvar::new()));
        let render_alive = Arc::new(Mutex::new(true));
        let hdr_meta = Arc::new(Mutex::new(None));
        let hdr_applied = Arc::new(Mutex::new(false));

        let stream_url = stream_url.to_string();
        let player = MpvPlayer {
            wake,
            mpv,
            subsurface: subsurface.clone(),
            render_wake: render_wake.clone(),
            render_alive: render_alive.clone(),
            render_thread: None,
            hdr_meta: hdr_meta.clone(),
            hdr_applied,
        };

        let arc = Arc::new(player);

        let arc_for_thread = arc.clone();
        let sub_for_thread = subsurface;
        let wake_egui = arc.wake.clone();
        let render_wake_t = render_wake;
        let render_alive_t = render_alive;
        let hdr_meta_t = hdr_meta;

        /* Render context MUST exist before loadfile, ellers init libmpv vo
         * feiler ("No render context set") og video track droppes. */
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<()>>(1);

        let handle = thread::Builder::new()
            .name("nixlymedia-mpv-render".into())
            .spawn(move || {
                Self::render_thread_main(
                    arc_for_thread,
                    sub_for_thread,
                    wake_egui,
                    render_wake_t,
                    render_alive_t,
                    hdr_meta_t,
                    ready_tx,
                );
            })
            .map_err(|e| anyhow!("spawn render thread: {e}"))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(anyhow!("render context init: {e}")),
            Err(e) => return Err(anyhow!("render thread died: {e}")),
        }

        arc.mpv
            .command("loadfile", &[&stream_url])
            .map_err(|e| anyhow!("mpv loadfile: {e}"))?;

        // SAFETY: we just created the Arc, only one strong ref outside thread.
        // We need to stash the handle. Use Arc::get_mut isn't safe with the clone above.
        // Instead store handle via a side channel:
        unsafe {
            let raw = Arc::as_ptr(&arc) as *mut MpvPlayer;
            (*raw).render_thread = Some(handle);
        }

        Ok(arc)
    }

    fn render_thread_main(
        player: Arc<MpvPlayer>,
        sub: Arc<SubsurfaceVideo>,
        wake_egui: Arc<Mutex<Option<egui::Context>>>,
        render_wake: Arc<(Mutex<bool>, Condvar)>,
        render_alive: Arc<Mutex<bool>>,
        hdr_meta: Arc<Mutex<Option<hdr::HdrMeta>>>,
        ready_tx: mpsc::SyncSender<Result<()>>,
    ) {
        if let Err(e) = sub.make_current() {
            let msg = format!("render thread make_current: {e}");
            crate::nlog!("{msg}");
            let _ = ready_tx.send(Err(anyhow!(msg)));
            return;
        }

        let gl_ctx = GlCtx { sub: sub.clone() };
        let wl_display = sub.wl_display_ptr as *const c_void;
        /* libmpv2 6.x: RenderContext<'a> låner fra &Mpv. Arc<MpvPlayer>
         * holdes av denne tråden helt til den exiter, så lånet er gyldig
         * gjennom hele funksjonen. */
        let p_ref: &MpvPlayer = unsafe { &*Arc::as_ptr(&player) };
        let mut render = match p_ref.mpv.create_render_context([
            RenderParam::ApiType(RenderParamApiType::OpenGl),
            RenderParam::InitParams(OpenGLInitParams {
                get_proc_address: proc_loader,
                ctx: gl_ctx,
            }),
            RenderParam::WaylandDisplay(wl_display),
        ]) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("create_render_context failed: {e}");
                crate::nlog!("{msg}");
                let _ = ready_tx.send(Err(anyhow!(msg)));
                return;
            }
        };

        /* Signal init OK. main thread issuer loadfile etter dette → vo
         * libmpv finner aktiv render context. */
        let _ = ready_tx.send(Ok(()));

        let wake_pair = render_wake.clone();
        let wake_egui_cb = wake_egui.clone();
        render.set_update_callback(move || {
            let (lock, cv) = &*wake_pair;
            *lock.lock() = true;
            cv.notify_all();
            if let Some(ctx) = wake_egui_cb.lock().as_ref() {
                ctx.request_repaint();
            }
        });

        let mut last_hdr_probe = std::time::Instant::now() - Duration::from_secs(2);
        let mut last_stats = std::time::Instant::now();
        let mut rendered_count: u64 = 0;
        let mut presented_total: u64 = 0;
        let mut render_err_count: u64 = 0;
        let mut swap_err_count: u64 = 0;
        let log_stats = crate::log::enabled();

        loop {
            if !*render_alive.lock() {
                break;
            }

            // Wait for wake signal (with timeout for periodic HDR probe)
            {
                let (lock, cv) = &*render_wake;
                let mut pending = lock.lock();
                if !*pending {
                    let _ = cv
                        .wait_for(&mut pending, Duration::from_millis(100));
                }
                *pending = false;
            }

            if !*render_alive.lock() {
                break;
            }

            let (w, h) = sub.dimensions();
            let t_render = std::time::Instant::now();
            if let Err(e) = render.render::<GlCtx>(0, w, h, true) {
                crate::nlog!("mpv render: {e}");
                render_err_count += 1;
            }
            let render_us = t_render.elapsed().as_micros();
            /* Be om presentation feedback FØR commit (swap_buffers
             * committer surface). */
            sub.request_presentation_feedback();
            let t_swap = std::time::Instant::now();
            if let Err(e) = sub.swap_buffers() {
                crate::nlog!("swap_buffers: {e}");
                swap_err_count += 1;
            }
            let swap_us = t_swap.elapsed().as_micros();
            rendered_count += 1;
            /* Dispatch wayland events for å fange presented-events fra
             * tidligere frames. Hver bekreftet presentation = ett
             * vsync-intervall til mpv. Fallback: hvis wp_presentation
             * mangler, kall én gang per swap. */
            sub.pump();
            let presented = sub.take_presented();
            presented_total += presented;
            if presented == 0 {
                render.report_swap();
            } else {
                for _ in 0..presented {
                    render.report_swap();
                }
            }

            /* Periodisk frame-timing stats (kun når logging aktivt). */
            if log_stats && last_stats.elapsed() >= Duration::from_secs(1) {
                let dt = last_stats.elapsed().as_secs_f64();
                last_stats = std::time::Instant::now();
                let rendered_fps = rendered_count as f64 / dt;
                let presented_fps = presented_total as f64 / dt;
                let p = unsafe { &(*Arc::as_ptr(&player)).mpv };
                let dropped = p.get_property::<i64>("frame-drop-count").unwrap_or(0);
                let vo_dropped = p.get_property::<i64>("vo-delayed-frame-count").unwrap_or(0);
                let av_diff = p.get_property::<f64>("avsync").unwrap_or(0.0);
                let cache_state = p.get_property::<i64>("cache-buffering-state").unwrap_or(0);
                let demuxer_secs = p.get_property::<f64>("demuxer-cache-duration").unwrap_or(0.0);
                let est_fps = p.get_property::<f64>("estimated-vf-fps").unwrap_or(0.0);
                let container_fps = p.get_property::<f64>("container-fps").unwrap_or(0.0);
                crate::nlog!(
                    "stats: render_fps={rendered_fps:.2} present_fps={presented_fps:.2} \
                     render_us={render_us} swap_us={swap_us} \
                     drop={dropped} vo_delay={vo_dropped} av_diff={av_diff:+.3} \
                     buf={cache_state}% demux_s={demuxer_secs:.1} \
                     vf_fps={est_fps:.3} src_fps={container_fps:.3} \
                     err_render={render_err_count} err_swap={swap_err_count}"
                );
                rendered_count = 0;
                presented_total = 0;
            }

            // Periodic HDR detection (cheap, every ~500ms)
            if last_hdr_probe.elapsed() > Duration::from_millis(500) {
                last_hdr_probe = std::time::Instant::now();
                let meta = hdr::detect(unsafe { &(*Arc::as_ptr(&player)).mpv });
                *hdr_meta.lock() = meta;
            }
        }

        let _ = sub.release_current();
    }

    pub fn attach_repaint(&self, ctx: &egui::Context) {
        *self.wake.lock() = Some(ctx.clone());
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

    pub fn current_hdr(&self) -> Option<hdr::HdrMeta> {
        self.hdr_meta.lock().clone()
    }

    pub fn set_passthrough_pq(&self, enable: bool) {
        if enable {
            /* Compositor will present PQ. mpv must emit PQ untouched
             * (tone-mapping=clip = "do nothing"). */
            let _ = self.mpv.set_property("target-prim", "bt.2020");
            let _ = self.mpv.set_property("target-trc", "pq");
            let _ = self.mpv.set_property("tone-mapping", "clip");
        } else {
            /* SDR display: restore BT.2390 rolloff (matches init default). */
            let _ = self.mpv.set_property("target-prim", "auto");
            let _ = self.mpv.set_property("target-trc", "auto");
            let _ = self.mpv.set_property("tone-mapping", "bt.2390");
        }
    }

    pub fn stop(&self) {
        let _ = self.mpv.command("stop", &[]);
    }

    pub fn shutdown(&self) {
        *self.render_alive.lock() = false;
        let (lock, cv) = &*self.render_wake;
        *lock.lock() = true;
        cv.notify_all();
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(h) = self.render_thread.take() {
            let _ = h.join();
        }
    }
}
