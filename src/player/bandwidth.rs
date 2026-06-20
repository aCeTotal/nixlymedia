use std::sync::Arc;

use parking_lot::Mutex;
use tokio::runtime::Handle;

use crate::api::Api;

const PROBE_BYTES: u64 = 8 * 1024 * 1024;
/* Hard tak på probe-varighet. En treg eller stallende linje (server
 * aksepterer TCP men trickler data) skal aldri henge Probing-fasen — ved
 * timeout bruker vi det vi rakk å måle (eller 0 = ukjent → lengste buffer)
 * og går videre. Uten dette satt vi i Probing til reqwest-klientens egen
 * 15 s request-timeout, og en helt død strøm kunne stoppe avspilling. */
const PROBE_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BufferPolicy {
    InstantStart,
    PreloadShort,
    PreloadLong,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub mbps: f64,
    pub bitrate_mbps: f64,
    pub policy: BufferPolicy,
    pub preload_seconds: u32,
    pub cache_seconds: u32,
}

#[derive(Debug, Clone, Default)]
pub struct ProbeProgress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub mbps_running: f64,
    pub finished: bool,
    pub result: Option<ProbeResult>,
}

#[derive(Clone)]
pub struct BandwidthProbe {
    inner: Arc<Mutex<ProbeProgress>>,
}

impl BandwidthProbe {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ProbeProgress::default())),
        }
    }

    pub fn snapshot(&self) -> ProbeProgress {
        self.inner.lock().clone()
    }

    pub fn set_instant(&self, cache_secs: u32) {
        *self.inner.lock() = ProbeProgress {
            bytes_done: 0,
            bytes_total: 0,
            mbps_running: 0.0,
            finished: true,
            result: Some(ProbeResult {
                mbps: 0.0,
                bitrate_mbps: 0.0,
                policy: BufferPolicy::InstantStart,
                preload_seconds: 0,
                cache_seconds: cache_secs,
            }),
        };
    }

    pub fn start(&self, api: &Api, rt: &Handle, stream_id: i64, bitrate_bps: i64) {
        *self.inner.lock() = ProbeProgress::default();
        let api = api.clone();
        let state = self.inner.clone();
        rt.spawn(async move {
            let target = PROBE_BYTES;
            state.lock().bytes_total = target;
            let progress_state = state.clone();
            let probe = api.probe_bandwidth_stream(stream_id, target, move |done, elapsed| {
                let mut g = progress_state.lock();
                g.bytes_done = done;
                if elapsed > 0.05 {
                    g.mbps_running = (done as f64 * 8.0) / (elapsed * 1_000_000.0);
                }
            });
            /* Timeout-vakt rundt probe. Faller den ut (treg/død linje) bruker
             * vi siste løpende måling — som regel en lav, men reell verdi som
             * trygt mapper til lengste buffer-policy. */
            let mbps = match tokio::time::timeout(
                std::time::Duration::from_secs(PROBE_TIMEOUT_SECS),
                probe,
            )
            .await
            {
                Ok(Ok(bps)) => bps * 8.0 / 1_000_000.0,
                Ok(Err(_)) => 0.0,
                Err(_) => state.lock().mbps_running,
            };
            let bitrate_mbps = (bitrate_bps as f64) / 1_000_000.0;
            let safe_bitrate = if bitrate_mbps > 0.1 { bitrate_mbps } else { 8.0 };
            let ratio = mbps / safe_bitrate;
            let (policy, preload, cache) = if ratio >= 2.0 {
                (BufferPolicy::InstantStart, 5, 600)
            } else if ratio >= 1.2 {
                (BufferPolicy::PreloadShort, 30, 600)
            } else {
                (BufferPolicy::PreloadLong, 120, 900)
            };
            let mut g = state.lock();
            g.bytes_done = target;
            g.mbps_running = mbps;
            g.finished = true;
            g.result = Some(ProbeResult {
                mbps,
                bitrate_mbps: safe_bitrate,
                policy,
                preload_seconds: preload,
                cache_seconds: cache,
            });
        });
    }
}
