pub mod epg;
pub mod xtream;

pub use epg::EpgIndex;
pub use xtream::fetch_channels;

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub logo: Option<String>,
    pub group: Option<String>,
    pub url: String,
    pub epg_id: Option<String>,
}

pub fn sort_no_first_quality_top(list: &mut [Channel]) {
    list.sort_by(|a, b| {
        let na = is_norwegian(a);
        let nb = is_norwegian(b);
        nb.cmp(&na)
            .then_with(|| quality_rank(b).cmp(&quality_rank(a)))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn is_norwegian(c: &Channel) -> bool {
    let probe = |s: &str| {
        let up = s.to_uppercase();
        up.contains("NORWAY")
            || up.contains("NORGE")
            || up.contains("NORSK")
            || up.starts_with("NO ")
            || up.starts_with("NO|")
            || up.starts_with("NO:")
            || up.contains("|NO|")
            || up.contains(" NO ")
    };
    c.group.as_deref().map(probe).unwrap_or(false) || probe(&c.name)
}

fn quality_rank(c: &Channel) -> u8 {
    let hay = format!("{} {}", c.name, c.group.as_deref().unwrap_or("")).to_uppercase();
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

/// True if category name looks like VOD/movie/series content rather than live TV.
pub fn is_vod_category(name: &str) -> bool {
    let up = name.to_uppercase();
    const NEEDLES: &[&str] = &[
        "VOD", "MOVIE", "MOVIES", "FILM", "FILMER", "FILMS",
        "SERIE", "SERIES", "SERIER", "SHOW SERIE",
    ];
    NEEDLES.iter().any(|n| up.contains(n))
}
