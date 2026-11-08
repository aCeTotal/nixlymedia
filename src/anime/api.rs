use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use base64::Engine;
use parking_lot::Mutex;
use reqwest::Client;
use serde::Deserialize;

use super::model::{Category, Episode, Page, Season, Series};

const HOST: &str = "https://www.crunchyroll.com";
/* Public anonymous web client. */
const ANON_CLIENT: &str = "cr_web";
const LOCALE: &str = "en-US";
/* Renew before the token lapses. */
const TOKEN_MARGIN: Duration = Duration::from_secs(60);
/* Each episode lists once per dub. */
const DUB_HEADROOM: u32 = 3;
/* Largest page browse accepts. */
const PAGE_MAX: u32 = 100;

#[derive(Deserialize)]
struct Token {
    access_token: String,
    expires_in: u64,
}

struct Bearer {
    header: String,
    valid_until: Instant,
}

#[derive(Clone)]
pub struct CrApi {
    http: Client,
    bearer: Arc<Mutex<Option<Bearer>>>,
}

impl CrApi {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent("nixlymedia/0.1")
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest client");
        Self {
            http,
            bearer: Arc::new(Mutex::new(None)),
        }
    }

    async fn bearer(&self) -> Result<String> {
        if let Some(b) = self.bearer.lock().as_ref() {
            if Instant::now() < b.valid_until {
                return Ok(b.header.clone());
            }
        }
        let basic = base64::engine::general_purpose::STANDARD.encode(format!("{ANON_CLIENT}:"));
        let token: Token = self
            .http
            .post(format!("{HOST}/auth/v1/token"))
            .header("Authorization", format!("Basic {basic}"))
            .form(&[("grant_type", "client_id")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let header = format!("Bearer {}", token.access_token);
        let lifetime = Duration::from_secs(token.expires_in).saturating_sub(TOKEN_MARGIN);
        *self.bearer.lock() = Some(Bearer {
            header: header.clone(),
            valid_until: Instant::now() + lifetime,
        });
        Ok(header)
    }

    async fn page<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<Vec<T>> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let page: Page<T> = self
            .http
            .get(format!("{HOST}{path}{sep}locale={LOCALE}"))
            .header("Authorization", self.bearer().await?)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(page.data)
    }

    pub async fn categories(&self) -> Result<Vec<Category>> {
        self.page("/content/v2/discover/categories").await
    }

    /* Popular series, optionally by category. */
    pub async fn popular(&self, category: Option<&str>, n: u32) -> Result<Vec<Series>> {
        let filter = category
            .map(|c| format!("&categories={}", urlencoding::encode(c)))
            .unwrap_or_default();
        let path =
            format!("/content/v2/discover/browse?type=series&sort_by=popularity&n={n}{filter}");
        self.page(&path).await
    }

    /* Newest episodes, dubs folded together. */
    pub async fn new_episodes(&self, n: u32) -> Result<Vec<Episode>> {
        let fetch = (n * DUB_HEADROOM).min(PAGE_MAX);
        let path =
            format!("/content/v2/discover/browse?type=episode&sort_by=newly_added&n={fetch}");
        let mut episodes: Vec<Episode> = self.page(&path).await?;
        let mut seen = HashSet::new();
        episodes.retain(|e| {
            e.episode_metadata
                .as_ref()
                .is_some_and(|m| seen.insert((m.series_id.clone(), m.episode.clone())))
        });
        episodes.truncate(n as usize);
        Ok(episodes)
    }

    pub async fn series(&self, id: &str) -> Result<Series> {
        let mut found: Vec<Series> = self.page(&format!("/content/v2/cms/series/{id}")).await?;
        found
            .pop()
            .ok_or_else(|| anyhow::anyhow!("series {id} not found"))
    }

    pub async fn seasons(&self, series_id: &str) -> Result<Vec<Season>> {
        self.page(&format!("/content/v2/cms/series/{series_id}/seasons"))
            .await
    }

    pub async fn episodes(&self, season_id: &str) -> Result<Vec<Episode>> {
        self.page(&format!("/content/v2/cms/seasons/{season_id}/episodes"))
            .await
    }
}
