use std::ffi::c_void;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use khronos_egl as egl;
use parking_lot::Mutex;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_output::{self, WlOutput}, wl_registry,
    wl_subcompositor::WlSubcompositor, wl_subsurface::WlSubsurface,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_egl::WlEglSurface;
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
    /* Display refresh-intervall i nanosekund fra siste Presented-event.
     * Compositor rapporterer eksakt tid mellom presented frame og neste
     * vsync — autoritativ display-fps-kilde. 0 = ikke målt enda. */
    pub refresh_ns: u32,
    /* wl_output proxies — hold liv så mode-events kommer inn. */
    pub outputs: Vec<WlOutput>,
    /* Nominell refresh i mHz fra første wl_output.mode (Current-flag).
     * Tilgjengelig FØR første swap — brukes til å push display-fps-
     * override før loadfile. 0 = ikke mottatt enda. */
    pub nominal_refresh_mhz: i32,
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

    pub width: Mutex<i32>,
    pub height: Mutex<i32>,

    pub color: Mutex<Option<ColorMgr>>,
}

unsafe impl Send for SubsurfaceVideo {}
unsafe impl Sync for SubsurfaceVideo {}

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
        let mut outputs: Vec<WlOutput> = Vec::new();
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
                "wl_output" => {
                    outputs.push(globals.registry().bind::<WlOutput, _, _>(
                        g.name,
                        g.version.min(4),
                        &qh,
                        (),
                    ));
                }
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

        // 8-bit RGBA8 client surface. HDR scanout-format velges av compositor
        // via wp_color_manager_v1 image description (se wl_color.rs).
        // NVIDIA Wayland EGLStream eksponerer ikke 10-bit configs uansett.
        let attrs = [
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
        let egl_config = egl
            .choose_first_config(egl_display, &attrs)
            .map_err(|e| anyhow!("eglChooseConfig: {e}"))?
            .ok_or_else(|| anyhow!("no EGL config"))?;
        crate::nlog!("subsurface EGL config: 8-bit RGBA8 (HDR via wp_color_manager_v1)");

        let ctx_attrs = [
            egl::CONTEXT_MAJOR_VERSION,
            3,
            egl::CONTEXT_MINOR_VERSION,
            3,
            egl::CONTEXT_OPENGL_PROFILE_MASK,
            egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        let egl_context = egl
            .create_context(egl_display, egl_config, None, &ctx_attrs)
            .map_err(|e| anyhow!("eglCreateContext: {e}"))?;

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
        let n_outputs = outputs.len();
        let mut queue = queue;
        let mut tmp_state = SubState {
            compositor: Some(compositor),
            subcompositor: Some(subcompositor),
            presentation,
            presented_count: 0,
            refresh_ns: 0,
            outputs,
            nominal_refresh_mhz: 0,
        };
        /* Roundtrip pumper wl_output.geometry/mode/done før vi gir fra
         * oss state — mpv kan da få display-fps før loadfile. */
        let _ = queue.roundtrip(&mut tmp_state);
        let nominal_mhz = tmp_state.nominal_refresh_mhz;
        let state = Arc::new(Mutex::new(tmp_state));
        let queue = Arc::new(Mutex::new(queue));
        crate::nlog!(
            "wp_presentation: {}, wl_outputs: {n_outputs}, nominal_hz: {:.3}",
            if has_presentation { "bound" } else { "absent" },
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
            width: Mutex::new(width.max(1)),
            height: Mutex::new(height.max(1)),
            color: Mutex::new(None),
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
        *self.width.lock() = w;
        *self.height.lock() = h;
        self.wl_egl.lock().resize(w, h, 0, 0);
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

    /* Returnerer antall confirmed-presented frames siden forrige kall
     * og nullstiller telleren. Mpv bruker dette til å derivere fps. */
    pub fn take_presented(&self) -> u64 {
        let mut s = self.state.lock();
        let n = s.presented_count;
        s.presented_count = 0;
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

    /* Nominell refresh-Hz fra wl_output.mode. Tilgjengelig fra
     * SubsurfaceVideo::new — brukes til å initialisere mpv display-fps
     * FØR loadfile, så interpolation+display-resample får riktig target
     * fra første frame. None hvis ingen wl_output mode mottatt. */
    pub fn nominal_hz(&self) -> Option<f64> {
        let m = self.state.lock().nominal_refresh_mhz;
        if m <= 0 {
            None
        } else {
            Some(m as f64 / 1000.0)
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
        let mut q = self.queue.lock();
        let mut s = self.state.lock();
        let _ = q.dispatch_pending(&mut s);
        let _ = q.flush();
    }

    /* Commit the child surface so any pending color-management state
     * change (e.g. unset_image_description) takes effect immediately. */
    pub fn commit_child(&self) {
        self.child_surface.commit();
        let q = self.queue.lock();
        let _ = q.flush();
    }
}

impl Drop for SubsurfaceVideo {
    fn drop(&mut self) {
        let _ = self
            .egl
            .make_current(self.egl_display, None, None, None);
        let _ = self
            .egl
            .destroy_surface(self.egl_display, *self.egl_surface.lock());
        let _ = self
            .egl
            .destroy_context(self.egl_display, self.egl_context);
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

impl Dispatch<WlOutput, ()> for SubState {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        /* Mode-event har Current-flag for den aktive moden. Refresh er i
         * mHz. Vi tar første aktive mode og holder den — multi-output
         * setups bruker første reported (typisk primær på user-hardware). */
        if let wl_output::Event::Mode { flags, refresh, .. } = event {
            let is_current = match flags {
                wayland_client::WEnum::Value(m) => m.contains(wl_output::Mode::Current),
                _ => false,
            };
            if is_current && refresh > 0 && state.nominal_refresh_mhz == 0 {
                state.nominal_refresh_mhz = refresh;
            }
        }
    }
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
            Event::Discarded => {
                state.presented_count = state.presented_count.saturating_add(1);
            }
            _ => {}
        }
    }
}

// silence unused warning when CM not yet wired
#[allow(dead_code)]
fn _phantom_cm(_: &CmState) {}
