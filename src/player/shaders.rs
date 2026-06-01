/* Embedded GLSL-shaders. include_bytes! ved kompiletid, skriv til
 * XDG_CACHE_HOME/nixlymedia/shaders ved første kall, returner path til
 * mpv via glsl-shaders property.
 *
 * Hvorfor disk: mpv glsl-shaders tar filsti, ikke inline kildekode.
 * Cache-skriving idempotent — overwriter bare hvis innhold endret. */

use std::io::Write;
use std::path::PathBuf;

/* KrigBilateral.glsl (LGPL-3.0, av Shiandow/igv). Lisens-header bevart
 * i shader-filen. Brukere kan erstatte filen i cache-dir for å swappe
 * eller modifisere — oppfyller LGPL-vilkår. */
const KRIG_BILATERAL: &str = include_str!("shaders/KrigBilateral.glsl");

fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(dirs::cache_dir)?;
    Some(base.join("nixlymedia").join("shaders"))
}

fn write_if_changed(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == content {
            return Ok(());
        }
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(content.as_bytes())?;
    Ok(())
}

/// Skriv KrigBilateral til cache, returner path som string, eller None
/// ved IO-feil (mpv kjører da uten shader, faller tilbake til cscale=mitchell).
pub fn krig_bilateral_path() -> Option<String> {
    let dir = cache_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("KrigBilateral.glsl");
    if let Err(e) = write_if_changed(&path, KRIG_BILATERAL) {
        crate::nlog!("shader cache write failed: {e}");
        return None;
    }
    path.to_str().map(String::from)
}
