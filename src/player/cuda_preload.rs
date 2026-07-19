/* ffmpeg/ffnvcodec dlopen'er libcuda.so.1 og libnvcuvid.so.1 med bare
 * SONAME. På NixOS ligger de i /run/opengl-driver/lib, som IKKE er i
 * ld-searchpath (EGL fungerer likevel — GLVND-json har absolutt path).
 * Resultat: "Failed to load CUDA symbols" → hwdec=nvdec faller til
 * software decode av 4K HEVC. Preload med absolutt path registrerer
 * SONAME i prosessen; ffmpegs senere dlopen(SONAME) treffer da den
 * allerede lastede lib-en. Libs holdes lastet for prosessens levetid. */

use libloading::Library;
use once_cell::sync::Lazy;

static CUDA_LIBS: Lazy<Vec<Library>> = Lazy::new(|| {
    let mut held = Vec::new();
    for soname in ["libcuda.so.1", "libnvcuvid.so.1"] {
        if let Some(lib) = load(soname) {
            held.push(lib);
        } else {
            crate::nlog!("cuda preload: {soname} ikke funnet — nvdec vil feile");
        }
    }
    held
});

fn load(soname: &str) -> Option<Library> {
    /* SONAME først: dekker systemer der driver-lib allerede er i path. */
    if let Ok(lib) = unsafe { Library::new(soname) } {
        return Some(lib);
    }
    for dir in ["/run/opengl-driver/lib", "/usr/lib64", "/usr/lib"] {
        let path = format!("{dir}/{soname}");
        if let Ok(lib) = unsafe { Library::new(&path) } {
            crate::nlog!("cuda preload: {path}");
            return Some(lib);
        }
    }
    None
}

/* Kall før mpv-init når hwdec=nvdec. Idempotent. */
pub fn ensure() {
    Lazy::force(&CUDA_LIBS);
}
