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
    let mut sport_brands: std::collections::HashSet<String> = std::collections::HashSet::new();
    for c in list.iter() {
        if is_norwegian(c) && is_sport(c) {
            sport_brands.insert(brand_key(&c.name));
        }
    }

    list.sort_by(|a, b| {
        let ta = tier(a, &sport_brands);
        let tb = tier(b, &sport_brands);
        ta.cmp(&tb)
            .then_with(|| {
                if ta == 0 {
                    let ba = brand_key(&a.name);
                    let bb = brand_key(&b.name);
                    ba.cmp(&bb).then_with(|| {
                        let sa = if is_sport(a) { 0 } else { 1 };
                        let sb = if is_sport(b) { 0 } else { 1 };
                        sa.cmp(&sb)
                    })
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .then_with(|| quality_rank(b).cmp(&quality_rank(a)))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn tier(c: &Channel, sport_brands: &std::collections::HashSet<String>) -> u8 {
    let no = is_norwegian(c);
    if no {
        if sport_brands.contains(&brand_key(&c.name)) {
            return 0;
        }
        return 1;
    }
    if is_scandinavian(c) || is_english_language(c) {
        return 2;
    }
    3
}

fn brand_key(name: &str) -> String {
    let up = name.to_uppercase();
    let cleaned: String = up
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let tokens: Vec<&str> = cleaned
        .split_whitespace()
        .filter(|t| *t != "NO")
        .collect();
    if tokens.is_empty() {
        return String::new();
    }
    if tokens[0] == "TV" && tokens.len() >= 2 && tokens[1].chars().all(|c| c.is_ascii_digit()) {
        return format!("TV{}", tokens[1]);
    }
    tokens[0].to_string()
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

fn is_scandinavian(c: &Channel) -> bool {
    let probe = |s: &str| {
        let up = s.to_uppercase();
        up.contains("SWEDEN")
            || up.contains("SVERIGE")
            || up.contains("SVENSK")
            || up.contains("DENMARK")
            || up.contains("DANMARK")
            || up.contains("DANSK")
            || code_match(&up, "SE")
            || code_match(&up, "SV")
            || code_match(&up, "DK")
            || code_match(&up, "DA")
    };
    c.group.as_deref().map(probe).unwrap_or(false) || probe(&c.name)
}

fn is_english_language(c: &Channel) -> bool {
    let probe = |s: &str| {
        let up = s.to_uppercase();
        up.contains("ENGLISH")
            || up.contains("UNITED KINGDOM")
            || up.contains("UNITED STATES")
            || code_match(&up, "UK")
            || code_match(&up, "GB")
            || code_match(&up, "EN")
            || code_match(&up, "US")
            || code_match(&up, "USA")
            || code_match(&up, "IE")
            || code_match(&up, "AU")
            || code_match(&up, "CA")
            || code_match(&up, "NZ")
    };
    c.group.as_deref().map(probe).unwrap_or(false) || probe(&c.name)
}

fn code_match(up: &str, code: &str) -> bool {
    up.starts_with(&format!("{code} "))
        || up.starts_with(&format!("{code}|"))
        || up.starts_with(&format!("{code}:"))
        || up.contains(&format!("|{code}|"))
        || up.contains(&format!(" {code} "))
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
