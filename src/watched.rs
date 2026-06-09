use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/* <2 min remaining = regnes som sett ferdig. Brukes både til check-ikon
 * og til å droppe resume-posisjon ved neste avspilling. */
pub const COMPLETED_REMAINING_SECS: f64 = 120.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchedEntry {
    pub position: f64,
    pub duration: f64,
    pub completed: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct OnDisk {
    episodes: HashMap<String, WatchedEntry>,
}

pub struct WatchedDb {
    path: PathBuf,
    map: HashMap<i64, WatchedEntry>,
    dirty: bool,
}

impl WatchedDb {
    pub fn load() -> Self {
        let path = data_path();
        let map = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<OnDisk>(&s).ok())
            .map(|d| {
                d.episodes
                    .into_iter()
                    .filter_map(|(k, v)| k.parse::<i64>().ok().map(|id| (id, v)))
                    .collect()
            })
            .unwrap_or_default();
        Self { path, map, dirty: false }
    }

    pub fn get(&self, id: i64) -> Option<&WatchedEntry> {
        self.map.get(&id)
    }

    pub fn is_completed(&self, id: i64) -> bool {
        self.map.get(&id).map(|e| e.completed).unwrap_or(false)
    }

    pub fn update(&mut self, id: i64, position: f64, duration: f64) {
        if duration <= 0.0 || !position.is_finite() || position < 0.0 {
            return;
        }
        let remaining = duration - position;
        let completed = remaining < COMPLETED_REMAINING_SECS;
        let changed = match self.map.get(&id) {
            Some(e) => {
                (e.position - position).abs() > 1.0
                    || e.completed != completed
                    || (e.duration - duration).abs() > 1.0
            }
            None => true,
        };
        if changed {
            self.map.insert(id, WatchedEntry { position, duration, completed });
            self.dirty = true;
        }
    }

    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let on_disk = OnDisk {
            episodes: self
                .map
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        };
        let Ok(json) = serde_json::to_string(&on_disk) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if fs::write(&tmp, json).is_ok() && fs::rename(&tmp, &self.path).is_ok() {
            self.dirty = false;
        }
    }
}

fn data_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        return home.join(".local/share/nixlymedia/watched.json");
    }
    PathBuf::from("/tmp/nixlymedia-watched.json")
}
