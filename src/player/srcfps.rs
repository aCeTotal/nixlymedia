/* Hvilken fps displayet skal matche.
 *
 * container-fps er kildens rate og kjent med én gang, men den er feil når
 * filterkjeden endrer raten: deinterlacing av 50i gir container-fps 25 mens
 * vo leverer 50 fps — settes displayet da til 25 Hz mister vi annenhver
 * frame. estimated-vf-fps måler faktisk vo-rate, men bruker noen sekunder
 * på å bli stabil og er ubrukelig rett etter loadfile.
 *
 * Derfor: container-fps umiddelbart, og bytt til vf-fps først når den har
 * vært stabil og tydelig forskjellig flere målinger på rad. */

/* Avvik fra container-fps før vi i det hele tatt vurderer vf-fps. */
const DIVERGENCE: f64 = 0.10;
/* Hvor likt vf-fps må måles fra gang til gang for å regnes som stabil. */
const STABILITY: f64 = 0.02;
/* Antall like målinger på rad før vi bytter (pollen går hvert 500ms). */
const AGREE_NEEDED: u32 = 4;

pub struct VfFps {
    last: f64,
    agree: u32,
}

impl VfFps {
    pub fn new() -> Self {
        Self { last: 0.0, agree: 0 }
    }

    pub fn pick(&mut self, container: f64, vf: f64) -> f64 {
        if container <= 0.0 {
            /* Ingen container-metadata (noen HLS/mpegts-strømmer): vf-fps
             * er eneste kilde, men bruk den bare når den er stabil. */
            return if self.stable(vf) { vf } else { 0.0 };
        }
        if vf <= 0.0 || (vf / container - 1.0).abs() <= DIVERGENCE {
            self.agree = 0;
            self.last = vf;
            return container;
        }
        if self.stable(vf) {
            vf
        } else {
            container
        }
    }

    fn stable(&mut self, vf: f64) -> bool {
        if vf <= 0.0 {
            self.agree = 0;
            self.last = 0.0;
            return false;
        }
        if self.last > 0.0 && (vf / self.last - 1.0).abs() <= STABILITY {
            self.agree += 1;
        } else {
            self.agree = 0;
        }
        self.last = vf;
        self.agree >= AGREE_NEEDED
    }
}
