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
 * Strategi: scan /proc/asound/cards for Nvidia/HDMI-kort, finn første
 * device med eld#<dev>.<sub> som har monitor_present=1 og eld_valid=1.
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
        if present && valid {
            connected.push(dev);
        }
    }
    connected.sort();
    connected.into_iter().next()
}
