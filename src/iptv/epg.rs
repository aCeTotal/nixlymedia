use anyhow::{anyhow, Result};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::Api;

const CACHE_TTL_SECS: u64 = 15 * 60;

#[derive(Clone, Debug)]
pub struct EpgProgram {
    pub start: i64,
    pub stop: i64,
    pub title: String,
}

#[derive(Default, Clone, Debug)]
pub struct EpgIndex {
    by_channel: HashMap<String, Vec<EpgProgram>>,
}

impl EpgIndex {
    pub fn current(&self, channel_id: &str, now: i64) -> Option<&EpgProgram> {
        let programs = self.by_channel.get(channel_id)?;
        programs
            .iter()
            .find(|p| p.start <= now && now < p.stop)
    }

    pub fn is_active(&self, channel_id: &str) -> bool {
        self.by_channel
            .get(channel_id)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    pub fn is_empty(&self) -> bool {
        self.by_channel.is_empty()
    }
}

fn cache_path() -> Option<PathBuf> {
    let dir = dirs::cache_dir()?.join("nixlymedia");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("xmltv.xml"))
}

fn cache_fresh(path: &PathBuf) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    age.as_secs() < CACHE_TTL_SECS
}

pub async fn fetch_epg(api: &Api, base: &str, user: &str, pass: &str) -> Result<EpgIndex> {
    let path = cache_path();
    let xml = if let Some(p) = path.as_ref().filter(|p| cache_fresh(p)) {
        match std::fs::read_to_string(p) {
            Ok(s) => {
                crate::nlog!("epg: using disk cache ({} bytes)", s.len());
                s
            }
            Err(_) => fetch_remote(api, base, user, pass, path.as_ref()).await?,
        }
    } else {
        fetch_remote(api, base, user, pass, path.as_ref()).await?
    };
    parse_xmltv(&xml)
}

async fn fetch_remote(
    api: &Api,
    base: &str,
    user: &str,
    pass: &str,
    cache: Option<&PathBuf>,
) -> Result<String> {
    let url = format!("{base}/xmltv.php?username={user}&password={pass}");
    crate::nlog!("epg: fetching {url}");
    let bytes = api.get_bytes_url(&url).await?;
    let s = String::from_utf8(bytes.to_vec())
        .map_err(|e| anyhow!("xmltv not utf-8: {e}"))?;
    if let Some(p) = cache {
        let _ = std::fs::write(p, &s);
    }
    Ok(s)
}

pub fn parse_xmltv(xml: &str) -> Result<EpgIndex> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut by_channel: HashMap<String, Vec<EpgProgram>> = HashMap::new();
    let mut buf = Vec::new();

    let mut cur_channel: Option<String> = None;
    let mut cur_start: i64 = 0;
    let mut cur_stop: i64 = 0;
    let mut in_title = false;
    let mut cur_title = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.name();
                let tag = std::str::from_utf8(name.as_ref()).unwrap_or("");
                match tag {
                    "programme" => {
                        let mut start = 0i64;
                        let mut stop = 0i64;
                        let mut channel = String::new();
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or("");
                            let val = attr
                                .unescape_value()
                                .map(|v| v.into_owned())
                                .unwrap_or_default();
                            match key {
                                "start" => start = parse_xmltv_time(&val).unwrap_or(0),
                                "stop" => stop = parse_xmltv_time(&val).unwrap_or(0),
                                "channel" => channel = val,
                                _ => {}
                            }
                        }
                        cur_channel = if channel.is_empty() {
                            None
                        } else {
                            Some(channel)
                        };
                        cur_start = start;
                        cur_stop = stop;
                        cur_title.clear();
                    }
                    "title" if cur_channel.is_some() => {
                        in_title = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) if in_title => {
                if let Ok(s) = t.unescape() {
                    cur_title.push_str(&s);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let tag = std::str::from_utf8(name.as_ref()).unwrap_or("");
                match tag {
                    "title" => in_title = false,
                    "programme" => {
                        if let Some(ch) = cur_channel.take() {
                            if cur_start > 0 && cur_stop > cur_start {
                                by_channel.entry(ch).or_default().push(EpgProgram {
                                    start: cur_start,
                                    stop: cur_stop,
                                    title: std::mem::take(&mut cur_title),
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("xmltv parse: {e}")),
            _ => {}
        }
        buf.clear();
    }

    for v in by_channel.values_mut() {
        v.sort_by_key(|p| p.start);
    }

    crate::nlog!("epg: parsed {} channels", by_channel.len());
    Ok(EpgIndex { by_channel })
}

/// Parse XMLTV time: "YYYYMMDDHHMMSS +HHMM" or "YYYYMMDDHHMMSS".
/// Returns unix timestamp (seconds, UTC).
fn parse_xmltv_time(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 14 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let mon: u32 = s.get(4..6)?.parse().ok()?;
    let day: u32 = s.get(6..8)?.parse().ok()?;
    let hour: i64 = s.get(8..10)?.parse().ok()?;
    let min: i64 = s.get(10..12)?.parse().ok()?;
    let sec: i64 = s.get(12..14)?.parse().ok()?;

    let mut offset_secs: i64 = 0;
    if s.len() >= 20 {
        let tz = s[14..].trim();
        if tz.len() >= 5 {
            let sign = match tz.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => 0,
            };
            if sign != 0 {
                let oh: i64 = tz.get(1..3)?.parse().ok()?;
                let om: i64 = tz.get(3..5)?.parse().ok()?;
                offset_secs = sign * (oh * 3600 + om * 60);
            }
        }
    }

    let days = days_from_civil(year, mon, day);
    let secs = days * 86400 + hour * 3600 + min * 60 + sec - offset_secs;
    Some(secs)
}

/// Howard Hinnant's days_from_civil — days since 1970-01-01 (UTC).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let m = m as u64;
    let d = d as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
