use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::watched::COMPLETED_REMAINING_SECS;

#[derive(Clone, Serialize, Deserialize)]
pub struct Tracked {
    pub series_id: String,
    pub series_title: String,
    pub season_id: String,
    pub image: Option<String>,
    pub episode_id: String,
    pub episode_label: String,
    pub episode_title: String,
    pub position_secs: u64,
    pub duration_secs: u64,
    pub updated: u64,
}

impl Tracked {
    pub fn finished(&self) -> bool {
        self.duration_secs > 0
            && (self.duration_secs.saturating_sub(self.position_secs) as f64)
                < COMPLETED_REMAINING_SECS
    }

    pub fn progress(&self) -> f32 {
        if self.duration_secs == 0 {
            return 0.0;
        }
        (self.position_secs as f32 / self.duration_secs as f32).clamp(0.0, 1.0)
    }
}

/* Started series, newest first. */
pub struct Tracker {
    path: PathBuf,
    series: Vec<Tracked>,
}

impl Tracker {
    pub fn load() -> Self {
        let path = data_path();
        let mut series: Vec<Tracked> = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        series.sort_by_key(|t| std::cmp::Reverse(t.updated));
        Self { path, series }
    }

    pub fn recent(&self) -> &[Tracked] {
        &self.series
    }

    pub fn len(&self) -> usize {
        self.series.len()
    }

    pub fn get(&self, series_id: &str) -> Option<&Tracked> {
        self.series.iter().find(|t| t.series_id == series_id)
    }

    pub fn record(&mut self, mut entry: Tracked) {
        entry.updated = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.series.retain(|t| t.series_id != entry.series_id);
        self.series.insert(0, entry);
        self.flush();
    }

    fn flush(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let Ok(json) = serde_json::to_string(&self.series) else {
            return;
        };
        let tmp = self.path.with_extension("json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
    }
}

fn data_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".local/share/nixlymedia/anime.json"))
        .unwrap_or_else(|| PathBuf::from("/tmp/nixlymedia-anime.json"))
}
