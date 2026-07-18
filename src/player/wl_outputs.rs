use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_output::{self, WlOutput},
    wl_registry,
};
use wayland_client::{Connection, Dispatch, QueueHandle};

/* One-shot wl_output-query over en EGEN wayland-connection.
 *
 * Vi kan IKKE binde wl_output på winits delte connection: compositor
 * (wlroots) sender wl_surface.enter for HVER wl_output-resource
 * klienten har bundet. sctk lagrer da vår proxy (user-data OutputIdx,
 * ikke sctk OutputData) i SurfaceData, og winits MonitorHandle::size()
 * gjør data::<OutputData>().unwrap() → panic i egui_winit
 * (winit-0.30.13 wayland/output.rs:49) ved neste mode-switch/enter.
 *
 * Egen connection = egen klient server-side; ingen kryssforurensning.
 * Droppes ved retur — navn/refresh er statiske (connector-navn endres
 * ikke, og ekte refresh kommer løpende via wp_presentation_feedback). */

pub struct OutputInfo {
    /* wl_output.name (v4+), stabil connector-id, f.eks. "eDP-1". */
    pub name: Option<String>,
    /* Refresh i mHz fra mode med Current-flag. 0 = ikke mottatt. */
    pub refresh_mhz: i32,
}

struct QState {
    infos: Vec<OutputInfo>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for QState {
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

impl Dispatch<WlOutput, usize> for QState {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        idx: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(info) = state.infos.get_mut(*idx) else {
            return;
        };
        match event {
            wl_output::Event::Mode { flags, refresh, .. } => {
                let is_current = match flags {
                    wayland_client::WEnum::Value(m) => m.contains(wl_output::Mode::Current),
                    _ => false,
                };
                if is_current && refresh > 0 {
                    info.refresh_mhz = refresh;
                }
            }
            wl_output::Event::Name { name } => {
                info.name = Some(name);
            }
            _ => {}
        }
    }
}

/* Alle outputs med navn + nominell refresh. Tom Vec ved feil (ingen
 * WAYLAND_DISPLAY e.l.) — kallere behandler det som "ukjent". */
pub fn query() -> Vec<OutputInfo> {
    let Ok(conn) = Connection::connect_to_env() else {
        return Vec::new();
    };
    let Ok((globals, mut queue)) = registry_queue_init::<QState>(&conn) else {
        return Vec::new();
    };
    let qh = queue.handle();
    let mut state = QState { infos: Vec::new() };
    for g in globals.contents().clone_list() {
        if g.interface == "wl_output" {
            let idx = state.infos.len();
            state.infos.push(OutputInfo {
                name: None,
                refresh_mhz: 0,
            });
            globals
                .registry()
                .bind::<WlOutput, _, _>(g.name, g.version.min(4), &qh, idx);
        }
    }
    /* Roundtrip pumper geometry/mode/name/done for alle outputs. */
    let _ = queue.roundtrip(&mut state);
    state.infos
}
