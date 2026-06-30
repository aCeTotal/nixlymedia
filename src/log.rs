use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::OnceLock;
use std::time::Instant;

use parking_lot::Mutex;

static LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

const DEFAULT_PATH: &str = "/tmp/nixlymedia.log";

pub fn path() -> String {
    std::env::var("NIXLY_LOG_FILE").unwrap_or_else(|_| DEFAULT_PATH.into())
}

pub fn init() {
    let _ = START.set(Instant::now());
    /* På som standard ved normal oppstart. Skru av eksplisitt med
     * NIXLY_LOG=0/false/no. */
    let enabled = !matches!(
        std::env::var("NIXLY_LOG").ok().as_deref(),
        Some("0") | Some("false") | Some("no")
    );
    let f = if enabled {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path())
            .ok()
            .map(Mutex::new)
    } else {
        None
    };
    let _ = LOG.set(f);
    if enabled {
        write(&format!("=== nixlymedia log start (path={}) ===", path()));
    }
}

pub fn enabled() -> bool {
    matches!(LOG.get(), Some(Some(_)))
}

pub fn write(msg: &str) {
    let ts = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let line = format!("[{ts:8.3}] {msg}");
    eprintln!("{line}");
    if let Some(Some(f)) = LOG.get() {
        let mut g = f.lock();
        let _ = writeln!(g, "{line}");
        let _ = g.flush();
    }
}

#[macro_export]
macro_rules! nlog {
    ($($arg:tt)*) => { $crate::log::write(&format!($($arg)*)) };
}
