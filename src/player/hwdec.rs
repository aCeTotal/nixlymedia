pub(crate) fn drm_drivers() -> Vec<String> {
    let mut drivers: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("card") || name.contains('-') {
                continue;
            }
            let driver_link = entry.path().join("device/driver");
            if let Ok(target) = std::fs::read_link(&driver_link) {
                if let Some(d) = target.file_name().and_then(|s| s.to_str()) {
                    drivers.push(d.to_string());
                }
            }
        }
    }
    drivers
}

fn forced_gpu() -> Option<&'static str> {
    match std::env::var("NIXLY_GPU").ok().as_deref() {
        Some("intel") => Some("intel"),
        Some("nvidia") => Some("nvidia"),
        Some("amd") => Some("amd"),
        _ => None,
    }
}

/* render_driver: DRM-driver for GPU-en som backer EGL-konteksten
 * (SubsurfaceVideo::render_driver). Autoritativ på hybrid-GPU — sysfs-
 * scan sier bare hvilke GPU-er som FINNES, ikke hvilken som renderer.
 * None = ukjent → fall tilbake til sysfs-heuristikk. */
pub fn detect(render_driver: Option<&str>) -> &'static str {
    if let Some(g) = forced_gpu() {
        return match g {
            "intel" => "vaapi",
            "nvidia" => "nvdec",
            "amd" => "vaapi",
            _ => "auto",
        };
    }
    if let Some(d) = render_driver {
        if d.starts_with("nvidia") {
            return "nvdec";
        }
        if d == "amdgpu" {
            return "vaapi";
        }
        if d == "xe" {
            return "vaapi-copy";
        }
        if d == "i915" {
            return "vaapi";
        }
    }
    let drivers = drm_drivers();
    if drivers.iter().any(|d| d.starts_with("nvidia")) {
        return "nvdec";
    }
    if drivers.iter().any(|d| d == "amdgpu") {
        return "vaapi";
    }
    if drivers.iter().any(|d| d == "xe") {
        return "vaapi-copy";
    }
    if drivers.iter().any(|d| d == "i915") {
        return "vaapi";
    }
    "auto"
}

pub fn is_intel_igpu_active(render_driver: Option<&str>) -> bool {
    if let Some(g) = forced_gpu() {
        return g == "intel";
    }
    if let Some(d) = render_driver {
        return d == "i915" || d == "xe";
    }
    let drivers = drm_drivers();
    let has_intel = drivers.iter().any(|d| d == "i915" || d == "xe");
    let has_discrete = drivers.iter().any(|d| d.starts_with("nvidia") || d == "amdgpu");
    has_intel && !has_discrete
}
