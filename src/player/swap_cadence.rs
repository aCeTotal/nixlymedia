use std::time::Instant;

/* Lengre hull = pause, ikke vsync. */
const GAP_MAX_SECS: f64 = 0.25;
/* Færre intervaller gir ingen dom. */
const MIN_INTERVALS: u32 = 8;

/* Målt report_swap-rate per vindu. */
#[derive(Default)]
pub struct SwapCadence {
    last: Option<Instant>,
    sum_secs: f64,
    intervals: u32,
}

impl SwapCadence {
    pub fn note(&mut self, now: Instant) {
        let Some(prev) = self.last.replace(now) else {
            return;
        };
        let dt = now.duration_since(prev).as_secs_f64();
        if dt >= GAP_MAX_SECS {
            return;
        }
        self.sum_secs += dt;
        self.intervals += 1;
    }

    /* Rate siden forrige kall. */
    pub fn take_hz(&mut self) -> Option<f64> {
        let (sum, n) = (self.sum_secs, self.intervals);
        self.sum_secs = 0.0;
        self.intervals = 0;
        (n >= MIN_INTERVALS).then(|| f64::from(n) / sum)
    }

    /* Timeout er ingen vblank. */
    pub fn break_chain(&mut self) {
        self.last = None;
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const PERIOD: Duration = Duration::from_nanos(41_708_375);

    fn ticks(start: Instant, period: Duration, count: u32) -> impl Iterator<Item = Instant> {
        (0..count).map(move |i| start + period * i)
    }

    #[test]
    fn measures_steady_rate() {
        let mut c = SwapCadence::default();
        ticks(Instant::now(), PERIOD, 25).for_each(|t| c.note(t));
        assert!((c.take_hz().unwrap() - 23.976).abs() < 1e-3);
    }

    #[test]
    fn ignores_pause_gap() {
        let mut c = SwapCadence::default();
        let start = Instant::now();
        ticks(start, PERIOD, 13).for_each(|t| c.note(t));
        ticks(start + Duration::from_secs(3), PERIOD, 13).for_each(|t| c.note(t));
        assert!((c.take_hz().unwrap() - 23.976).abs() < 1e-3);
    }

    #[test]
    fn sparse_window_has_no_verdict() {
        let mut c = SwapCadence::default();
        ticks(Instant::now(), PERIOD, MIN_INTERVALS).for_each(|t| c.note(t));
        assert!(c.take_hz().is_none());
    }

    #[test]
    fn take_starts_new_window() {
        let mut c = SwapCadence::default();
        let start = Instant::now();
        ticks(start, PERIOD, 25).for_each(|t| c.note(t));
        c.take_hz();
        let fast = Duration::from_millis(16);
        ticks(start + PERIOD * 24 + fast, fast, 30).for_each(|t| c.note(t));
        assert!((c.take_hz().unwrap() - 62.5).abs() < 1e-6);
    }

    #[test]
    fn broken_chain_skips_timeout_interval() {
        let mut c = SwapCadence::default();
        let start = Instant::now();
        ticks(start, PERIOD, 13).for_each(|t| c.note(t));
        c.break_chain();
        ticks(start + PERIOD * 12 + Duration::from_millis(104), PERIOD, 13)
            .for_each(|t| c.note(t));
        assert!((c.take_hz().unwrap() - 23.976).abs() < 1e-3);
    }

    #[test]
    fn reset_drops_open_interval() {
        let mut c = SwapCadence::default();
        let start = Instant::now();
        c.note(start);
        c.reset();
        ticks(start + Duration::from_millis(200), PERIOD, 9).for_each(|t| c.note(t));
        assert!((c.take_hz().unwrap() - 23.976).abs() < 1e-3);
    }
}
