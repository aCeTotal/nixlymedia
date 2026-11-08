use serde::Deserialize;

#[derive(Deserialize)]
pub struct Page<T> {
    pub data: Vec<T>,
}

#[derive(Deserialize, Clone)]
pub struct Image {
    pub width: u32,
    pub source: String,
}

/* Image sets nest one level. */
#[derive(Deserialize, Clone, Default)]
pub struct Images {
    #[serde(default)]
    pub poster_tall: Vec<Vec<Image>>,
    #[serde(default)]
    pub poster_wide: Vec<Vec<Image>>,
    #[serde(default)]
    pub thumbnail: Vec<Vec<Image>>,
}

impl Images {
    pub fn tall(&self, min_width: u32) -> Option<&str> {
        pick(&self.poster_tall, min_width)
    }

    pub fn wide(&self, min_width: u32) -> Option<&str> {
        pick(&self.poster_wide, min_width).or_else(|| pick(&self.thumbnail, min_width))
    }
}

/* Smallest sufficient image, else largest. */
fn pick(sets: &[Vec<Image>], min_width: u32) -> Option<&str> {
    let set = sets.first()?;
    set.iter()
        .filter(|i| i.width >= min_width)
        .min_by_key(|i| i.width)
        .or_else(|| set.iter().max_by_key(|i| i.width))
        .map(|i| i.source.as_str())
}

#[derive(Deserialize, Clone, Default)]
pub struct SeriesMeta {
    #[serde(default)]
    pub episode_count: u32,
    #[serde(default)]
    pub series_launch_year: Option<u32>,
    #[serde(default)]
    pub tenant_categories: Option<Vec<String>>,
    #[serde(default)]
    pub maturity_ratings: Option<Vec<String>>,
}

#[derive(Deserialize, Clone)]
pub struct Series {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub images: Images,
    #[serde(default)]
    series_metadata: Option<SeriesMeta>,
    #[serde(flatten)]
    flat: SeriesMeta,
}

impl Series {
    /* Browse nests metadata, CMS flattens it. */
    pub fn meta(&self) -> &SeriesMeta {
        self.series_metadata.as_ref().unwrap_or(&self.flat)
    }
}

#[derive(Deserialize, Clone, Default)]
pub struct EpisodeMeta {
    pub series_id: String,
    #[serde(default)]
    pub series_title: String,
    #[serde(default)]
    pub season_id: String,
    #[serde(default)]
    pub episode: String,
    #[serde(default)]
    pub duration_ms: u64,
}

/* Browse nests metadata, seasons don't. */
#[derive(Deserialize, Clone)]
pub struct Episode {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub slug_title: String,
    #[serde(default)]
    pub images: Images,
    #[serde(default)]
    pub episode_metadata: Option<EpisodeMeta>,
    #[serde(default)]
    pub episode: String,
    #[serde(default)]
    pub duration_ms: u64,
}

impl Episode {
    pub fn label(&self) -> &str {
        match &self.episode_metadata {
            Some(m) if !m.episode.is_empty() => &m.episode,
            _ => &self.episode,
        }
    }

    pub fn duration_secs(&self) -> u64 {
        let ms = match &self.episode_metadata {
            Some(m) if m.duration_ms > 0 => m.duration_ms,
            _ => self.duration_ms,
        };
        ms / 1000
    }
}

#[derive(Deserialize, Clone)]
pub struct Season {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub season_number: u32,
}

#[derive(Deserialize, Clone)]
pub struct CategoryTitle {
    pub title: String,
}

#[derive(Deserialize, Clone)]
pub struct CategoryImages {
    #[serde(default)]
    pub background: Vec<Image>,
}

#[derive(Deserialize, Clone)]
pub struct Category {
    pub id: String,
    pub localization: CategoryTitle,
    pub images: CategoryImages,
}

impl Category {
    pub fn background(&self, min_width: u32) -> Option<&str> {
        pick(std::slice::from_ref(&self.images.background), min_width)
    }
}
