use serde::Deserialize;

fn de_opt_str<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(d)?;
    Ok(opt.filter(|s| !s.is_empty()))
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Status {
    pub status: String,
    pub server_name: String,
    #[serde(default)]
    pub rating: i32,
    #[serde(default)]
    pub active_ips: i32,
    #[serde(default)]
    pub max_ips: i32,
    #[serde(default)]
    pub can_start: bool,
    #[serde(default)]
    pub scanning: bool,
    #[serde(default)]
    pub upload_mbps: i32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Movie {
    pub id: i64,
    #[serde(default)]
    pub r#type: i32,
    pub title: String,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub show_name: Option<String>,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub duration: i32,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default)]
    pub tmdb_id: i64,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub tmdb_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub poster: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub backdrop: Option<String>,
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub rating: f32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub genres: Option<String>,
}

impl Movie {
    pub fn display_title(&self) -> &str {
        self.tmdb_title.as_deref().unwrap_or(&self.title)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct TvShow {
    pub id: i64,
    #[serde(default)]
    pub r#type: i32,
    pub title: String,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub show_name: Option<String>,
    #[serde(default)]
    pub episode_count: i32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub seasons: Option<String>,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub duration: i32,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default)]
    pub tmdb_id: i64,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub tmdb_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub poster: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub backdrop: Option<String>,
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub rating: f32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub genres: Option<String>,
    #[serde(default)]
    pub tmdb_total_seasons: i32,
    #[serde(default)]
    pub tmdb_total_episodes: i32,
    #[serde(default)]
    pub tmdb_episode_runtime: i32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub tmdb_status: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub tmdb_next_episode: Option<String>,
}

impl TvShow {
    pub fn display_title(&self) -> &str {
        self.tmdb_title
            .as_deref()
            .or(self.show_name.as_deref())
            .unwrap_or(&self.title)
    }

    pub fn key(&self) -> &str {
        self.tmdb_title
            .as_deref()
            .or(self.show_name.as_deref())
            .unwrap_or(&self.title)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SeasonInfo {
    pub season: i32,
    pub episode_count: i32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct Episode {
    pub id: i64,
    #[serde(default)]
    pub r#type: i32,
    pub title: String,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub show_name: Option<String>,
    pub season: i32,
    pub episode: i32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub episode_display: Option<String>,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub duration: i32,
    #[serde(default)]
    pub tmdb_id: i64,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub tmdb_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub poster: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub backdrop: Option<String>,
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub rating: f32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub episode_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub episode_overview: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub still: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct MediaDetail {
    pub id: i64,
    #[serde(default)]
    pub r#type: i32,
    pub title: String,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub show_name: Option<String>,
    #[serde(default)]
    pub season: i32,
    #[serde(default)]
    pub episode: i32,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub duration: i32,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub codec_video: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub codec_audio: Option<String>,
    #[serde(default)]
    pub bitrate: i64,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub tmdb_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub poster: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub backdrop: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub release_date: Option<String>,
    #[serde(default)]
    pub year: i32,
    #[serde(default)]
    pub rating: f32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub genres: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub episode_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub episode_overview: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub still: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub hdr_format: Option<String>,
    #[serde(default)]
    pub bit_depth: i32,
    #[serde(default)]
    pub audio_channels: i32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub audio_layout: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub audio_features: Option<String>,
    #[serde(default)]
    pub cast: Vec<CastMember>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ServerCollection {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub count: i32,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub poster: Option<String>,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub backdrop: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ServerCollectionDetail {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub items: Vec<MediaDetail>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct CastMember {
    #[serde(default)]
    pub tmdb_person_id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub character: String,
    #[serde(default, deserialize_with = "de_opt_str")]
    pub profile: Option<String>,
}
