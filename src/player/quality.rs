/* Adaptiv render-kvalitet. Statisk GPU-profil (iGPU vs dGPU) bommer på
 * alt mellom ytterpunktene — målt: full 4K-kjede (nlmeans + deband +
 * error-diffusion) tar 130 ms/frame på RTX 3070 Laptop → 8 fps og
 * konstante frame-drops. Mål faktisk render-tid og trapp ned dyreste
 * ledd til frametiden er under budsjett (1/kilde-fps). Trapper aldri
 * opp igjen — oscillering gir synlig kvalitetspumping. Sterk GPU når
 * aldri terskelen og beholder full kjede. */

use std::time::{Duration, Instant};

use libmpv2::Mpv;

/* Ny måleperiode etter hvert trinn — shader-recompile gir spike som
 * ellers ville trigget neste trinn umiddelbart. */
const SETTLE: Duration = Duration::from_secs(2);
/* Frames før EMA er troverdig nok til å felle dom. */
const MIN_SAMPLES: u32 = 12;
/* Render-tid over 85% av frame-intervallet = drops i praksis (swap,
 * pump og mpv-overhead kommer i tillegg). */
const BUDGET_FRAC: f64 = 0.85;
const EMA_ALPHA: f64 = 0.15;
const TIER_MAX: u8 = 3;

pub struct AdaptiveQuality {
    tier: u8,
    ema_us: f64,
    samples: u32,
    last_step: Instant,
}

impl AdaptiveQuality {
    pub fn new() -> Self {
        AdaptiveQuality {
            tier: 0,
            ema_us: 0.0,
            samples: 0,
            last_step: Instant::now(),
        }
    }

    pub fn tier(&self) -> u8 {
        self.tier
    }

    /* Kalles per faktisk videoframe (update-callback-vekkelse) med målt
     * render-tid. fps = container-fps for kilden (0 = ukjent ennå). */
    pub fn note_frame(&mut self, mpv: &Mpv, render_us: f64, fps: f64) {
        if self.tier >= TIER_MAX || fps <= 0.0 {
            return;
        }
        self.samples += 1;
        self.ema_us = if self.samples == 1 {
            render_us
        } else {
            self.ema_us + EMA_ALPHA * (render_us - self.ema_us)
        };
        if self.samples < MIN_SAMPLES || self.last_step.elapsed() < SETTLE {
            return;
        }
        let budget_us = 1_000_000.0 / fps * BUDGET_FRAC;
        if self.ema_us <= budget_us {
            return;
        }
        /* Grovt over budsjett (>2×): hopp to trinn — kortere stotring
         * før avspillingen setter seg. */
        let steps = if self.ema_us > 2.0 * budget_us { 2 } else { 1 };
        for _ in 0..steps {
            if self.tier >= TIER_MAX {
                break;
            }
            self.tier += 1;
            self.apply(mpv);
        }
        crate::nlog!(
            "adaptive quality: render {:.1} ms > budsjett {:.1} ms → tier {}",
            self.ema_us / 1000.0,
            budget_us / 1000.0,
            self.tier
        );
        self.samples = 0;
        self.ema_us = 0.0;
        self.last_step = Instant::now();
    }

    fn apply(&self, mpv: &Mpv) {
        match self.tier {
            /* nlmeans er dyrest — patch-søk på kilde-oppløsning. */
            1 => {
                let mut paths: Vec<String> = Vec::new();
                if let Some(p) = super::shaders::artcnn_path() {
                    paths.push(p);
                }
                if let Some(p) = super::shaders::krig_bilateral_path() {
                    paths.push(p);
                }
                let _ = mpv.set_property("glsl-shaders", paths.join(":").as_str());
            }
            2 => {
                let _ = mpv.set_property("glsl-shaders", "");
                let _ = mpv.set_property("hdr-compute-peak", "no");
            }
            /* Samme minimum som iGPU-profilen i mpv.rs init. */
            _ => {
                let _ = mpv.set_property("deband", "no");
                let _ = mpv.set_property("dither", "fruit");
                let _ = mpv.set_property("scale", "spline36");
                let _ = mpv.set_property("cscale", "bilinear");
                let _ = mpv.set_property("correct-downscaling", "no");
            }
        }
    }
}
