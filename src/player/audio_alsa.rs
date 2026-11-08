use std::fs;

/* Lydevnene til et tilkoblet HDMI-anlegg, lest fra ELD-en. */
pub struct HdmiAudio {
    /* mpv audio-device, format "alsa/hw:CARD,DEV". */
    pub device: String,
    /* mpv audio-spdif-koder anlegget kan bitstreame (ac3, dts, eac3,
     * dts-hd, truehd), i ELD-rekkefølge. Tom = ingen komprimert SAD. */
    pub spdif_codecs: Vec<String>,
    /* Største LPCM-kanalantall ELD-en annonserer. */
    pub lpcm_max_ch: u32,
}

/* Tilkoblet HDMI-anlegg, surround foretrukket. */
pub fn detect_hdmi_audio() -> Option<HdmiAudio> {
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

    /* PCH-kort bærer også iGPU-HDMI. */
    let mut nvidia: Vec<u32> = Vec::new();
    let mut other: Vec<u32> = Vec::new();
    for (idx, line) in &candidates {
        if line.to_lowercase().contains("nvidia") {
            nvidia.push(*idx);
        } else {
            other.push(*idx);
        }
    }

    /* Nvidia før iGPU HDMI. */
    let mut found: Vec<HdmiAudio> = Vec::new();
    for card in nvidia.iter().chain(other.iter()) {
        for (dev, content) in connected_hdmi_devices(*card) {
            found.push(HdmiAudio {
                device: format!("alsa/hw:{},{}", card, dev),
                spdif_codecs: eld_spdif_codecs(&content),
                lpcm_max_ch: eld_lpcm_max_channels(&content),
            });
        }
    }
    let pick = found.iter().position(HdmiAudio::is_surround).unwrap_or(0);
    (pick < found.len()).then(|| found.swap_remove(pick))
}

impl HdmiAudio {
    /* Mer enn stereo-LPCM. */
    pub fn is_surround(&self) -> bool {
        !self.spdif_codecs.is_empty() || self.lpcm_max_ch >= 6
    }
}

/* Tilkoblede devices med gyldig ELD. */
fn connected_hdmi_devices(card: u32) -> Vec<(u32, String)> {
    let dir = format!("/proc/asound/card{}", card);
    let mut connected: Vec<(u32, String)> = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return connected;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix("eld#") else {
            continue;
        };
        let dev_str = rest.split('.').next().unwrap_or_default();
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
            connected.push((dev, content));
        }
    }
    connected.sort_by_key(|(dev, _)| *dev);
    connected
}

/* mpv audio-spdif-koder anlegget annonserer som komprimerte SAD-er.
 *
 * ELD-linje fra kernelens hda_eld.c:
 *   sad0_coding_type  [0x2] AC-3
 * CEA-861 audio format codes → mpv-navn (bare de mpv kan bitstreame):
 *   0x2 AC-3 → ac3     0x7 DTS   → dts     0xa E-AC-3 → eac3
 *   0xb DTS-HD → dts-hd 0xc MLP (TrueHD) → truehd
 */
fn eld_spdif_codecs(content: &str) -> Vec<String> {
    let mut codecs: Vec<String> = Vec::new();
    for ln in content.lines() {
        let ln = ln.trim();
        let Some(rest) = ln.strip_prefix("sad") else {
            continue;
        };
        let Some((_, field)) = rest.split_once('_') else {
            continue;
        };
        if !field.starts_with("coding_type") {
            continue;
        }
        /* Match på hex-koden i [0x..], ikke navnet — navnestrengen
         * varierer mellom kernelversjoner, koden er stabil (CEA-861). */
        let Some(hex_start) = field.find("[0x") else {
            continue;
        };
        let Some(hex_end) = field[hex_start..].find(']') else {
            continue;
        };
        let hex = &field[hex_start + 3..hex_start + hex_end];
        let Ok(code) = u32::from_str_radix(hex.trim(), 16) else {
            continue;
        };
        let mpv = match code {
            0x2 => "ac3",
            0x7 => "dts",
            0xa => "eac3",
            0xb => "dts-hd",
            0xc => "truehd",
            _ => continue,
        };
        if !codecs.iter().any(|c| c == mpv) {
            codecs.push(mpv.to_string());
        }
    }
    codecs
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

#[cfg(test)]
mod tests {
    use super::*;

    /* ELD-utdrag i kernelens hda_eld.c-format: AVR med LPCM 8ch +
     * DTS-HD + TrueHD. */
    const AVR_ELD: &str = "\
monitor_present\t\t1
eld_valid\t\t1
monitor_name\t\tDENON-AVR
sad_count\t\t3
sad0_coding_type\t[0x1] LPCM
sad0_channels\t\t8
sad1_coding_type\t[0xb] DTS-HD
sad1_channels\t\t8
sad2_coding_type\t[0xc] MLP
sad2_channels\t\t8
";

    /* TV som kun tar 2ch LPCM. */
    const TV_ELD: &str = "\
monitor_present\t\t1
eld_valid\t\t1
sad0_coding_type\t[0x1] LPCM
sad0_channels\t\t2
";

    #[test]
    fn avr_advertises_passthrough_and_8ch_lpcm() {
        assert_eq!(eld_spdif_codecs(AVR_ELD), vec!["dts-hd", "truehd"]);
        assert_eq!(eld_lpcm_max_channels(AVR_ELD), 8);
    }

    #[test]
    fn stereo_tv_has_no_passthrough() {
        assert!(eld_spdif_codecs(TV_ELD).is_empty());
        assert_eq!(eld_lpcm_max_channels(TV_ELD), 2);
    }
}
