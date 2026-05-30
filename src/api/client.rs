use anyhow::Result;
use base64::Engine;
use bytes::Bytes;
use reqwest::Client;

use crate::config;

#[derive(Clone)]
pub struct Api {
    http: Client,
    base: String,
    auth_header: String,
}

impl Api {
    pub fn new() -> Self {
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", config::AUTH_USER, config::AUTH_PASS));
        let auth_header = format!("Basic {token}");
        let http = Client::builder()
            .user_agent("nixlymedia/0.1")
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("reqwest client");
        Self {
            http,
            base: config::SERVER_BASE.to_string(),
            auth_header,
        }
    }

    pub fn auth_header(&self) -> &str {
        &self.auth_header
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.json::<T>().await?)
    }

    pub async fn get_json_url<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("HTTP {} fra {}", status.as_u16(), url);
        }
        Ok(resp.json::<T>().await?)
    }

    pub async fn get_bytes_url(&self, url: &str) -> Result<Bytes> {
        let resp = self.http.get(url).send().await?.error_for_status()?;
        Ok(resp.bytes().await?)
    }

    pub async fn get_bytes(&self, path: &str) -> Result<Bytes> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .send()
            .await?
            .error_for_status()?;
        Ok(resp.bytes().await?)
    }

    pub async fn probe_bandwidth_stream<F>(
        &self,
        stream_id: i64,
        bytes_target: u64,
        mut on_progress: F,
    ) -> Result<f64>
    where
        F: FnMut(u64, f64) + Send,
    {
        use futures_util::StreamExt;
        let url = format!("{}/stream/{}", self.base, stream_id);
        let end = bytes_target.saturating_sub(1);
        let started = std::time::Instant::now();
        let resp = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Range", format!("bytes=0-{end}"))
            .send()
            .await?
            .error_for_status()?;
        let mut stream = resp.bytes_stream();
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            total += chunk.len() as u64;
            let elapsed = started.elapsed().as_secs_f64();
            on_progress(total, elapsed);
            if total >= bytes_target {
                break;
            }
        }
        let elapsed = started.elapsed().as_secs_f64().max(1e-6);
        Ok((total as f64) / elapsed)
    }

    pub fn stream_url(&self, id: i64) -> String {
        format!("{}/stream/{}", self.base, id)
    }
}
