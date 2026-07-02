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

    /* Måler reell nedlastingshastighet mot stream-endepunktet og leser
     * total filstørrelse fra Content-Range (206) / Content-Length (200).
     *
     * Hastighet beregnes steady-state: første MiB (TCP slow-start +
     * connection setup) holdes utenfor målevinduet, ellers under-
     * rapporteres raske linjer kraftig og buffer-policy blir feil.
     * Stopper tidlig når vi har nok sample (>= 4 s og >= 2 MiB) så
     * trege linjer ikke bruker evigheter på selve målingen.
     *
     * Returnerer (bytes/s, total filstørrelse). */
    pub async fn probe_bandwidth_stream<F>(
        &self,
        stream_id: i64,
        bytes_target: u64,
        mut on_progress: F,
    ) -> Result<(f64, Option<u64>)>
    where
        F: FnMut(u64, f64) + Send,
    {
        use futures_util::StreamExt;
        const WARMUP_BYTES: u64 = 1024 * 1024;
        const MIN_SAMPLE_SECS: f64 = 4.0;
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
        let file_size = if resp.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            /* "Content-Range: bytes 0-8388607/8123456789" → total etter '/'. */
            resp.headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.rsplit('/').next())
                .and_then(|s| s.trim().parse::<u64>().ok())
        } else {
            /* Server ignorerte Range → 200 med hele filen. */
            resp.content_length()
        };
        let mut stream = resp.bytes_stream();
        let mut total: u64 = 0;
        /* (bytes, tidspunkt) der warmup-vinduet sluttet. */
        let mut warmup: Option<(u64, f64)> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            total += chunk.len() as u64;
            let elapsed = started.elapsed().as_secs_f64();
            if warmup.is_none() && total >= WARMUP_BYTES {
                warmup = Some((total, elapsed));
            }
            on_progress(total, elapsed);
            if total >= bytes_target {
                break;
            }
            if elapsed >= MIN_SAMPLE_SECS && total >= 2 * WARMUP_BYTES {
                break;
            }
        }
        let elapsed = started.elapsed().as_secs_f64().max(1e-6);
        let speed = match warmup {
            Some((wb, wt)) if total > wb && elapsed > wt + 0.2 => {
                (total - wb) as f64 / (elapsed - wt)
            }
            _ => total as f64 / elapsed,
        };
        Ok((speed, file_size))
    }

    pub fn stream_url(&self, id: i64) -> String {
        format!("{}/stream/{}", self.base, id)
    }

    /* Tving server til å åpne+lese en ekte mediefil slik at media-HDD-en
     * holdes våken (spindown-timer nullstilles). Bittelite Range så vi
     * ikke laster ned noe av betydning; body droppes. Feil ignoreres —
     * dette er best-effort keepalive, ikke kritisk sti. */
    pub async fn warm_disk(&self, id: i64) -> Result<()> {
        let url = format!("{}/stream/{}", self.base, id);
        let _ = self
            .http
            .get(&url)
            .header("Authorization", &self.auth_header)
            .header("Range", "bytes=0-1023")
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
