use anyhow::Result;

use crate::api::Api;

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub logo: Option<String>,
    pub group: Option<String>,
    pub url: String,
}

pub async fn fetch_channels(api: &Api, m3u_url: &str) -> Result<Vec<Channel>> {
    let txt = api.get_text_url(m3u_url).await?;
    let mut list = parse(&txt);
    sort_no_first_quality_top(&mut list);
    Ok(list)
}

pub fn parse(s: &str) -> Vec<Channel> {
    let mut out = Vec::new();
    let mut pending: Option<PartialChannel> = None;
    for line in s.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXTINF") {
            pending = Some(parse_extinf(rest));
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(p) = pending.take() {
            if p.name.is_empty() {
                continue;
            }
            out.push(Channel {
                name: p.name,
                logo: p.logo,
                group: p.group,
                url: line.to_string(),
            });
        }
    }
    out
}

struct PartialChannel {
    name: String,
    logo: Option<String>,
    group: Option<String>,
}

fn parse_extinf(rest: &str) -> PartialChannel {
    let rest = rest.trim_start_matches(':').trim_start();
    let (attrs_part, name) = split_extinf_outside_quotes(rest);
    PartialChannel {
        name: name.trim().to_string(),
        logo: extract_attr(attrs_part, "tvg-logo"),
        group: extract_attr(attrs_part, "group-title"),
    }
}

fn split_extinf_outside_quotes(rest: &str) -> (&str, &str) {
    let bytes = rest.as_bytes();
    let mut in_quote = false;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' {
            in_quote = !in_quote;
        }
        if !in_quote && b == b',' {
            return (&rest[..i], &rest[i + 1..]);
        }
    }
    (rest, "")
}

fn extract_attr(s: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = s.find(&needle)?;
    let after = &s[start + needle.len()..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

fn is_norwegian(c: &Channel) -> bool {
    let probe = |s: &str| {
        let up = s.to_uppercase();
        up.contains("NORWAY")
            || up.contains("NORGE")
            || up.contains("NORSK")
            || up.starts_with("NO ")
            || up.starts_with("NO|")
            || up.contains("|NO|")
            || up.contains(" NO ")
    };
    c.group.as_deref().map(probe).unwrap_or(false) || probe(&c.name)
}

fn quality_rank(c: &Channel) -> u8 {
    let hay = format!(
        "{} {}",
        c.name,
        c.group.as_deref().unwrap_or("")
    )
    .to_uppercase();
    if hay.contains("4K") || hay.contains("UHD") {
        4
    } else if hay.contains("FHD") || hay.contains("1080") {
        3
    } else if hay.contains(" HD") || hay.contains("HD ") || hay.ends_with("HD") || hay.contains("|HD") {
        2
    } else if hay.contains(" SD") || hay.ends_with("SD") {
        1
    } else {
        0
    }
}

fn sort_no_first_quality_top(list: &mut [Channel]) {
    list.sort_by(|a, b| {
        let na = is_norwegian(a);
        let nb = is_norwegian(b);
        nb.cmp(&na)
            .then_with(|| quality_rank(b).cmp(&quality_rank(a)))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}
