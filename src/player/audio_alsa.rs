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

/* Detekter tilkoblet HDMI ALSA hw-device og hva anlegget kan ta imot.
 *
 * Hvorfor hw: direkte og ikke PipeWire? PW HDMI-sink-profil følger
 * EDID. TV som rapporterer kun stereo i EDID gjør at PW eksponerer
 * 2ch og downmixer 5.1 før det treffer ALSA. Ved å åpne ALSA hw:
 * direkte sender vi enten bit-perfect surround-LPCM eller en
 * komprimert bitstream uten at PW rører den — kernel-driveren sjekker
 * ikke EDID, og TV/AVR mottar det rått over eARC.
 *
 * Passthrough kontra LPCM velges "etter høyttaleranlegget": ELD-ens
 * SAD-liste sier hva anlegget faktisk dekoder. En AVR som annonserer
 * TrueHD/DTS-HD skal få kilde-bitstreamen (native Atmos/DTS:X), ikke
 * en nedmikset LPCM. En sink uten komprimerte SAD-er, men med LPCM
 * >= 6ch, får 6ch LPCM. Alt annet (TV som kun tar stereo-LPCM, BT,
 * analog) faller til pipewire-ao-en som nedmikser korrekt etter EDID —
 * en direkte hw:-åpning mot en 2ch-only-sink gir stillhet, og holder
 * devicet eksklusivt så alle andre apper også blir stumme.
 *
 * Merk: en vellykket hw:-åpning er eksklusiv. Derfor kun ekte
 * AVR/eARC (komprimert SAD eller LPCM >= 6ch) får direktepathen.
 *
 * Strategi: scan /proc/asound/cards for Nvidia/HDMI-kort, finn første
 * device med eld#<dev>.<sub> som har monitor_present=1, eld_valid=1
 * og enten en komprimert SAD eller LPCM-SAD >= 6ch.
 */
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
        if let Some((dev, content)) = find_connected_hdmi_device(*card) {
            return Some(HdmiAudio {
                device: format!("alsa/hw:{},{}", card, dev),
                spdif_codecs: eld_spdif_codecs(&content),
                lpcm_max_ch: eld_lpcm_max_channels(&content),
            });
        }
    }
    None
}

/* Første device på kortet med et anlegg som kan ta imot noe utover
 * stereo-LPCM. Returnerer (dev, ELD-innhold) så kaller-siden slipper å
 * lese fila på nytt. */
fn find_connected_hdmi_device(card: u32) -> Option<(u32, String)> {
    let dir = format!("/proc/asound/card{}", card);
    let entries = fs::read_dir(&dir).ok()?;
    let mut connected: Vec<(u32, String)> = Vec::new();
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
        /* Enten kan anlegget bitstreame (komprimert SAD) eller det tar
         * 6ch LPCM. Bare stereo-LPCM hører hjemme på pipewire-pathen. */
        if present
            && valid
            && (!eld_spdif_codecs(&content).is_empty()
                || eld_lpcm_max_channels(&content) >= 6)
        {
            connected.push((dev, content));
        }
    }
    connected.sort_by_key(|(dev, _)| *dev);
    connected.into_iter().next()
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
