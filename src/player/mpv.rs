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
    pub watchdog_alive: Arc<std::sync::atomic::AtomicBool>,
    pub watchdog_thread: Option<thread::JoinHandle<()>>,
    pub stream_url: String,
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
            /* vd-lavc-fast=no: ikke skip-loop-filter / fast-IDCT i ffmpeg
             * software-fallback. NVDEC-pathen bryr seg ikke, men hvis
             * codec faller til SW (uvanlig profil/farge) får vi full
             * dekoder-kvalitet istf speedup-shortcuts. */
            init.set_property("vd-lavc-fast", "no")?;
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
            /* LPCM-only path. Bypass PipeWire — PW HDMI-sink-profil
             * følger EDID, så TV som rapporterer stereo gjør at PW
             * eksponerer 2ch og downmixer 5.1 før ALSA. Vi åpner ALSA
             * hw: direkte (kernel-driver sjekker ikke EDID) og sender
             * 6ch s32 LPCM rett ut HDMI. TV/AVR mottar full 5.1 over
             * eARC uavhengig av EDID-rapportering.
             *
             * Fallback til pipewire/pulse hvis HDMI-detect feiler eller
             * ikke noe HDMI er tilkoblet (f.eks. ren desktop-bruk). */
            match crate::player::audio_alsa::detect_hdmi_hw() {
                Some(dev) => {
                    crate::nlog!("audio: ALSA hw direct -> {dev}");
                    init.set_property("ao", "alsa,pipewire,pulse")?;
                    init.set_property("audio-device", dev.as_str())?;
                }
                None => {
                    crate::nlog!("audio: no HDMI ELD found, fallback to pipewire");
                    init.set_property("ao", "pipewire,pulse,alsa")?;
                }
            }
            /* Be om FL/FR/FC/LFE/SL/SR. Med ALSA hw: aksepterer Nvidia
             * HDMI 6ch s32le 48000 uavhengig av EDID. */
            init.set_property("audio-channels", "5.1")?;
            init.set_property("audio-samplerate", 48000_i64)?;
            /* 32-bit float intern → 32-bit signed ut. Maksimal headroom
             * for downmix-summering (8ch→5.1 + LFE-mix) uten clipping
             * eller dither-tap. HDMI/eARC tar s32 LPCM. */
            init.set_property("audio-format", "s32")?;
            /* Bevar original kilde-layout gjennom dekoderen; la mpv selv
             * gjøre downmix til 5.1 med høy-presisjon float-matrise. */
            init.set_property("ad-lavc-downmix", "no")?;
            /* SOX-kvalitet resampler. Default filter-size=16/cutoff=0.94
             * gir hørbar aliasing på HD-kilder. 32-tap + 0.97 cutoff +
             * 14-bit phase = transparent kvalitet, neglisjerbar CPU. */
            init.set_property("audio-resample-filter-size", 32_i64)?;
            init.set_property("audio-resample-cutoff", 0.97)?;
            init.set_property("audio-resample-phase-shift", 14_i64)?;
            /* Ingen volum-attenuering, ingen ReplayGain-justering. */
            init.set_property("volume", 100.0)?;
            init.set_property("replaygain", "no")?;
            /* 1.0s buffer absorberer demuxer-spikes (observed swap_us=1220ms
             * stall) og TrueHD 8ch dekoderlast uten audible delay. */
            init.set_property("audio-buffer", 1.0)?;
            /* Default audio-delay kompenserer for typisk Wayland compositor-
             * latency: video går render→swap→compositor commit→present (~1-2
             * frames), mens ALSA hw: direct sender lyd rett ut HDMI med
             * minimal latency. Resultat: lyd ligger ~1 frame foran video.
             * +0.040 s ≈ 1 frame @ 24 fps. Bruker kan finjustere live med
             * [ / ] (±10 ms) eller Shift+[ / Shift+] (±50 ms). */
            init.set_property("audio-delay", crate::config::AUDIO_DELAY_DEFAULT)?;
            let igpu = crate::player::hwdec::is_intel_igpu_active();
            if igpu {
                init.set_property("scale", "spline36")?;
                init.set_property("dscale", "mitchell")?;
                init.set_property("cscale", "bilinear")?;
            } else {
                /* Luma upscale: ewa_lanczossharp + antiring=1.0 (nedenfor)
                 * = skarp uten halos. Chroma håndteres av KrigBilateral
                 * shader (CHROMA hook) — Kriging-basert luma-guided
                 * upscaler som dropper standard cscale. mitchell står som
                 * fallback hvis shader-fil mangler eller hooket ikke fyrer. */
                init.set_property("scale", "ewa_lanczossharp")?;
                init.set_property("dscale", "mitchell")?;
                init.set_property("cscale", "mitchell")?;
            }
            /* KrigBilateral chroma-from-luma. Beste tilgjengelige
             * chroma-rekonstruksjon — bruker luma som guide for å predikere
             * chroma. Klart bedre enn mitchell på text-kanter og fine
             * detaljer. Skrives til XDG cache ved oppstart. Hvis IO feiler,
             * mpv fortsetter med mitchell. */
            if let Some(path) = crate::player::shaders::krig_bilateral_path() {
                init.set_property("glsl-shaders", path.as_str())?;
                crate::nlog!("glsl-shaders = {path}");
            } else {
                crate::nlog!("KrigBilateral shader unavailable, cscale=mitchell fallback");
            }
            /* error-diffusion: Floyd-Steinberg-style spatial dither, langt
             * bedre enn ordered (fruit) for 10→8-bit på panels uten ekte
             * 10-bit input. Krever compute shaders (GL 4.3+, sikret av
             * EGL-context-bump i wl_subsurface). Hvis compute er
             * utilgjengelig (gammelt hardware), faller mpv tilbake til
             * fruit automatisk. */
            init.set_property("dither", "error-diffusion")?;
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
            /* Live TS leverer 1x realtime; 300s readahead jakter data som
             * ikke eksisterer ennå → demuxer-thread spinner og maskerer
             * stall-deteksjon. 60s holder for å absorbere kortvarige hikk
             * uten å forvirre cache-buffering-state. */
            init.set_property("demuxer-readahead-secs", 60_i64)?;
            init.set_property("demuxer-thread", "yes")?;
            init.set_property("demuxer-termination-timeout", 1.0)?;
            init.set_property("stream-buffer-size", 256_i64 * 1024 * 1024)?;
            /* 600s timeout = stall blokkerer 10 min før reconnect prøver.
             * 30s er nok for langsomme servere men gir reconnect_on_network_
             * error sjanse til å fyre raskt. */
            init.set_property("network-timeout", 30_i64)?;
            init.set_property("prefetch-playlist", "yes")?;
            /* force-seekable=yes på live TS provoserer Range-requests som
             * 416 og bryter cache. Default (auto) — la mpv detektere. */
            init.set_property("keep-open", "yes")?;
            init.set_property("idle", "yes")?;
            init.set_property("input-default-bindings", "no")?;
            init.set_property("input-vo-keyboard", "no")?;
            init.set_property("osc", "no")?;
            init.set_property("osd-bar", "no")?;
            /* IPTV TS over HTTP freeze etter 5-10s = TCP-hiccup → EOF, og med
             * keep-open=yes går mpv idle. FFmpeg HTTP-protokollen reconnect
             * options er av default. stream-lavf-o = AVOptions for streams
             * åpnet via stream_lavf (http/https/etc). reconnect_streamed
             * dekker non-seekable live TS. reconnect_at_eof = reopen ved
             * server-close. reconnect_on_network_error = reopen ved ECONN*.
             * reconnect_max_retries=20 = ~100s med backoff før gi opp. */
            /* multiple_requests=1: ffmpeg HTTP-protokoll gjenbruker samme TCP-
             * connection for påfølgende Range-requests istedenfor å reconnecte
             * per range. Krever server med ekte keep-alive (vår nixlymediaserver
             * forhandler Connection-header siden keep-alive-fixen). Uten dette
             * lukker ffmpeg socket etter hver request og keep-alive blir bortkastet. */
            let reconnect_opts = "reconnect=1,reconnect_streamed=1,reconnect_delay_max=5,reconnect_at_eof=1,reconnect_on_network_error=1,reconnect_on_http_error=4xx\\,5xx,reconnect_max_retries=20,multiple_requests=1";
            init.set_property("stream-lavf-o", reconnect_opts)?;
            /* demuxer-lavf-o speiler stream-lavf-o men gjelder lavf-demuxerens
             * nested IO (HLS variant playlists, fmp4 fragmenter). Uten dette
             * vil HLS-IPTV miste sub-streams ved kortvarig hikk selv om
             * master playlist har reconnect på. */
            init.set_property("demuxer-lavf-o", reconnect_opts)?;
            /* Mange IPTV-providere blokkerer ukjente User-Agents (anti-leech).
             * VLC-string er universelt akseptert. */
            init.set_property("user-agent", "VLC/3.0.20 LibVLC/3.0.20")?;
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
            /* 99.995 = ignorer øverste 0.005% pixels ved peak-deteksjon.
             * Hindrer specular highlights fra å presse hele scenens
             * tonemapping mørkere. Synlig løft i HDR-kontrast. */
            init.set_property("hdr-peak-percentile", 99.995)?;
            /* Perceptual bevarer out-of-gamut detalj ved soft-rolloff;
             * clip kapper hardt og taper fargenuanser i mettede partier
             * (sterke røde/grønne i HDR-kilder). */
            init.set_property("gamut-mapping-mode", "perceptual")?;
            /* Force full-range RGB out of mpv. Compositor handles final
             * monitor signalling; "auto" on Nvidia/HDMI can pick limited
             * range and crush blacks to grey. */
            init.set_property("video-output-levels", "full")?;
            /* Audio-master pacing. display-resample chainet audio til
             * compositor sin rapporterte display-fps; selv små drift
             * mellom rapportert refresh og faktisk swap-intervall (f.eks.
             * 23.976 vs 24.75Hz reelt) ga konstant audio-resampling og
             * hørbar stutter (av_diff toggle 0/-40ms hvert ~5s). Audio
             * master + nixlytile mode-switch til eksakt 23.976/VRR gjør
             * at video selv sitter ren mot display-klokken, mens lyden
             * leveres bit-stabilt. interpolation krever display-sync, så
             * den skrus av — ingen effekt med audio-sync uansett. */
            init.set_property("video-sync", "audio")?;
            init.set_property("interpolation", "no")?;
            /* mpv default 0.050s — render() blokkerer ~50ms før hvert
             * supposed display-time. Kombinert med eglSwapBuffers vsync-
             * throttle gir det dobbel pacing og 10-15 fps render-loop.
             * Vi eier timing via swap+report_swap (BlockForTargetTime=
             * false nedenfor). 0 = ingen pre-render delay. */
            init.set_property("video-timing-offset", 0.0)?;
            /* Debanding for mørke gradienter på 10-bit HDR → 8/10-bit scanout.
             * Røyk, skybanker og fade-to-black er typiske banding-magneter.
             * iter=3 + range=20 = bredt sample-vindu, mer aggressiv enn
             * default, men terskel=48 hindrer sampler fra å hoppe over
             * skarpe kanter (tekst, kontrast-objekter). Med cscale fikset
             * (KrigBilateral) er det ingen chroma-ringer som forsterker
             * deband-halo, så vi kan kjøre sterkere uten tekst-artifakter.
             * grain=8 legger stokastisk støy som maskerer residual banding;
             * holdes lavt fordi PQ-passthrough forsterker støy mer enn SDR. */
            init.set_property("deband", "yes")?;
            init.set_property("deband-iterations", 3_i64)?;
            init.set_property("deband-range", 20_i64)?;
            init.set_property("deband-threshold", 48_i64)?;
            init.set_property("deband-grain", 8_i64)?;
            /* Full antiring (1.0) på begge skalere. På luma med ewa_lanczossharp
             * fjerner det halos rundt detaljerte kanter ved upscale; uten
             * antiring kunne sinc-respons ringe. På cscale med mitchell er
             * det no-op (mitchell ringer ikke), men holder safe default
             * dersom filter byttes. 0.7 lot ringinger fortsatt slippe gjennom. */
            init.set_property("scale-antiring", 1.0)?;
            init.set_property("cscale-antiring", 1.0)?;
            if !auth_header.is_empty() {
                init.set_property(
                    "http-header-fields",
                    format!("Authorization: {auth_header}"),
                )?;
            }
            Ok(())
        })
        .map_err(|e| anyhow!("mpv init: {e}"))?;

        let wake = Arc::new(Mutex::new(None::<egui::Context>));
        let render_wake = Arc::new((Mutex::new(false), Condvar::new()));
        let render_alive = Arc::new(Mutex::new(true));
        let hdr_meta = Arc::new(Mutex::new(None));
        let hdr_applied = Arc::new(Mutex::new(false));

        let stream_url = stream_url.to_string();
        let watchdog_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let player = MpvPlayer {
            wake,
            mpv,
            subsurface: subsurface.clone(),
            render_wake: render_wake.clone(),
            render_alive: render_alive.clone(),
            render_thread: None,
            hdr_meta: hdr_meta.clone(),
            hdr_applied,
            watchdog_alive: watchdog_alive.clone(),
            watchdog_thread: None,
            stream_url: stream_url.clone(),
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

        /* Push nominell display-fps FØR loadfile. wl_output.mode er
         * fetched i SubsurfaceVideo::new (roundtrip), så Hz er kjent
         * allerede. Uten dette antar mpv 30 fps og bom-pacer 23.976-
         * kilde under interpolation+oversample. wp_presentation_feedback
         * vil senere overskrive med ekte refresh (fra render thread). */
        if let Some(hz) = arc.subsurface.nominal_hz() {
            let _ = arc.mpv.set_property("display-fps-override", hz);
            crate::nlog!("display-fps-override (nominal pre-loadfile) = {hz:.3}");
        } else {
            crate::nlog!("nominal display-fps unknown — mpv will guess until first present");
        }

        arc.mpv
            .command("loadfile", &[&stream_url])
            .map_err(|e| anyhow!("mpv loadfile: {e}"))?;

        /* Watchdog: lytt på end-file events. EOF/ERROR på live IPTV =
         * server droppet oss og ffmpeg-reconnect ga opp. Reload URL via
         * loadfile. STOP/QUIT/REDIRECT = ignorer (bruker eller mpv internt).
         * VOD detect via duration: > 0 = ekte fil, ikke reload på natural
         * EOF. Live mpegts rapporterer duration=0 / unset. */
        let watchdog = {
            let arc_w = arc.clone();
            let alive = watchdog_alive.clone();
            let url = stream_url.clone();
            thread::Builder::new()
                .name("nixlymedia-mpv-watchdog".into())
                .spawn(move || Self::watchdog_main(arc_w, alive, url))
                .map_err(|e| anyhow!("spawn watchdog: {e}"))?
        };

        // SAFETY: we just created the Arc, only one strong ref outside thread.
        // We need to stash the handle. Use Arc::get_mut isn't safe with the clone above.
        // Instead store handle via a side channel:
        unsafe {
            let raw = Arc::as_ptr(&arc) as *mut MpvPlayer;
            (*raw).render_thread = Some(handle);
            (*raw).watchdog_thread = Some(watchdog);
        }

        Ok(arc)
    }

    fn watchdog_main(
        player: Arc<MpvPlayer>,
        alive: Arc<std::sync::atomic::AtomicBool>,
        stream_url: String,
    ) {
        use libmpv2::events::{Event, PropertyData};
        use libmpv2::Format;
        /* Egen client-handle (mpv_create_client) → vår wait_event-loop
         * blokkerer ikke andre clients. Default-handle brukes av andre
         * kallere. */
        let client = match player.mpv.create_client(Some("watchdog")) {
            Ok(c) => c,
            Err(e) => {
                crate::nlog!("watchdog: create_client failed: {e}");
                return;
            }
        };
        let _ = client.disable_deprecated_events();
        let _ = client.observe_property("duration", Format::Double, 1);

        let mut last_duration: f64 = 0.0;
        let mut consecutive_reloads: u32 = 0;
        let mut last_reload = std::time::Instant::now() - Duration::from_secs(60);

        while alive.load(std::sync::atomic::Ordering::Relaxed) {
            let ev = client.wait_event(1.0);
            if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let Some(Ok(ev)) = ev else { continue };
            match ev {
                Event::PropertyChange {
                    name: "duration",
                    change: PropertyData::Double(d),
                    ..
                } => {
                    last_duration = d;
                }
                Event::EndFile(reason) => {
                    /* Reason values from mpv_end_file_reason:
                     *   0 EOF, 2 STOP, 3 QUIT, 4 ERROR, 5 REDIRECT.
                     * STOP/QUIT/REDIRECT = bruker/internt → ikke reload. */
                    let reason_u = reason as u32;
                    let is_eof = reason_u == 0;
                    let is_err = reason_u == 4;
                    if !is_eof && !is_err {
                        crate::nlog!("watchdog: end-file reason={reason_u} ignore");
                        continue;
                    }
                    /* VOD = endelig duration. Hvis EOF og duration > 0 antar
                     * vi naturlig slutt; ikke reload. ERROR reload uansett. */
                    if is_eof && last_duration > 0.5 {
                        crate::nlog!(
                            "watchdog: EOF on VOD (duration={last_duration:.1}s) — no reload"
                        );
                        continue;
                    }
                    /* Backoff: hvis vi reloader > 5 ganger på 30s, server er
                     * nede. Stopp for å unngå retry-storm. */
                    if last_reload.elapsed() < Duration::from_secs(30) {
                        consecutive_reloads += 1;
                        if consecutive_reloads > 5 {
                            crate::nlog!(
                                "watchdog: > 5 reloads i 30s — gir opp, stream sannsynligvis nede"
                            );
                            continue;
                        }
                    } else {
                        consecutive_reloads = 0;
                    }
                    last_reload = std::time::Instant::now();
                    /* Liten delay før reload — gir TCP-laget tid til å rydde
                     * og hindrer hammer-loop ved umiddelbar feilende reload. */
                    thread::sleep(Duration::from_millis(500));
                    if !alive.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    crate::nlog!(
                        "watchdog: end-file reason={reason_u} → reload {stream_url}"
                    );
                    if let Err(e) = player.mpv.command("loadfile", &[&stream_url]) {
                        crate::nlog!("watchdog: reload failed: {e}");
                    }
                }
                Event::Shutdown => break,
                _ => {}
            }
        }
        crate::nlog!("watchdog exit");
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
            /* mpv default: render() blokkerer inntil supposed display-
             * time → limit til video FPS (libmpv2 doc). Med Wayland EGL
             * pacer eglSwapBuffers allerede til vsync, så mpv-side block
             * blir dobbeltpacing. false → render() returnerer
             * umiddelbart; vi rapporterer ekte vsync via report_swap. */
            RenderParam::BlockForTargetTime(false),
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
        let mut last_fps_push = std::time::Instant::now() - Duration::from_secs(2);
        let mut last_video_rate_check = std::time::Instant::now() - Duration::from_secs(2);
        let mut video_rate = crate::player::nixlytile_ipc::VideoRate::new();
        let mut last_pushed_hz: f64 = 0.0;
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

            /* Fortell nixlytile (compositor) hva slags refresh som passer
             * dette videosignalet. nixlytile slår på VRR om TV støtter,
             * ellers velger eksakt mode eller integer-multippel. På
             * idle/EOF restorer den max refresh. Pause = no-op (vi sender
             * ikke stopped ved pause). 500ms throttle for å fange første
             * frame-loaded transition raskt. */
            if last_video_rate_check.elapsed() > Duration::from_millis(500) {
                last_video_rate_check = std::time::Instant::now();
                let p = unsafe { &(*Arc::as_ptr(&player)).mpv };
                let idle = p.get_property::<bool>("idle-active").unwrap_or(true);
                let eof = p.get_property::<bool>("eof-reached").unwrap_or(false);
                let fps = p.get_property::<f64>("container-fps").unwrap_or(0.0);
                if !idle && !eof && fps > 0.0 {
                    if let Some(name) = sub.first_output_name() {
                        video_rate.playing(&name, fps);
                    }
                } else if idle || eof {
                    video_rate.stopped();
                }
            }

            /* Push ekte display refresh til mpv så interpolation+
             * display-resample får riktig target. Compositor rapporterer
             * refresh ns i wp_presentation_feedback.Presented. Re-push
             * kun ved endring (mode-change / output-switch) for å unngå
             * unødvendige property-set kall. 1s throttle. */
            if last_fps_push.elapsed() > Duration::from_secs(1) {
                last_fps_push = std::time::Instant::now();
                if let Some(hz) = sub.display_hz() {
                    if (hz - last_pushed_hz).abs() > 0.05 {
                        let p = unsafe { &(*Arc::as_ptr(&player)).mpv };
                        let _ = p.set_property("display-fps-override", hz);
                        last_pushed_hz = hz;
                        crate::nlog!("display-fps-override = {hz:.3}");
                    }
                }
            }
        }

        /* Sørg for at TV faller tilbake til max refresh når appen lukker
         * eller render-thread avslutter av andre grunner. */
        video_rate.stopped();
        /* Drop render context FØR release_current. mpv_render_context_free
         * kaller cuGraphicsUnregisterResource på CUDA-GL-interop ressurser,
         * som krever current GL-kontekst. Uten dette: CUDA_ERROR_INVALID_
         * GRAPHICS_CONTEXT, NVDEC-ressurser lekker, GPU-clocks låst høyt,
         * vifter 100%. */
        drop(render);
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

    /* Hopper til nærmeste keyframe i stedet for eksakt frame. mpv kan da
     * skippe demuxer-resync og frame-by-frame decoding mellom mål og
     * keyframe. Brukes ved hold-spoling der visuell smidighet er
     * mindre viktig enn lav latens og rask resume. */
    pub fn seek_keyframe(&self, secs: f64) {
        let s = format!("{secs:.2}");
        let _ = self.mpv.command("seek", &[&s, "relative+keyframes"]);
    }

    pub fn audio_delay(&self) -> f64 {
        self.mpv.get_property::<f64>("audio-delay").unwrap_or(0.0)
    }

    /* Endrer audio-delay med delta og returnerer ny verdi. Positivt delta
     * forsinker lyd mer (kompenserer for lyd som ligger foran video).
     * Clamper til ±2 s for å unngå utilsiktet ekstrem-verdi ved tastefeil. */
    pub fn adjust_audio_delay(&self, delta: f64) -> f64 {
        let cur = self.audio_delay();
        let new_val = (cur + delta).clamp(-2.0, 2.0);
        let _ = self.mpv.set_property("audio-delay", new_val);
        new_val
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
        /* Watchdog OFF før stop-cmd. Ellers ser den end-file reason=STOP
         * og ignorerer, men hvis bruker stopper presist samme øyeblikk
         * som server-EOF kommer, kunne den ha reloadet. Sett flag først. */
        self.watchdog_alive
            .store(false, std::sync::atomic::Ordering::Relaxed);
        /* Be nixlytile restore max refresh FØR vi river ned. Render-tråden
         * har også en stopped()-fallback ved exit, men på bruker-stop blir
         * shutdown signalert umiddelbart etter command("stop"), så
         * 500ms-pollingen rekker ikke å se idle-active=true. Da kan
         * last_sent være stale og fallback-en blir no-op. Synkron send
         * her fjerner racet — IPC fyrer alltid på user-stop. */
        if let Some(name) = self.subsurface.first_output_name() {
            crate::player::nixlytile_ipc::send_video_stopped(&name);
        }
        let _ = self.mpv.command("stop", &[]);
        /* Frigjør demuxer-cache umiddelbart. "stop" alene avslutter
         * playback men beholder typisk demuxer-buffer i påvente av
         * neste loadfile (16 GiB cap kan henge igjen i RAM til Drop).
         * playlist-clear fjerner alle entries; cache=no-toggle tvinger
         * libmpv til å rive demuxer ned og slippe back/forward-bytes
         * tilbake til OS. Re-enable cache=yes så neste loadfile finner
         * den i samme tilstand som ved init. */
        let _ = self.mpv.command("playlist-clear", &[]);
        let _ = self.mpv.set_property("cache", "no");
        let _ = self.mpv.set_property("cache", "yes");
    }

    pub fn shutdown(&self) {
        *self.render_alive.lock() = false;
        self.watchdog_alive
            .store(false, std::sync::atomic::Ordering::Relaxed);
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
        if let Some(h) = self.watchdog_thread.take() {
            let _ = h.join();
        }
    }
}
