use std::sync::Arc;

use parking_lot::Mutex;
use tokio::runtime::Handle;

use crate::api::Api;

const PROBE_BYTES: u64 = 8 * 1024 * 1024;

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

    pub fn start(&self, api: &Api, rt: &Handle, stream_id: i64, bitrate_bps: i64) {
        *self.inner.lock() = ProbeProgress::default();
        let api = api.clone();
        let state = self.inner.clone();
        rt.spawn(async move {
            let target = PROBE_BYTES;
            state.lock().bytes_total = target;
            let progress_state = state.clone();
            let mbps = api
                .probe_bandwidth_stream(stream_id, target, move |done, elapsed| {
                    let mut g = progress_state.lock();
                    g.bytes_done = done;
                    if elapsed > 0.05 {
                        g.mbps_running = (done as f64 * 8.0) / (elapsed * 1_000_000.0);
                    }
                })
                .await
                .map(|bps| bps * 8.0 / 1_000_000.0)
                .unwrap_or(0.0);
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
