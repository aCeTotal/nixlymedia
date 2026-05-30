use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;

use super::{is_vod_category, sort_no_first_quality_top, Channel};
use crate::api::Api;

#[derive(Deserialize)]
struct Category {
    category_id: String,
    category_name: String,
}

#[derive(Deserialize)]
struct LiveStream {
    name: String,
    stream_id: i64,
    #[serde(default)]
    stream_icon: Option<String>,
    #[serde(default)]
    category_id: Option<String>,
    #[serde(default)]
    epg_channel_id: Option<String>,
}

pub async fn fetch_channels(api: &Api, base: &str, user: &str, pass: &str) -> Result<Vec<Channel>> {
    let cats_url = format!(
        "{base}/player_api.php?username={user}&password={pass}&action=get_live_categories"
    );
    let streams_url = format!(
        "{base}/player_api.php?username={user}&password={pass}&action=get_live_streams"
    );

    let cats: Vec<Category> = api.get_json_url(&cats_url).await?;
    let streams: Vec<LiveStream> = api.get_json_url(&streams_url).await?;

    let cat_by_id: HashMap<String, String> = cats
        .into_iter()
        .map(|c| (c.category_id, c.category_name))
        .collect();

    let mut list: Vec<Channel> = streams
        .into_iter()
        .filter_map(|s| {
            let group = s.category_id.as_ref().and_then(|id| cat_by_id.get(id).cloned());
            if group.as_deref().map(is_vod_category).unwrap_or(false) {
                return None;
            }
            Some(Channel {
                url: format!("{base}/live/{user}/{pass}/{}.ts", s.stream_id),
                name: s.name,
                logo: s.stream_icon.filter(|x| !x.is_empty()),
                group,
                epg_id: s.epg_channel_id.filter(|x| !x.is_empty()),
            })
        })
        .collect();

    sort_no_first_quality_top(&mut list);
    Ok(list)
}
