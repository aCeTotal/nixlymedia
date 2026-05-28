use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/* Klient for nixlytile sin Niri-kompatible IPC. Brukes til å fortelle
 * compositoren at video starter/stopper på en gitt output, så den kan
 * sette display refresh rate (VRR hvis støttet, ellers nærmeste eksakte
 * mode, ellers heltall-multippel).
 *
 * Wire: én linje JSON per request over $NIRI_SOCKET. Server svarer
 * {"Ok":"Handled"} eller {"Err":...}. Vi åpner-skriver-leser-lukker
 * per melding (få meldinger, ikke verdt persistent socket).
 *
 * Snap-til-standard skjer her: container-fps fra mpv kommer som
 * f.eks. 23.976024 — vi snapper til 23.976 før send, så nixlytile
 * kan matche eksisterende 23.976/47.952/119.880 modes presist. */

const STANDARD_RATES: &[f64] = &[
    23.976, 24.000, 25.000, 29.970, 30.000, 47.952, 48.000, 50.000, 59.940, 60.000, 100.000,
    119.880, 120.000,
];

pub struct VideoRate {
    last_sent: Option<(String, f64)>,
}

impl VideoRate {
    pub fn new() -> Self {
        Self { last_sent: None }
    }

    pub fn playing(&mut self, output: &str, raw_fps: f64) {
        let fps = snap_to_standard(raw_fps);
        if let Some((o, f)) = &self.last_sent {
            if o == output && (f - fps).abs() < 0.001 {
                return;
            }
        }
        let msg = format!(
            "{{\"Action\":{{\"VideoPlaying\":{{\"output\":\"{}\",\"fps\":{:.3}}}}}}}\n",
            output, fps
        );
        match send(&msg) {
            Ok(reply) => {
                crate::nlog!("VideoPlaying {} @ {:.3} fps -> {}", output, fps, reply.trim());
                self.last_sent = Some((output.to_string(), fps));
            }
            Err(e) => crate::nlog!("VideoPlaying IPC failed: {e}"),
        }
    }

    pub fn stopped(&mut self) {
        let Some((output, _)) = self.last_sent.clone() else {
            return;
        };
        let msg = format!(
            "{{\"Action\":{{\"VideoStopped\":{{\"output\":\"{}\"}}}}}}\n",
            output
        );
        match send(&msg) {
            Ok(reply) => {
                crate::nlog!("VideoStopped {} -> {}", output, reply.trim());
                self.last_sent = None;
            }
            Err(e) => crate::nlog!("VideoStopped IPC failed: {e}"),
        }
    }
}

/* Synchronous one-shot VideoStopped. Brukes fra stop-path utenfor
 * render-tråden, hvor vi ikke har VideoRate-state. Idempotent på
 * nixlytile-siden (restore_max_refresh_rate er no-op om video-mode
 * allerede er restored), så det er trygt at både denne og render-tråden
 * sender ved exit. */
pub fn send_video_stopped(output: &str) {
    let msg = format!(
        "{{\"Action\":{{\"VideoStopped\":{{\"output\":\"{}\"}}}}}}\n",
        output
    );
    match send(&msg) {
        Ok(reply) => crate::nlog!("VideoStopped {} -> {}", output, reply.trim()),
        Err(e) => crate::nlog!("VideoStopped IPC failed: {e}"),
    }
}

fn snap_to_standard(raw: f64) -> f64 {
    let mut best = raw;
    let mut best_diff = f64::INFINITY;
    for &r in STANDARD_RATES {
        let d = (raw - r).abs();
        if d < best_diff {
            best_diff = d;
            best = r;
        }
    }
    /* Tolerance 0.5 fps. Utenfor → returner rå (eksotiske rater, f.eks.
     * 90 fps source — la nixlytile sin scoring håndtere). */
    if best_diff <= 0.5 {
        best
    } else {
        raw
    }
}

fn send(msg: &str) -> std::io::Result<String> {
    let path = env::var("NIRI_SOCKET").map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "NIRI_SOCKET not set")
    })?;
    let mut sock = UnixStream::connect(&path)?;
    sock.set_write_timeout(Some(Duration::from_millis(500)))?;
    sock.set_read_timeout(Some(Duration::from_millis(500)))?;
    sock.write_all(msg.as_bytes())?;
    let mut buf = [0u8; 256];
    let n = sock.read(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf[..n]).into_owned())
}
