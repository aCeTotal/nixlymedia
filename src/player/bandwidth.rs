use std::sync::Arc;

use parking_lot::Mutex;
use tokio::runtime::Handle;

use crate::api::Api;

const PROBE_BYTES: u64 = 16 * 1024 * 1024;
/* Hard tak på probe-varighet. En treg eller stallende linje (server
 * aksepterer TCP men trickler data) skal aldri henge Probing-fasen — ved
 * timeout bruker vi det vi rakk å måle (eller 0 = ukjent → lengste buffer)
 * og går videre. Selve proben stopper uansett tidlig ved nok sample
 * (>= 4 s), så taket treffes bare på nesten døde linjer. */
const PROBE_TIMEOUT_SECS: u64 = 10;

/* Linje regnes som god nok for direktestart når målt hastighet er minst
 * 1.3× reell bitrate — 30% margin for jitter og bitrate-peaks. */
const INSTANT_RATIO: f64 = 1.3;
/* Antatt vedvarende hastighet = 85% av målt (langtids-throughput ligger
 * typisk under kortprobe). Brukes i buffer-dimensjoneringen. */
const SUSTAIN_FACTOR: f64 = 0.85;
/* Forhåndsbuffer større enn dette må disk-backes (RAM-cache er capped
 * til 2 GiB i mpv-init). */
const DISK_CACHE_THRESHOLD_BYTES: f64 = 1.5 * 1024.0 * 1024.0 * 1024.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BufferPolicy {
    InstantStart,
    PreloadShort,
    PreloadLong,
}

#[derive(Debug, Clone)]
pub struct ProbeResult {
    /* Målt steady-state linjehastighet, Mbit/s. */
    pub mbps: f64,
    /* Reell filbitrate (størrelse/varighet) når kjent, ellers metadata/8.0. */
    pub bitrate_mbps: f64,
    pub policy: BufferPolicy,
    /* Nødvendig forhåndsbuffer i INNHOLDS-sekunder før avspilling starter. */
    pub preload_seconds: u32,
    /* mpv cache-secs — settes til hele varigheten så resten av filen
     * fortsetter å laste i maks fart under avspilling. */
    pub cache_seconds: u32,
    /* Total filstørrelse fra Content-Range, når server oppga den. */
    pub file_size: Option<u64>,
    /* Forhåndsbufferet er for stort for RAM → mpv cache-on-disk. */
    pub disk_cache: bool,
    /* Estimert veggklokketid for å fylle forhåndsbufferet, sekunder.
     * Brukes til hard-cap i Preloading (aldri vent evig). */
    pub expected_wall_secs: f64,
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
    /* Generasjonsteller. start()/set_instant bumper den; en gammel probe-
     * task (fra forrige play, avbrutt av nytt start()-kall) sjekker gen før
     * hver skriving og dør stille istf å klusse inn stale result/progress
     * i den nye avspillingens Probing-fase. */
    gen: Arc<std::sync::atomic::AtomicU64>,
}

impl BandwidthProbe {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ProbeProgress::default())),
            gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    pub fn snapshot(&self) -> ProbeProgress {
        self.inner.lock().clone()
    }

    pub fn set_instant(&self, cache_secs: u32) {
        self.gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
                file_size: None,
                disk_cache: false,
                expected_wall_secs: 0.0,
            }),
        };
    }

    pub fn start(
        &self,
        api: &Api,
        rt: &Handle,
        stream_id: i64,
        bitrate_bps: i64,
        duration_secs: f64,
        resume_pos: f64,
    ) {
        use std::sync::atomic::Ordering;
        let my_gen = self.gen.fetch_add(1, Ordering::Relaxed) + 1;
        *self.inner.lock() = ProbeProgress::default();
        let api = api.clone();
        let state = self.inner.clone();
        let gen = self.gen.clone();
        rt.spawn(async move {
            let target = PROBE_BYTES;
            state.lock().bytes_total = target;
            let progress_state = state.clone();
            let progress_gen = gen.clone();
            let probe = api.probe_bandwidth_stream(stream_id, target, move |done, elapsed| {
                if progress_gen.load(Ordering::Relaxed) != my_gen {
                    return;
                }
                let mut g = progress_state.lock();
                g.bytes_done = done;
                if elapsed > 0.05 {
                    g.mbps_running = (done as f64 * 8.0) / (elapsed * 1_000_000.0);
                }
            });
            /* Timeout-vakt rundt probe. Faller den ut (treg/død linje) bruker
             * vi siste løpende måling — som regel en lav, men reell verdi som
             * trygt mapper til lengste buffer-policy. */
            let (mbps, file_size) = match tokio::time::timeout(
                std::time::Duration::from_secs(PROBE_TIMEOUT_SECS),
                probe,
            )
            .await
            {
                Ok(Ok((bps, size))) => (bps * 8.0 / 1_000_000.0, size),
                Ok(Err(_)) => (0.0, None),
                Err(_) => (state.lock().mbps_running, None),
            };
            let result = compute_policy(mbps, file_size, bitrate_bps, duration_secs, resume_pos);
            crate::nlog!(
                "probe: {:.1} Mbps, bitrate {:.1} Mbps, size {:?}, policy {:?}, preload {}s, disk {}, eta {:.0}s",
                result.mbps, result.bitrate_mbps, result.file_size,
                result.policy, result.preload_seconds, result.disk_cache,
                result.expected_wall_secs
            );
            if gen.load(Ordering::Relaxed) != my_gen {
                return;
            }
            let mut g = state.lock();
            g.bytes_done = target;
            g.mbps_running = mbps;
            g.finished = true;
            g.result = Some(result);
        });
    }
}

/* Dimensjonerer forhåndsbufferet mot filen som faktisk skal spilles.
 *
 * Matematikk: med gjenstående spilletid D (s), bitrate B og vedvarende
 * hastighet S (samme enhet) må forhåndsbufferet P (innholds-sekunder)
 * oppfylle P·B + S·t >= B·t for hele avspillingen (t <= D), dvs.
 *   P >= D·(1 - S/B).
 * S >= B → P = 0 (direktestart). Vi bruker S = SUSTAIN_FACTOR·målt og
 * legger 10 s slack på toppen. */
fn compute_policy(
    mbps: f64,
    file_size: Option<u64>,
    bitrate_bps: i64,
    duration_secs: f64,
    resume_pos: f64,
) -> ProbeResult {
    /* Reell bitrate fra størrelse/varighet trumfer metadata (som ofte er
     * 0 eller container-oppgitt gjennomsnitt uten overhead). */
    let meta_mbps = (bitrate_bps as f64) / 1_000_000.0;
    let real_mbps = match file_size {
        Some(sz) if duration_secs > 1.0 => (sz as f64 * 8.0) / (duration_secs * 1_000_000.0),
        _ => 0.0,
    };
    let bitrate_mbps = if real_mbps > 0.1 {
        real_mbps
    } else if meta_mbps > 0.1 {
        meta_mbps
    } else {
        8.0
    };

    let remaining = if duration_secs > 1.0 {
        (duration_secs - resume_pos).max(60.0)
    } else {
        0.0
    };
    /* cache-secs = hele gjenstående varighet: mpv skal aldri slutte å
     * readahead'e — resten av filen lastes i maks fart under avspilling. */
    let cache_seconds = if remaining > 0.0 {
        (remaining as u32).saturating_add(120).max(900)
    } else {
        900
    };

    let ratio = mbps / bitrate_mbps;
    if ratio >= INSTANT_RATIO {
        return ProbeResult {
            mbps,
            bitrate_mbps,
            policy: BufferPolicy::InstantStart,
            preload_seconds: 5,
            cache_seconds,
            file_size,
            disk_cache: false,
            expected_wall_secs: 0.0,
        };
    }

    /* Målt ~0 = ukjent linje (probe feilet). Ikke lås brukeren i et
     * "buffre hele filen"-estimat — moderat buffer og la mpv cache-pause
     * rebuffre ved behov. */
    if mbps <= 0.05 || remaining <= 0.0 {
        let preload = if remaining > 0.0 { remaining.min(120.0) } else { 120.0 };
        return ProbeResult {
            mbps,
            bitrate_mbps,
            policy: BufferPolicy::PreloadLong,
            preload_seconds: preload as u32,
            cache_seconds,
            file_size,
            disk_cache: false,
            expected_wall_secs: 60.0,
        };
    }

    let sustain_ratio = (ratio * SUSTAIN_FACTOR).min(1.0);
    let preload = (remaining * (1.0 - sustain_ratio) + 10.0).clamp(10.0, remaining);
    let bitrate_bytes = bitrate_mbps * 1_000_000.0 / 8.0;
    let speed_bytes = mbps * 1_000_000.0 / 8.0;
    let needed_bytes = preload * bitrate_bytes;
    let disk_cache = needed_bytes > DISK_CACHE_THRESHOLD_BYTES;
    let expected_wall_secs = needed_bytes / speed_bytes;
    let policy = if preload <= 45.0 {
        BufferPolicy::PreloadShort
    } else {
        BufferPolicy::PreloadLong
    };
    ProbeResult {
        mbps,
        bitrate_mbps,
        policy,
        preload_seconds: preload.ceil() as u32,
        cache_seconds,
        file_size,
        disk_cache,
        expected_wall_secs,
    }
}
