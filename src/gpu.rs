/* På hybrid Intel+Nvidia velger Wayland-EGL compositorens GPU (i915/xe),
 * ikke dGPU-en. GLVND må tvinges til Nvidia-vendor FØR libEGL lastes —
 * gjelder hele prosessen (eframe/glow-UI + video-subsurface + libmpv).
 * Må derfor kalles i main() før eframe::run_native. */
pub fn force_nvidia_if_hybrid() {
    match std::env::var("NIXLY_GPU").ok().as_deref() {
        Some("intel") | Some("amd") => return,
        _ => {}
    }

    let drivers = crate::player::hwdec::drm_drivers();
    let has_nvidia = drivers.iter().any(|d| d.starts_with("nvidia"));
    let has_intel = drivers.iter().any(|d| d == "i915" || d == "xe");
    if !has_nvidia || !has_intel {
        return;
    }

    if !nvidia_egl_vendor_present() {
        crate::nlog!("hybrid GPU: nvidia kernel-driver funnet, men ingen GLVND egl_vendor.d-json — tvinger ikke");
        return;
    }

    /* Sett kun hvis ikke allerede satt, så brukeren kan overstyre.
     * __NV_PRIME_RENDER_OFFLOAD settes IKKE: fra driver 595 gir den
     * EGL_BAD_DISPLAY i eglGetPlatformDisplay på Wayland (GLX-mekanisme;
     * GLVND-vendor-valget under er nok for EGL). */
    for (k, v) in [
        ("__EGL_VENDOR_LIBRARY_NAMES", "nvidia"),
        ("__GLX_VENDOR_LIBRARY_NAME", "nvidia"),
        ("__VK_LAYER_NV_optimus", "NVIDIA_only"),
    ] {
        if std::env::var_os(k).is_none() {
            std::env::set_var(k, v);
        }
    }
    crate::nlog!("hybrid GPU (intel+nvidia): tvinger nvidia via GLVND/PRIME env");
}

/* GLVND laster kun vendorer med json i egl_vendor.d. Uten nvidia-json
 * ville __EGL_VENDOR_LIBRARY_NAMES=nvidia gitt null vendorer → EGL-init
 * feiler hardt. Sjekk standard-dirs + NixOS-path + evt. override-dir. */
fn nvidia_egl_vendor_present() -> bool {
    let mut dirs: Vec<String> = Vec::new();
    if let Ok(v) = std::env::var("__EGL_VENDOR_LIBRARY_DIRS") {
        dirs.extend(v.split(':').map(str::to_string));
    }
    dirs.extend(
        [
            "/etc/glvnd/egl_vendor.d",
            "/usr/share/glvnd/egl_vendor.d",
            "/run/opengl-driver/share/glvnd/egl_vendor.d",
        ]
        .map(str::to_string),
    );
    dirs.iter().any(|dir| {
        std::fs::read_dir(dir).map_or(false, |entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().contains("nvidia"))
        })
    })
}
