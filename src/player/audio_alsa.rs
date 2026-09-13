use std::fs;

/* Detekter tilkoblet HDMI ALSA hw-device. Returnerer mpv-format
 * "alsa/hw:CARD,DEV" (f.eks. "alsa/hw:1,3").
 *
 * Hvorfor hw: direkte og ikke PipeWire? PW HDMI-sink-profil følger
 * EDID. TV som rapporterer kun stereo i EDID gjør at PW eksponerer
 * 2ch og downmixer 5.1 før det treffer ALSA. Ved å åpne ALSA hw:
 * direkte sender vi 6ch s32 LPCM uten downmix — kernel-driveren
 * sjekker ikke EDID, og TV/AVR mottar bit-perfect surround over eARC.
 *
 * Direktepathen krever at sinken faktisk DEKODER 6ch LPCM: kernelens
 * channels_max akkumulerer over alle SAD-er (også komprimerte AC3/
 * EAC3), så en 6ch LPCM-åpning lykkes mot en TV som bare dekoder 2ch
 * LPCM — TV-en spiller da stillhet. Og en vellykket hw:-åpning holder
 * devicet eksklusivt, så PipeWire-sinken (alle andre apper) blir stum
 * samtidig. Derfor: kun ELD med en LPCM-SAD på >= 6 kanaler (ekte
 * AVR/eARC) får direktepathen; TV-er faller til pipewire-ao-en, som
 * downmixer korrekt etter EDID.
 *
 * Strategi: scan /proc/asound/cards for Nvidia/HDMI-kort, finn første
 * device med eld#<dev>.<sub> som har monitor_present=1, eld_valid=1
 * og LPCM-SAD >= 6ch.
 */
pub fn detect_hdmi_hw() -> Option<String> {
    let cards = fs::read_to_string("/proc/asound/cards").ok()?;
    let mut candidates: Vec<(u32, &str)> = Vec::new();
    for line in cards.lines() {
        let t = line.trim_start();
        let mut it = t.split_whitespace();
        let Some(idx_str) = it.next() else {
            continue;
        };
        let Ok(idx) = idx_str.parse::<u32>() else {
            continue;
        };
        candidates.push((idx, line));
    }

    let mut nvidia: Vec<u32> = Vec::new();
    let mut hdmi: Vec<u32> = Vec::new();
    for (idx, line) in &candidates {
        let l = line.to_lowercase();
        if l.contains("nvidia") {
            nvidia.push(*idx);
        } else if l.contains("hdmi") {
            hdmi.push(*idx);
        }
    }

    /* Nvidia foretrekkes (dGPU) over iGPU HDMI. */
    for card in nvidia.iter().chain(hdmi.iter()) {
        if let Some(dev) = find_connected_hdmi_device(*card) {
            return Some(format!("alsa/hw:{},{}", card, dev));
        }
    }
    None
}

fn find_connected_hdmi_device(card: u32) -> Option<u32> {
    let dir = format!("/proc/asound/card{}", card);
    let entries = fs::read_dir(&dir).ok()?;
    let mut connected: Vec<u32> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix("eld#") else {
            continue;
        };
        let mut parts = rest.split('.');
        let dev_str = parts.next()?;
        let Ok(dev) = dev_str.parse::<u32>() else {
            continue;
        };
        let content = match fs::read_to_string(e.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mut present = false;
        let mut valid = false;
        for ln in content.lines() {
            let ln = ln.trim();
            if let Some(v) = ln.strip_prefix("monitor_present") {
                if v.trim() == "1" {
                    present = true;
                }
            } else if let Some(v) = ln.strip_prefix("eld_valid") {
                if v.trim() == "1" {
                    valid = true;
                }
            }
        }
        if present && valid && eld_lpcm_max_channels(&content) >= 6 {
            connected.push(dev);
        }
    }
    connected.sort();
    connected.into_iter().next()
}

/* Største kanalantall en LPCM-SAD i ELD-fila annonserer. Linjeformat
 * fra kernelens hda_eld.c:
 *   sad0_coding_type  [0x1] LPCM
 *   sad0_channels     2
 */
fn eld_lpcm_max_channels(content: &str) -> u32 {
    let mut max = 0u32;
    for ln in content.lines() {
        let ln = ln.trim();
        let Some(rest) = ln.strip_prefix("sad") else {
            continue;
        };
        let Some((idx, field)) = rest.split_once('_') else {
            continue;
        };
        if !field.starts_with("coding_type") || !field.contains("LPCM") {
            continue;
        }
        let key = format!("sad{}_channels", idx);
        for l2 in content.lines() {
            let l2 = l2.trim();
            let Some(v) = l2.strip_prefix(key.as_str()) else {
                continue;
            };
            if let Ok(ch) = v.trim().parse::<u32>() {
                if ch > max {
                    max = ch;
                }
            }
        }
    }
    max
}
