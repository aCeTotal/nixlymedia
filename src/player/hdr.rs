use libmpv2::Mpv;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transfer {
    Sdr,
    Pq,
    Hlg,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Primaries {
    Bt709,
    Bt2020,
    DciP3,
    Other,
}

#[derive(Clone, Debug)]
pub struct HdrMeta {
    pub transfer: Transfer,
    pub primaries: Primaries,
    pub mastering_min_luminance: Option<f64>,
    pub mastering_max_luminance: Option<f64>,
    pub max_cll: Option<f64>,
    pub max_fall: Option<f64>,
    pub sig_peak: Option<f64>,
}

impl HdrMeta {
    pub fn is_hdr(&self) -> bool {
        matches!(self.transfer, Transfer::Pq | Transfer::Hlg)
    }
}

fn parse_transfer(s: &str) -> Transfer {
    match s {
        "pq" | "smpte2084" | "st2084" => Transfer::Pq,
        "hlg" | "arib-std-b67" => Transfer::Hlg,
        _ => Transfer::Sdr,
    }
}

fn parse_primaries(s: &str) -> Primaries {
    match s {
        "bt.709" | "bt709" => Primaries::Bt709,
        "bt.2020" | "bt2020" => Primaries::Bt2020,
        "dci-p3" | "display-p3" => Primaries::DciP3,
        _ => Primaries::Other,
    }
}

pub fn detect(mpv: &Mpv) -> Option<HdrMeta> {
    let gamma = mpv.get_property::<String>("video-params/gamma").ok()?;
    let prim = mpv
        .get_property::<String>("video-params/primaries")
        .unwrap_or_default();
    let transfer = parse_transfer(&gamma);
    let primaries = parse_primaries(&prim);
    let sig_peak = mpv.get_property::<f64>("video-params/sig-peak").ok();
    let max_cll = mpv
        .get_property::<f64>("video-params/max-cll")
        .ok()
        .or_else(|| mpv.get_property::<f64>("video-params/max-content-light").ok());
    let max_fall = mpv
        .get_property::<f64>("video-params/max-fall")
        .ok()
        .or_else(|| mpv.get_property::<f64>("video-params/max-frame-light").ok());
    let mastering_min_luminance = mpv
        .get_property::<f64>("video-params/min-luma")
        .ok()
        .or_else(|| mpv.get_property::<f64>("video-params/mastering-display-min-luma").ok());
    let mastering_max_luminance = mpv
        .get_property::<f64>("video-params/max-luma")
        .ok()
        .or_else(|| mpv.get_property::<f64>("video-params/mastering-display-max-luma").ok());

    Some(HdrMeta {
        transfer,
        primaries,
        mastering_min_luminance,
        mastering_max_luminance,
        max_cll,
        max_fall,
        sig_peak,
    })
}
