use std::env;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/* Klient for nixlytile sin Niri-kompatible IPC. Brukes til å fortelle
 * compositoren at video starter/stopper, så den kan sette display refresh
 * rate (VRR hvis skjermen holder kildens rate, ellers nærmeste eksakte
 * mode, ellers heltall-multippel).
 *
 * Wire: én linje JSON per request over $NIRI_SOCKET. Server svarer
 * {"Ok":{"VideoMode":{"output":..,"mhz":..,"vrr":..}}} på VideoPlaying,
 * {"Ok":"Handled"} på VideoStopped, eller {"Err":...}. Vi åpner-skriver-
 * leser-lukker per melding (få meldinger, ikke verdt persistent socket).
 *
 * output: skjermnavn kun når maskinen har én skjerm; ellers "auto", som
 * ber nixlytile finne skjermen som faktisk viser videoen. Vi kan ikke vite
 * det selv — wl_output-listen vår er registry-rekkefølge, ikke skjermen
 * surfacet ligger på, og å gjette bytter mode på feil skjerm.
 *
 * Snap-til-standard skjer her: container-fps fra mpv kommer som
 * f.eks. 23.976024 — vi snapper til 23.976 før send, så nixlytile
 * kan matche eksisterende 23.976/47.952/119.880 modes presist. */

const STANDARD_RATES: &[f64] = &[
    23.976, 24.000, 25.000, 29.970, 30.000, 47.952, 48.000, 50.000, 59.940, 60.000, 100.000,
    119.880, 120.000,
];

/* Moden nixlytile faktisk satte. mhz = 0 om compositoren ikke rapporterte
 * (eldre nixlytile som svarer "Handled"). vrr = VRR pacer skjermen, da er
 * det ingen fast rate å verifisere mot. */
#[derive(Clone, Copy, Debug)]
pub struct AppliedMode {
    pub mhz: i32,
    pub vrr: bool,
}

impl AppliedMode {
    pub fn hz(&self) -> f64 {
        self.mhz as f64 / 1000.0
    }
}

/* Utfall av en playing()-melding. Failed betyr at det ikke er noen
 * nixlytile å vente på (kjører under en annen compositor) — kallere skal
 * da ikke holde avspilling tilbake. */
pub enum RateResult {
    Sent(Option<AppliedMode>),
    Duplicate,
    Failed,
}

pub struct VideoRate {
    output: String,
    last_sent: Option<f64>,
}

impl VideoRate {
    pub fn new(sole_output: Option<String>) -> Self {
        Self {
            output: sole_output.unwrap_or_else(|| "auto".to_string()),
            last_sent: None,
        }
    }

    pub fn playing(&mut self, raw_fps: f64) -> RateResult {
        let fps = snap_to_standard(raw_fps);
        if let Some(f) = self.last_sent {
            if (f - fps).abs() < 0.001 {
                return RateResult::Duplicate;
            }
        }
        let msg = format!(
            "{{\"Action\":{{\"VideoPlaying\":{{\"output\":\"{}\",\"fps\":{fps:.3}}}}}}}\n",
            self.output
        );
        match send(&msg) {
            Ok(reply) => {
                let applied = parse_video_mode(&reply);
                crate::nlog!(
                    "VideoPlaying @ {:.3} fps -> {} (applied {:.3} Hz{})",
                    fps,
                    reply.trim(),
                    applied.map(|a| a.hz()).unwrap_or(0.0),
                    if applied.map(|a| a.vrr).unwrap_or(false) { ", VRR" } else { "" }
                );
                self.last_sent = Some(fps);
                RateResult::Sent(applied)
            }
            Err(e) => {
                crate::nlog!("VideoPlaying IPC failed: {e}");
                RateResult::Failed
            }
        }
    }

    /* Glem hva som er sendt så neste playing() sender på nytt. Brukes når
     * målt display-Hz ikke lenger matcher moden compositoren rapporterte
     * (mode tapt ved hotplug, tag-switch eller mislykket commit) — uten
     * dette går resten av filmen på feil rate. */
    pub fn invalidate(&mut self) {
        self.last_sent = None;
    }

    pub fn stopped(&mut self) {
        if self.last_sent.is_none() {
            return;
        }
        match send(&stopped_msg(&self.output)) {
            Ok(reply) => {
                crate::nlog!("VideoStopped -> {}", reply.trim());
                self.last_sent = None;
            }
            Err(e) => crate::nlog!("VideoStopped IPC failed: {e}"),
        }
    }
}

fn stopped_msg(output: &str) -> String {
    format!("{{\"Action\":{{\"VideoStopped\":{{\"output\":\"{output}\"}}}}}}\n")
}

/* Synchronous one-shot VideoStopped. Brukes fra stop-path utenfor
 * render-tråden, hvor vi ikke har VideoRate-state. Idempotent på
 * nixlytile-siden (restore_max_refresh_rate er no-op om video-mode
 * allerede er restored), så det er trygt at både denne og render-tråden
 * sender ved exit. */
pub fn send_video_stopped(output: &str) {
    match send(&stopped_msg(output)) {
        Ok(reply) => crate::nlog!("VideoStopped -> {}", reply.trim()),
        Err(e) => crate::nlog!("VideoStopped IPC failed: {e}"),
    }
}

/* Minimal felt-plukk fra {"Ok":{"VideoMode":{"output":"HDMI-A-1","mhz":47952,
 * "vrr":false}}}. Eldre nixlytile svarer "Handled" → None. */
fn parse_video_mode(reply: &str) -> Option<AppliedMode> {
    let mhz_pos = reply.find("\"mhz\":")? + 6;
    let mhz: i32 = reply[mhz_pos..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit() && c != '-')
        .next()?
        .parse()
        .ok()?;
    let vrr = reply
        .find("\"vrr\":")
        .map(|p| reply[p + 6..].trim_start().starts_with("true"))
        .unwrap_or(false);
    Some(AppliedMode { mhz, vrr })
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
