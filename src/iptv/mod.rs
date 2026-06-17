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

/// Norwegian channels sorted as before, with FOX Sports / FOX One and BBC 4K sport
/// channels appended, then FIFA World Cup channels, then adult channels at the bottom.
pub fn arrange_channels(mut list: Vec<Channel>) -> Vec<Channel> {
    let mut extra: Vec<Channel> = list
        .iter()
        .filter(|c| (is_fox(c) || is_bbc_4k_sport(c)) && !is_norwegian(c))
        .cloned()
        .collect();
    let mut fifa: Vec<Channel> = list
        .iter()
        .filter(|c| is_fifa(c) && !is_norwegian(c))
        .cloned()
        .collect();
    let mut adult: Vec<Channel> = list.iter().filter(|c| is_adult(c)).cloned().collect();
    sort_no_first_quality_top(&mut list);
    extra.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    fifa.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    adult.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list.extend(extra);
    list.extend(fifa);
    list.extend(adult);
    list
}

fn is_fifa(c: &Channel) -> bool {
    let hay = format!("{} {}", c.name, c.group.as_deref().unwrap_or("")).to_uppercase();
    hay.contains("FIFA") || hay.contains("WORLD CUP") || hay.contains("WORLDCUP")
}

fn is_adult(c: &Channel) -> bool {
    let hay = format!("{} {}", c.name, c.group.as_deref().unwrap_or("")).to_uppercase();
    hay.contains("ADULT")
        || hay.contains("XXX")
        || hay.contains("18+")
        || hay.contains("PORN")
}

fn is_fox(c: &Channel) -> bool {
    let hay = format!("{} {}", c.name, c.group.as_deref().unwrap_or("")).to_uppercase();
    hay.contains("FOX SPORTS") || hay.contains("FOX ONE")
}

fn is_bbc_4k_sport(c: &Channel) -> bool {
    let hay = format!("{} {}", c.name, c.group.as_deref().unwrap_or("")).to_uppercase();
    hay.contains("BBC")
        && (hay.contains("4K") || hay.contains("UHD"))
        && (hay.contains("SPORT") || hay.contains("EUROSPORT"))
}

pub fn sort_no_first_quality_top(list: &mut Vec<Channel>) {
    list.retain(is_norwegian);
    list.sort_by(|a, b| {
        let sa = if is_sport(a) { 0 } else { 1 };
        let sb = if is_sport(b) { 0 } else { 1 };
        sa.cmp(&sb)
            .then_with(|| quality_rank(b).cmp(&quality_rank(a)))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn is_sport(c: &Channel) -> bool {
    let probe = |s: &str| {
        let up = s.to_uppercase();
        up.contains("SPORT") || up.contains("EUROSPORT")
    };
    c.group.as_deref().map(probe).unwrap_or(false) || probe(&c.name)
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
