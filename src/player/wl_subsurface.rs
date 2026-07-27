use std::ffi::c_void;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use khronos_egl as egl;
use parking_lot::Mutex;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_callback::{self, WlCallback},
    wl_compositor::WlCompositor, wl_region::WlRegion,
    wl_registry, wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_egl::WlEglSurface;
use wayland_protocols::wp::content_type::v1::client::{
    wp_content_type_manager_v1::WpContentTypeManagerV1,
    wp_content_type_v1::{Type as ContentType, WpContentTypeV1},
};
use wayland_protocols::wp::presentation_time::client::{
    wp_presentation::{self, WpPresentation},
    wp_presentation_feedback::{self, WpPresentationFeedback},
};

use crate::player::wl_color::{CmState, ColorMgr};

#[allow(dead_code)]
pub struct SubState {
    pub compositor: Option<WlCompositor>,
    pub subcompositor: Option<WlSubcompositor>,
    pub presentation: Option<WpPresentation>,
    /* Antall frames bekreftet presented av compositor siden forrige
     * take_presented(). Brukes til å pace mpv via report_swap. */
    pub presented_count: u64,
    pub discarded_count: u64,
    /* Display refresh-intervall i nanosekund fra siste Presented-event.
     * Compositor rapporterer eksakt tid mellom presented frame og neste
     * vsync — autoritativ display-fps-kilde. 0 = ikke målt enda. */
    pub refresh_ns: u32,
    /* Høyeste serial mottatt fra wl_surface.frame-callbacks. Render-
     * tråden venter på vblank etter commit ved å sammenligne mot
     * serialen request_frame() ga — report_swap skal skje ved ekte
     * flip, ikke ved eglSwapBuffers-retur (swap-interval 0 returnerer
     * umiddelbart). Serial (ikke bool) så en SEN callback fra en
     * tidligere timet-ut frame ikke kan feilkvittere neste frame og
     * gi report_swap på feil tidspunkt. */
    pub frame_done_serial: u64,
}

#[allow(dead_code)]
pub struct SubsurfaceVideo {
    pub conn: Connection,
    pub queue: Arc<Mutex<EventQueue<SubState>>>,
    pub qh: QueueHandle<SubState>,
    pub state: Arc<Mutex<SubState>>,

    pub wl_display_ptr: *mut c_void,

    pub parent_surface: WlSurface,
    pub child_surface: WlSurface,
    pub subsurface: WlSubsurface,

    pub wl_egl: Mutex<WlEglSurface>,
    pub egl: Arc<egl::DynamicInstance<egl::EGL1_5>>,
    pub egl_display: egl::Display,
    pub egl_config: egl::Config,
    pub egl_context: egl::Context,
    pub egl_surface: Mutex<egl::Surface>,

    /* Bits per fargekanal i valgt EGL-config (10 = AR30, 8 = RGBA8).
     * mpv setter dither-depth eksplisitt fra denne — auto-deteksjon mot
     * fbo 0 er upålitelig under libmpv render API. */
    pub color_bits: i32,

    pub width: Mutex<i32>,
    pub height: Mutex<i32>,

    /* wp_content_type_v1 VIDEO på både child (mpv) og parent (egui-
     * toplevel). nixlytile klassifiserer fullscreen-klienten som video
     * ut fra denne taggen — uten den treffer compositorens idle-gate
     * (1 Hz frame_done) og buffer-release stopper → eglSwapBuffers
     * blokkerer ~0.5s per frame. Destroyes i Drop så taggen fjernes
     * når avspilling stopper. */
    pub content_child: Option<WpContentTypeV1>,
    pub content_parent: Option<WpContentTypeV1>,

    pub color: Mutex<Option<ColorMgr>>,

    /* DRM-driver (i915/xe/nvidia/amdgpu) som backer EGL-displayet, fra
     * EGL_EXT_device_query. På hybrid-GPU (laptop: panel på i915 +
     * nvidia dGPU) er sysfs-scan misvisende — Wayland-EGL renderer på
     * compositorens GPU. Styrer hwdec-valg og shader-profil i mpv-init.
     * None = extension mangler; hwdec faller tilbake til sysfs-scan. */
    pub render_driver: Option<String>,

    /* Output-info fra egen wayland-connection (se wl_outputs.rs — å
     * binde wl_output på winits connection panic'er winit). Snapshot
     * fra init; ekte refresh kommer via wp_presentation_feedback. */
    pub nominal_refresh_mhz: i32,
    /* Navnet på skjermen KUN når maskinen har én. Med flere sier
     * registry-rekkefølgen ingenting om hvilken videoen ligger på, og da
     * lar vi nixlytile finne den selv (IPC output="auto"). */
    pub sole_output_name: Option<String>,

    /* Monotont økende id som tagges på hver frame-callback (udata).
     * Se frame_done_serial i SubState. */
    pub frame_serial: std::sync::atomic::AtomicU64,

    /* Serialiserer alle child_surface.commit()-kall mellom render-tråden
     * (eglSwapBuffers → wl_surface.attach + commit + flush) og main-tråden
     * (HDR-CM-state attach/detach + commit). Uten dette kan main-tråden
     * commite pending CM-state samtidig som render-tråden commiter ny
     * buffer → protokoll-race der compositor (wlroots) kan diskarte
     * frames eller bli i inkonsistent state = svart skjerm / freeze. */
    pub commit_lock: Mutex<()>,
}

unsafe impl Send for SubsurfaceVideo {}
unsafe impl Sync for SubsurfaceVideo {}

/* DRM-driver-navn for GPU-en bak EGL-displayet, via EGL_EXT_device_query
 * → DRM-node → /sys/class/drm/<node>/device/driver-symlink. Prøver
 * render-node (renderD*) først, så primary (card*). */
fn detect_render_driver(
    egl: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
) -> Option<String> {
    const EGL_DEVICE_EXT: i32 = 0x322C;
    const EGL_DRM_DEVICE_FILE_EXT: i32 = 0x3233;
    const EGL_DRM_RENDER_NODE_FILE_EXT: i32 = 0x3377;
    type QueryDisplayAttribExt = extern "system" fn(*mut c_void, i32, *mut isize) -> u32;
    type QueryDeviceStringExt =
        extern "system" fn(*mut c_void, i32) -> *const std::os::raw::c_char;

    let qda: QueryDisplayAttribExt =
        unsafe { std::mem::transmute(egl.get_proc_address("eglQueryDisplayAttribEXT")?) };
    let qds: QueryDeviceStringExt =
        unsafe { std::mem::transmute(egl.get_proc_address("eglQueryDeviceStringEXT")?) };

    let mut dev: isize = 0;
    if qda(display.as_ptr(), EGL_DEVICE_EXT, &mut dev) == 0 || dev == 0 {
        return None;
    }
    let node = [EGL_DRM_RENDER_NODE_FILE_EXT, EGL_DRM_DEVICE_FILE_EXT]
        .iter()
        .find_map(|&name| {
            let p = qds(dev as *mut c_void, name);
            if p.is_null() {
                None
            } else {
                Some(unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned())
            }
        })?;
    let base = std::path::Path::new(&node).file_name()?.to_str()?.to_string();
    let drv = std::fs::read_link(format!("/sys/class/drm/{base}/device/driver")).ok()?;
    Some(drv.file_name()?.to_str()?.to_string())
}

/* Nvidia (hybrid-laptop) returnerer EGL_FALSE fra eglChooseConfig UTEN å
 * sette error når ingen configs matcher (spec sier TRUE + num_config=0).
 * khronos-egl gjør get_error().unwrap() på FALSE-pathen → panic (None ved
 * EGL_SUCCESS). Kall derfor eglChooseConfig rått og behandle FALSE som
 * "ingen match" så 8-bit-fallbacken faktisk nås. */
fn choose_first_config_safe(
    egl: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    attrs: &[i32],
) -> Option<egl::Config> {
    type ChooseConfig =
        extern "system" fn(*mut c_void, *const i32, *mut *mut c_void, i32, *mut i32) -> u32;
    let f: ChooseConfig =
        unsafe { std::mem::transmute(egl.get_proc_address("eglChooseConfig")?) };
    let mut cfg: *mut c_void = std::ptr::null_mut();
    let mut count: i32 = 0;
    if f(display.as_ptr(), attrs.as_ptr(), &mut cfg, 1, &mut count) == 0
        || count < 1
        || cfg.is_null()
    {
        return None;
    }
    Some(unsafe { egl::Config::from_ptr(cfg) })
}

impl SubsurfaceVideo {
    pub unsafe fn new(
        wl_display_ptr: *mut c_void,
        parent_surface_ptr: *mut c_void,
        width: i32,
        height: i32,
    ) -> Result<Self> {
        if wl_display_ptr.is_null() || parent_surface_ptr.is_null() {
            return Err(anyhow!("null wayland pointers"));
        }
        let backend = Backend::from_foreign_display(wl_display_ptr.cast());
        let conn = Connection::from_backend(backend);

        let (globals, queue) = registry_queue_init::<SubState>(&conn)
            .map_err(|e| anyhow!("registry: {e}"))?;
        let qh = queue.handle();

        let mut compositor: Option<WlCompositor> = None;
        let mut subcompositor: Option<WlSubcompositor> = None;
        let mut presentation: Option<WpPresentation> = None;
        let mut content_type_mgr: Option<WpContentTypeManagerV1> = None;
        for g in globals.contents().clone_list() {
            match g.interface.as_str() {
                "wl_compositor" => {
                    compositor = Some(globals.registry().bind::<WlCompositor, _, _>(
                        g.name,
                        g.version.min(4),
                        &qh,
                        (),
                    ));
                }
                "wl_subcompositor" => {
                    subcompositor = Some(globals.registry().bind::<WlSubcompositor, _, _>(
                        g.name,
                        1,
                        &qh,
                        (),
                    ));
                }
                "wp_presentation" => {
                    presentation = Some(globals.registry().bind::<WpPresentation, _, _>(
                        g.name,
                        g.version.min(1),
                        &qh,
                        (),
                    ));
                }
                "wp_content_type_manager_v1" => {
                    content_type_mgr =
                        Some(globals.registry().bind::<WpContentTypeManagerV1, _, _>(
                            g.name,
                            1,
                            &qh,
                            (),
                        ));
                }
                /* wl_output bindes BEVISST IKKE her — se wl_outputs.rs. */
                _ => {}
            }
        }
        let compositor = compositor.ok_or_else(|| anyhow!("no wl_compositor"))?;
        let subcompositor = subcompositor.ok_or_else(|| anyhow!("no wl_subcompositor"))?;

        let parent_id = ObjectId::from_ptr(WlSurface::interface(), parent_surface_ptr.cast())
            .map_err(|_| anyhow!("invalid parent surface proxy"))?;
        let parent_surface = WlSurface::from_id(&conn, parent_id)
            .map_err(|_| anyhow!("parent surface from id"))?;

        let child_surface = compositor.create_surface(&qh, ());
        let subsurface = subcompositor.get_subsurface(&child_surface, &parent_surface, &qh, ());
        subsurface.set_desync();
        subsurface.set_position(0, 0);
        subsurface.place_below(&parent_surface);

        /* Set opaque region på child surface. EGL-config har alpha-kanal
         * (RGBA8 her, RGB10_A2 ved HDR-path). Uten opaque region antar
         * kompositoren surface har semi-transparent alpha → blender mot
         * bakgrunn. Mpv-render fyller ikke alpha-bittene meningsfullt
         * (skriver typisk 0) → svart skjerm på HDR/UHD TV.
         * i32::MAX dekker enhver fremtidig resize. */
        let region = compositor.create_region(&qh, ());
        region.add(0, 0, i32::MAX, i32::MAX);
        child_surface.set_opaque_region(Some(&region));
        region.destroy();

        /* Merk begge surfaces som VIDEO — child bærer mpv-bildet, parent
         * er toplevel-surfacet nixlytile leser content-type fra. State er
         * double-buffered; child commites rett under, parent på neste
         * egui-redraw. */
        let (content_child, content_parent) = match &content_type_mgr {
            Some(mgr) => {
                let cc = mgr.get_surface_content_type(&child_surface, &qh, ());
                cc.set_content_type(ContentType::Video);
                let cp = mgr.get_surface_content_type(&parent_surface, &qh, ());
                cp.set_content_type(ContentType::Video);
                (Some(cc), Some(cp))
            }
            None => (None, None),
        };
        crate::nlog!(
            "wp_content_type_manager_v1: {}",
            if content_type_mgr.is_some() { "bound (video tagged)" } else { "absent" }
        );

        let egl = Arc::new(
            egl::DynamicInstance::<egl::EGL1_5>::load_required()
                .map_err(|e| anyhow!("load libEGL: {e}"))?,
        );
        egl.bind_api(egl::OPENGL_API)
            .map_err(|e| anyhow!("eglBindAPI: {e}"))?;

        let egl_display = unsafe { egl.get_display(wl_display_ptr) }
            .ok_or_else(|| anyhow!("eglGetDisplay returned null"))?;
        egl.initialize(egl_display)
            .map_err(|e| anyhow!("eglInitialize: {e}"))?;

        let render_driver = detect_render_driver(&egl, egl_display);
        crate::nlog!(
            "EGL render device: {}",
            render_driver.as_deref().unwrap_or("ukjent (EGL_EXT_device_query mangler)")
        );

        /* 10-bit (AR30) client surface når driver+compositor støtter det —
         * 10-bit HDR/PQ når da skjermen uten 8-bit-kvantisering (banding).
         * EGL Wayland-platform eksponerer kun configs compositoren faktisk
         * aksepterer (gjelder både Mesa/AMD/Intel og NVIDIA), så et treff
         * her er trygt å bruke. Fallback: 8-bit RGBA8 + error-diffusion
         * dither i mpv. mpv får valgt dybde via color_bits()/dither-depth. */
        let attrs10 = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::RED_SIZE,
            10,
            egl::GREEN_SIZE,
            10,
            egl::BLUE_SIZE,
            10,
            egl::ALPHA_SIZE,
            2,
            egl::NONE,
        ];
        let attrs8 = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::NONE,
        ];
        let egl_config = choose_first_config_safe(&egl, egl_display, &attrs10)
            .or_else(|| {
                crate::nlog!("ingen 10-bit EGL config, faller tilbake til 8-bit");
                choose_first_config_safe(&egl, egl_display, &attrs8)
            })
            .ok_or_else(|| anyhow!("no EGL config (verken 10-bit eller 8-bit)"))?;
        /* Les reell dybde fra valgt config (≥-matching kan gi mer enn
         * forespurt) — denne styrer mpv dither-depth. */
        let color_bits = egl
            .get_config_attrib(egl_display, egl_config, egl::RED_SIZE)
            .unwrap_or(8);
        crate::nlog!(
            "subsurface EGL config: {color_bits}-bit per kanal (HDR via wp_color_manager_v1)"
        );

        /* GL 4.6 core. 3.3 ga oss compute shaders=0 → mpv disabled
         * hdr-compute-peak + error-diffusion dither (libplacebo features
         * krever GL 4.3+ for compute shaders, SSBO, image_load_store).
         * 2080 Ti og Intel UHD 6th-gen+ støtter 4.6 trivielt.
         * Fall tilbake til 3.3 hvis 4.6 feiler — gammel hardware. */
        let ctx_attrs_46 = [
            egl::CONTEXT_MAJOR_VERSION,
            4,
            egl::CONTEXT_MINOR_VERSION,
            6,
            egl::CONTEXT_OPENGL_PROFILE_MASK,
            egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        let egl_context = match egl.create_context(egl_display, egl_config, None, &ctx_attrs_46) {
            Ok(c) => {
                crate::nlog!("EGL context: GL 4.6 core");
                c
            }
            Err(e46) => {
                crate::nlog!("EGL 4.6 failed ({e46}), fallback 3.3 core");
                let ctx_attrs_33 = [
                    egl::CONTEXT_MAJOR_VERSION,
                    3,
                    egl::CONTEXT_MINOR_VERSION,
                    3,
                    egl::CONTEXT_OPENGL_PROFILE_MASK,
                    egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
                    egl::NONE,
                ];
                egl.create_context(egl_display, egl_config, None, &ctx_attrs_33)
                    .map_err(|e| anyhow!("eglCreateContext: {e}"))?
            }
        };

        let wl_egl = WlEglSurface::new(child_surface.id(), width.max(1), height.max(1))
            .map_err(|e| anyhow!("WlEglSurface: {e}"))?;
        let egl_surface = unsafe {
            egl.create_window_surface(
                egl_display,
                egl_config,
                wl_egl.ptr() as egl::NativeWindowType,
                None,
            )
        }
        .map_err(|e| anyhow!("eglCreateWindowSurface: {e}"))?;

        child_surface.commit();
        let _ = conn.flush();

        let has_presentation = presentation.is_some();
        let mut queue = queue;
        let mut tmp_state = SubState {
            compositor: Some(compositor),
            subcompositor: Some(subcompositor),
            presentation,
            presented_count: 0,
            discarded_count: 0,
            refresh_ns: 0,
            frame_done_serial: 0,
        };
        /* Pump pending events (registry/presentation clock) før state
         * gis fra oss. */
        let _ = queue.roundtrip(&mut tmp_state);
        let state = Arc::new(Mutex::new(tmp_state));
        let queue = Arc::new(Mutex::new(queue));

        /* Nominell refresh via egen connection (roundtrip inni query).
         * Tilgjengelig FØR loadfile → mpv får display-fps fra start. */
        let outputs_info = crate::player::wl_outputs::query();
        let sole_output_name = if outputs_info.len() == 1 {
            outputs_info[0].name.clone()
        } else {
            None
        };
        let nominal_mhz = outputs_info
            .iter()
            .map(|o| o.refresh_mhz)
            .find(|&m| m > 0)
            .unwrap_or(0);
        crate::nlog!(
            "wp_presentation: {}, wl_outputs: {}, nominal_hz: {:.3}",
            if has_presentation { "bound" } else { "absent" },
            outputs_info.len(),
            if nominal_mhz > 0 { nominal_mhz as f64 / 1000.0 } else { 0.0 }
        );

        Ok(Self {
            conn,
            queue,
            qh,
            state,
            wl_display_ptr,
            parent_surface,
            child_surface,
            subsurface,
            wl_egl: Mutex::new(wl_egl),
            egl,
            egl_display,
            egl_config,
            egl_context,
            egl_surface: Mutex::new(egl_surface),
            color_bits,
            width: Mutex::new(width.max(1)),
            height: Mutex::new(height.max(1)),
            content_child,
            content_parent,
            color: Mutex::new(None),
            render_driver,
            nominal_refresh_mhz: nominal_mhz,
            sole_output_name,
            frame_serial: std::sync::atomic::AtomicU64::new(0),
            commit_lock: Mutex::new(()),
        })
    }

    pub fn make_current(&self) -> Result<()> {
        let surf = *self.egl_surface.lock();
        self.egl
            .make_current(
                self.egl_display,
                Some(surf),
                Some(surf),
                Some(self.egl_context),
            )
            .map_err(|e| anyhow!("eglMakeCurrent: {e}"))
    }

    pub fn release_current(&self) -> Result<()> {
        self.egl
            .make_current(self.egl_display, None, None, None)
            .map_err(|e| anyhow!("eglMakeCurrent release: {e}"))
    }

    pub fn swap_buffers(&self) -> Result<()> {
        /* eglSwapBuffers internally attacher buffer + commiter child_surface.
         * Holder commit_lock så main-tråd ikke kan injecte sin egen commit
         * mellom attach og flush. */
        let _g = self.commit_lock.lock();
        let surf = *self.egl_surface.lock();
        self.egl
            .swap_buffers(self.egl_display, surf)
            .map_err(|e| anyhow!("eglSwapBuffers: {e}"))?;
        let _ = self.conn.flush();
        Ok(())
    }

    pub fn resize(&self, w: i32, h: i32) {
        let w = w.max(1);
        let h = h.max(1);
        if *self.width.lock() == w && *self.height.lock() == h {
            return;
        }
        /* Serialiser mot render-trådens eglSwapBuffers. wl_egl_window_resize
         * midt i en swap kan gi inkonsistent buffer-geometri. */
        let _g = self.commit_lock.lock();
        *self.width.lock() = w;
        *self.height.lock() = h;
        self.wl_egl.lock().resize(w, h, 0, 0);
    }

    /* eglSwapInterval. 0 = swap_buffers blokkerer aldri på compositor
     * frame-callbacks. Må kalles fra tråden der konteksten er current. */
    pub fn set_swap_interval(&self, interval: i32) {
        if let Err(e) = self.egl.swap_interval(self.egl_display, interval) {
            crate::nlog!("eglSwapInterval({interval}): {e}");
        }
    }

    pub fn set_position(&self, x: i32, y: i32) {
        self.subsurface.set_position(x, y);
        self.parent_surface.commit();
    }

    pub fn dimensions(&self) -> (i32, i32) {
        (*self.width.lock(), *self.height.lock())
    }

    /* Be om presentation feedback for nåværende commit. Må kalles FØR
     * eglSwapBuffers (som committer surface). presented-event mottas
     * senere — count drains via take_presented(). */
    pub fn request_presentation_feedback(&self) {
        let s = self.state.lock();
        if let Some(pres) = &s.presentation {
            pres.feedback(&self.child_surface, &self.qh, ());
        }
    }

    /* Be om frame-callback for neste commit. Kalles FØR eglSwapBuffers
     * (frame-request er double-buffered surface-state). Returnerer
     * serialen callbacken tagges med — gi den til wait_frame_done. */
    pub fn request_frame(&self) -> u64 {
        let s = self
            .frame_serial
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        self.child_surface.frame(&self.qh, s);
        s
    }

    /* Vent til compositor melder vblank (frame-callback Done med denne
     * serialen) for siste commit, maks `timeout`. Returnerer true ved
     * ekte vblank, false ved timeout. Timeout er kritisk: under
     * nixlytile mode-switch/VRR-engasjement uteblir callbacks —
     * blocking uten timeout ga tidligere frys av hele render-tråden.
     *
     * Poll-en skjer med lese-intent (prepare_read-guard) HOLDT. wl_display
     * deles med winit/egui på main-tråden; uten intent kan main-tråden
     * konsumere socket-data slik at callbacken vår legges i køen vår
     * uten at fd-en noensinne får mer data — vi sov da hele timeouten
     * og report_swap kom en vsync for sent eller uteble. Det ga målt
     * ~0.5-1 frame-drop/s (drop/vo_delay-klatring + av_diff-hopp på
     * ±1 vsync i stats-loggen) = konstant hakking. prepare_read()==None
     * betyr at backend alt har uleste events → loop og dispatch. */
    pub fn wait_frame_done(&self, serial: u64, timeout: std::time::Duration) -> bool {
        use std::os::fd::{AsFd, AsRawFd};
        let deadline = std::time::Instant::now() + timeout;
        loop {
            {
                let mut q = self.queue.lock();
                let mut s = self.state.lock();
                let _ = q.dispatch_pending(&mut s);
                let _ = q.flush();
                if s.frame_done_serial >= serial {
                    return true;
                }
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Some(guard) = self.conn.prepare_read() else {
                /* Events ligger alt i backend-køen — dispatch dem. */
                continue;
            };
            let mut pfd = libc::pollfd {
                fd: self.conn.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ms = (remaining.as_millis() as i32).max(1);
            let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
            if ready > 0 && (pfd.revents & libc::POLLIN) != 0 {
                let _ = guard.read();
            } else {
                drop(guard);
                if ready < 0 {
                    return false;
                }
            }
        }
    }

    /* Returnerer antall confirmed-presented frames siden forrige kall
     * og nullstiller telleren. Kun stats — pacing skjer via
     * wait_frame_done + report_swap. */
    pub fn take_presented(&self) -> u64 {
        let mut s = self.state.lock();
        let n = s.presented_count;
        s.presented_count = 0;
        n
    }

    pub fn take_discarded(&self) -> u64 {
        let mut s = self.state.lock();
        let n = s.discarded_count;
        s.discarded_count = 0;
        n
    }

    /* Display refresh-Hz fra siste compositor Presented-event, eller
     * None om ingen presented mottatt enda. Konvertert fra ns → Hz. */
    pub fn display_hz(&self) -> Option<f64> {
        let ns = self.state.lock().refresh_ns;
        if ns == 0 {
            None
        } else {
            Some(1_000_000_000.0 / ns as f64)
        }
    }

    /* Nominell refresh-Hz fra wl_output.mode (snapshot fra init) —
     * brukes til å initialisere mpv display-fps FØR loadfile, så
     * interpolation+display-resample får riktig target fra første
     * frame. None hvis ingen wl_output mode mottatt. */
    pub fn nominal_hz(&self) -> Option<f64> {
        if self.nominal_refresh_mhz <= 0 {
            None
        } else {
            Some(self.nominal_refresh_mhz as f64 / 1000.0)
        }
    }

    pub fn get_proc(&self, name: &str) -> *mut c_void {
        match self.egl.get_proc_address(name) {
            Some(p) => p as *mut c_void,
            None => std::ptr::null_mut(),
        }
    }

    pub fn child_wl_surface(&self) -> &WlSurface {
        &self.child_surface
    }

    pub fn pump(&self) {
        /* Les compositor→klient events non-blocking FØR dispatch. mpv-
         * render-tråden lager et wp_presentation_feedback-objekt hver frame;
         * Presented/Discarded er destructor-events som reaper objektet. Under
         * avspilling er egui/winit-loopen idle (ingen request_repaint, se
         * App::update), så host-loopen leser ikke wayland-fd-en. Uten egen
         * read her blir feedback-events aldri lest inn → objektene aldri
         * reaped → compositorens per-klient send-kø vokser ubegrenset på
         * tvers av plays → freeze (før kun fikset av nixlytile-restart).
         * prepare_read + poll(timeout=0) leser kun når data finnes, så
         * render-tråden aldri blokkerer. */
        use std::os::fd::{AsFd, AsRawFd};
        let _ = self.conn.flush();
        while let Some(guard) = self.conn.prepare_read() {
            let mut pfd = libc::pollfd {
                fd: self.conn.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, 0) };
            if ready <= 0 || (pfd.revents & libc::POLLIN) == 0 {
                /* Ingen ventende data — slipp guard (kansellerer read). */
                break;
            }
            if guard.read().is_err() {
                break;
            }
        }
        let mut q = self.queue.lock();
        let mut s = self.state.lock();
        let _ = q.dispatch_pending(&mut s);
        let _ = q.flush();
    }

    /* Kjør en compound-operasjon (set_image_description / unset → commit
     * → flush) atomisk mot render-tråden. Hindrer at swap_buffers sniker
     * inn en commit mellom CM-state-pending og vår commit. Lukket form
     * — kall sub.child_surface.commit() og sub.conn.flush() inne i
     * closuren, ikke et annet self-method (parking_lot Mutex er ikke
     * reentrant). */
    pub fn with_commit_lock<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let _g = self.commit_lock.lock();
        f()
    }
}

impl Drop for SubsurfaceVideo {
    fn drop(&mut self) {
        /* Unbind KUN hvis vår context er current på denne tråden. Drop
         * kjører på main-tråden der eframe/glutin sin GL-context er
         * current; en ubetinget make_current(None) river ned DEN — alle
         * påfølgende egui-GL-kall går til no-op dispatch (svart/frossen
         * UI) og neste eglSwapBuffers feiler → crash ved stop/neste play.
         * eglDestroySurface/Context krever ikke current context. */
        if self.egl.get_current_context() == Some(self.egl_context) {
            let _ = self.egl.make_current(self.egl_display, None, None, None);
        }
        let _ = self
            .egl
            .destroy_surface(self.egl_display, *self.egl_surface.lock());
        let _ = self
            .egl
            .destroy_context(self.egl_display, self.egl_context);
        /* Destroy wayland-objektene. Uten dette lekker child_surface +
         * wl_subsurface per play — de blir stående mappet under parent
         * med siste frame til prosess-exit, og compositor akkumulerer
         * surfaces for hver avspilling.
         * Content-type-objektene FØRST: destroy resetter taggen til NONE
         * (parent-surfacet lever videre og skal ikke være "video" i
         * menyene), og en ny play må kunne get_surface_content_type på
         * parent igjen uten already_constructed-protokollfeil. */
        if let Some(cc) = &self.content_child {
            cc.destroy();
        }
        if let Some(cp) = &self.content_parent {
            cp.destroy();
        }
        self.subsurface.destroy();
        self.child_surface.destroy();
        let _ = self.conn.flush();
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for SubState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for SubState {
    fn event(_: &mut Self, _: &WlCompositor, _: <WlCompositor as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSubcompositor, ()> for SubState {
    fn event(_: &mut Self, _: &WlSubcompositor, _: <WlSubcompositor as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSurface, ()> for SubState {
    fn event(_: &mut Self, _: &WlSurface, _: <WlSurface as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlSubsurface, ()> for SubState {
    fn event(_: &mut Self, _: &WlSubsurface, _: <WlSubsurface as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlRegion, ()> for SubState {
    fn event(_: &mut Self, _: &WlRegion, _: <WlRegion as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpContentTypeManagerV1, ()> for SubState {
    fn event(_: &mut Self, _: &WpContentTypeManagerV1, _: <WpContentTypeManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpContentTypeV1, ()> for SubState {
    fn event(_: &mut Self, _: &WpContentTypeV1, _: <WpContentTypeV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WpPresentation, ()> for SubState {
    fn event(
        _: &mut Self,
        _: &WpPresentation,
        _: wp_presentation::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        /* clock_id event ignoreres — vi bruker ikke timestamps,
         * kun presented/discarded count. */
    }
}

impl Dispatch<WpPresentationFeedback, ()> for SubState {
    fn event(
        state: &mut Self,
        _: &WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wp_presentation_feedback::Event;
        /* Feedback-objektet er server-side one-shot: serveren destroyer
         * det etter presented eller discarded. Client-side proxy ryddes
         * automatisk av wayland-rs. */
        match event {
            Event::Presented { refresh, .. } => {
                state.presented_count = state.presented_count.saturating_add(1);
                /* refresh=0 betyr "ukjent" (compositor støtter ikke å
                 * rapportere det). Behold forrige målte verdi. */
                if refresh != 0 {
                    state.refresh_ns = refresh;
                }
            }
            /* Discarded = frame aldri vist — teller IKKE som presented.
             * Å telle den ga inflatert presented-stat. */
            Event::Discarded => {
                state.discarded_count = state.discarded_count.saturating_add(1);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, u64> for SubState {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        serial: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frame_done_serial = state.frame_done_serial.max(*serial);
        }
    }
}

// silence unused warning when CM not yet wired
#[allow(dead_code)]
fn _phantom_cm(_: &CmState) {}
