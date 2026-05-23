use std::sync::Arc;

use parking_lot::Mutex;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_surface};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::wp::color_management::v1::client as cm;

use crate::player::hdr::{HdrMeta, Transfer};

pub const TF_ST2084_PQ: u32 = 11;
pub const PRIM_BT2020: u32 = 6;
pub const PRIM_SRGB: u32 = 1;
pub const TF_SRGB: u32 = 2;

const REFERENCE_LUM_HDR: u32 = 203;
const REFERENCE_LUM_SDR: u32 = 80;

pub struct CmState {
    pub manager: Option<cm::wp_color_manager_v1::WpColorManagerV1>,
    pub supported_tf: Vec<u32>,
    pub supported_primaries: Vec<u32>,
    pub supported_features: Vec<u32>,
    pub done: bool,
    pub preferred_tf: Option<u32>,
    pub preferred_changed_flag: bool,
}

impl CmState {
    fn new() -> Self {
        Self {
            manager: None,
            supported_tf: Vec::new(),
            supported_primaries: Vec::new(),
            supported_features: Vec::new(),
            done: false,
            preferred_tf: None,
            preferred_changed_flag: false,
        }
    }

    pub fn supports_pq(&self) -> bool {
        self.supported_tf.contains(&TF_ST2084_PQ)
            && self.supported_primaries.contains(&PRIM_BT2020)
    }
}

#[allow(dead_code)]
pub struct ColorMgr {
    pub conn: Connection,
    pub queue: Arc<Mutex<EventQueue<CmState>>>,
    pub qh: QueueHandle<CmState>,
    pub state: Arc<Mutex<CmState>>,
}

impl ColorMgr {
    pub fn new(conn: Connection) -> anyhow::Result<Self> {
        let (globals, queue) = registry_queue_init::<CmState>(&conn)
            .map_err(|e| anyhow::anyhow!("registry init: {e}"))?;
        let qh = queue.handle();
        let mut state = CmState::new();
        for g in globals.contents().clone_list() {
            if g.interface == "wp_color_manager_v1" {
                let mgr = globals
                    .registry()
                    .bind::<cm::wp_color_manager_v1::WpColorManagerV1, _, _>(
                        g.name,
                        g.version.min(2),
                        &qh,
                        (),
                    );
                state.manager = Some(mgr);
            }
        }
        let queue = Arc::new(Mutex::new(queue));
        let state = Arc::new(Mutex::new(state));

        {
            let mut q = queue.lock();
            let mut s = state.lock();
            let _ = q.roundtrip(&mut s);
        }

        Ok(Self {
            conn,
            queue,
            qh,
            state,
        })
    }

    pub fn pump(&self) {
        let mut q = self.queue.lock();
        let mut s = self.state.lock();
        let _ = q.dispatch_pending(&mut s);
        let _ = q.flush();
    }

    pub fn manager(&self) -> Option<cm::wp_color_manager_v1::WpColorManagerV1> {
        self.state.lock().manager.clone()
    }

    pub fn supports_pq(&self) -> bool {
        self.state.lock().supports_pq()
    }

    pub fn build_image_description(
        &self,
        meta: &HdrMeta,
    ) -> Option<cm::wp_image_description_v1::WpImageDescriptionV1> {
        let mgr = self.manager()?;
        let s = self.state.lock();
        let supports = s.supported_features.clone();
        let supports_parametric =
            supports.contains(&cm::wp_color_manager_v1::Feature::Parametric.into());
        let supports_mastering = supports
            .contains(&cm::wp_color_manager_v1::Feature::SetMasteringDisplayPrimaries.into());
        let supports_luminances =
            supports.contains(&cm::wp_color_manager_v1::Feature::SetLuminances.into());
        drop(s);
        if !supports_parametric {
            return None;
        }

        let creator = mgr.create_parametric_creator(&self.qh, ());

        let (tf, prim) = match (meta.transfer, meta.primaries) {
            (Transfer::Pq, _) => (TF_ST2084_PQ, PRIM_BT2020),
            (Transfer::Hlg, _) => return None,
            _ => (TF_SRGB, PRIM_SRGB),
        };
        creator.set_tf_named(
            cm::wp_color_manager_v1::TransferFunction::try_from(tf).ok()?,
        );
        creator.set_primaries_named(
            cm::wp_color_manager_v1::Primaries::try_from(prim).ok()?,
        );

        let ref_lum = if matches!(meta.transfer, Transfer::Pq) {
            REFERENCE_LUM_HDR
        } else {
            REFERENCE_LUM_SDR
        };
        if supports_luminances {
            let (min_x10000, max) = if matches!(meta.transfer, Transfer::Pq) {
                (5_u32, 10_000_u32)
            } else {
                (2_u32, 80_u32)
            };
            creator.set_luminances(min_x10000, max, ref_lum);
        }

        if supports_mastering && matches!(meta.transfer, Transfer::Pq) {
            if let (Some(min), Some(max)) =
                (meta.mastering_min_luminance, meta.mastering_max_luminance)
            {
                let min_x10000 = (min * 10_000.0).round().max(0.0) as u32;
                let max_u = max.round().max(1.0) as u32;
                creator.set_mastering_luminance(min_x10000, max_u);
            }
        }
        if let Some(v) = meta.max_cll {
            creator.set_max_cll(v.round().max(0.0) as u32);
        }
        if let Some(v) = meta.max_fall {
            creator.set_max_fall(v.round().max(0.0) as u32);
        }

        let desc = creator.create(&self.qh, ());
        Some(desc)
    }

    pub fn attach_to_surface(
        &self,
        surface: &wl_surface::WlSurface,
        desc: &cm::wp_image_description_v1::WpImageDescriptionV1,
    ) -> Option<cm::wp_color_management_surface_v1::WpColorManagementSurfaceV1> {
        let mgr = self.manager()?;
        let cm_surf = mgr.get_surface(surface, &self.qh, ());
        cm_surf.set_image_description(
            desc,
            cm::wp_color_manager_v1::RenderIntent::Perceptual,
        );
        Some(cm_surf)
    }

    pub fn get_feedback(
        &self,
        surface: &wl_surface::WlSurface,
    ) -> Option<cm::wp_color_management_surface_feedback_v1::WpColorManagementSurfaceFeedbackV1>
    {
        let mgr = self.manager()?;
        Some(mgr.get_surface_feedback(surface, &self.qh, ()))
    }

    pub fn preferred_changed(&self) -> bool {
        let mut s = self.state.lock();
        let v = s.preferred_changed_flag;
        s.preferred_changed_flag = false;
        v
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for CmState {
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

impl Dispatch<cm::wp_color_manager_v1::WpColorManagerV1, ()> for CmState {
    fn event(
        state: &mut Self,
        _: &cm::wp_color_manager_v1::WpColorManagerV1,
        event: cm::wp_color_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use cm::wp_color_manager_v1::Event;
        match event {
            Event::SupportedTfNamed { tf } => {
                state.supported_tf.push(tf.into());
            }
            Event::SupportedPrimariesNamed { primaries } => {
                state.supported_primaries.push(primaries.into());
            }
            Event::SupportedFeature { feature } => {
                state.supported_features.push(feature.into());
            }
            Event::SupportedIntent { .. } => {}
            Event::Done => {
                state.done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<cm::wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1, ()>
    for CmState
{
    fn event(
        _: &mut Self,
        _: &cm::wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1,
        _: cm::wp_image_description_creator_params_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<cm::wp_image_description_v1::WpImageDescriptionV1, ()> for CmState {
    fn event(
        _: &mut Self,
        _: &cm::wp_image_description_v1::WpImageDescriptionV1,
        event: cm::wp_image_description_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let cm::wp_image_description_v1::Event::Failed { cause, msg } = event {
            eprintln!("[nixlymedia] image_description failed: cause={cause:?} msg={msg}");
        }
    }
}

impl Dispatch<cm::wp_color_management_surface_v1::WpColorManagementSurfaceV1, ()> for CmState {
    fn event(
        _: &mut Self,
        _: &cm::wp_color_management_surface_v1::WpColorManagementSurfaceV1,
        _: cm::wp_color_management_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<cm::wp_color_management_surface_feedback_v1::WpColorManagementSurfaceFeedbackV1, ()>
    for CmState
{
    fn event(
        state: &mut Self,
        _: &cm::wp_color_management_surface_feedback_v1::WpColorManagementSurfaceFeedbackV1,
        event: cm::wp_color_management_surface_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use cm::wp_color_management_surface_feedback_v1::Event;
        match event {
            Event::PreferredChanged { .. } | Event::PreferredChanged2 { .. } => {
                state.preferred_changed_flag = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<cm::wp_color_management_output_v1::WpColorManagementOutputV1, ()> for CmState {
    fn event(
        _: &mut Self,
        _: &cm::wp_color_management_output_v1::WpColorManagementOutputV1,
        _: cm::wp_color_management_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
