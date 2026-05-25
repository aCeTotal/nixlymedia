pub fn detect() -> &'static str {
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
    if drivers.iter().any(|d| d.starts_with("nvidia")) {
        return "nvdec";
    }
    if drivers.iter().any(|d| d == "amdgpu") {
        return "vaapi";
    }
    if drivers.iter().any(|d| d == "xe") {
        return "vaapi";
    }
    if drivers.iter().any(|d| d == "i915") {
        return "vaapi";
    }
    "auto"
}
