use std::ffi::c_void;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use khronos_egl as egl;
use parking_lot::Mutex;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_registry, wl_subcompositor::WlSubcompositor,
    wl_subsurface::WlSubsurface, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_egl::WlEglSurface;

use crate::player::wl_color::{CmState, ColorMgr};

#[allow(dead_code)]
pub struct SubState {
    pub compositor: Option<WlCompositor>,
    pub subcompositor: Option<WlSubcompositor>,
}

#[allow(dead_code)]
pub struct SubsurfaceVideo {
    pub conn: Connection,
    pub queue: Arc<Mutex<EventQueue<SubState>>>,
    pub qh: QueueHandle<SubState>,
    pub state: Arc<Mutex<SubState>>,

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

        let attrs_10bit = [
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
        let attrs_8bit = [
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
        let (egl_config, is_10bit) = match egl.choose_first_config(egl_display, &attrs_10bit) {
            Ok(Some(c)) => (c, true),
            _ => match egl.choose_first_config(egl_display, &attrs_8bit) {
                Ok(Some(c)) => (c, false),
                _ => return Err(anyhow!("no EGL config")),
            },
        };
        eprintln!(
            "[nixlymedia] subsurface EGL config: {}",
            if is_10bit { "10-bit RGB10_A2" } else { "8-bit RGBA8 (HDR fallback)" }
        );

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

        let queue = Arc::new(Mutex::new(queue));
        let state = Arc::new(Mutex::new(SubState {
            compositor: Some(compositor),
            subcompositor: Some(subcompositor),
        }));

        Ok(Self {
            conn,
            queue,
            qh,
            state,
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

// silence unused warning when CM not yet wired
#[allow(dead_code)]
fn _phantom_cm(_: &CmState) {}
