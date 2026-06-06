use std::ffi::c_void;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::sync::OnceLock;

/* Signal-handler som dumper backtrace til samme loggfil som NIXLY_LOG.
 *
 * Mpv, NVDEC og libva-driverne kan SIGSEGV/SIGABRT i C-koden uten at Rust
 * panic-hook fanger det — prosessen dør med uflushede mpv-logger og vår
 * applog også. Stack-traces fra C-bibliotek er det eneste signalet vi
 * har for å diagnostisere disse krasjene.
 *
 * Implementasjonen bruker kun async-signal-safe funksjoner: write(2),
 * libc::backtrace, libc::backtrace_symbols_fd. Ingen malloc, ingen
 * std::fmt::Display (kan allokere). Etter dump re-raises signalet med
 * default handler så core-dump kan tas av OS hvis konfigurert. */

static LOG_FD: OnceLock<i32> = OnceLock::new();

fn open_log_fd() -> Option<i32> {
    let path = crate::log::path();
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    let fd = file.as_raw_fd();
    /* Leak filen så fd-en forblir gyldig til prosess slutt. Vi bruker den
     * kun fra signal-handlere; ingen normal close-sti er trygg uansett. */
    std::mem::forget(file);
    Some(fd)
}

extern "C" fn handler(sig: libc::c_int) {
    let fd = *LOG_FD.get().unwrap_or(&libc::STDERR_FILENO);

    let header: &[u8] = match sig {
        libc::SIGSEGV => b"\n=== SIGSEGV - segmentation fault ===\n",
        libc::SIGABRT => b"\n=== SIGABRT - abort (glibc / libplacebo / mpv assert) ===\n",
        libc::SIGBUS => b"\n=== SIGBUS - bus error ===\n",
        libc::SIGFPE => b"\n=== SIGFPE - floating-point exception ===\n",
        libc::SIGILL => b"\n=== SIGILL - illegal instruction ===\n",
        _ => b"\n=== fatal signal ===\n",
    };
    unsafe {
        libc::write(fd, header.as_ptr() as *const c_void, header.len());

        /* Kap stack ved 64 frames — nok til å spore mpv → libavcodec →
         * NVDEC eller wp_color_manager-stack. */
        let mut frames: [*mut c_void; 64] = [std::ptr::null_mut(); 64];
        let n = libc::backtrace(frames.as_mut_ptr(), frames.len() as libc::c_int);
        if n > 0 {
            libc::backtrace_symbols_fd(frames.as_ptr(), n, fd);
        }

        let footer: &[u8] = b"=== end backtrace ===\n";
        libc::write(fd, footer.as_ptr() as *const c_void, footer.len());
        libc::fsync(fd);

        /* Reinstaller default-handler og re-raise. OS leverer
         * core-dump hvis ulimit -c tillater. */
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

pub fn install() {
    if !crate::log::enabled() {
        return;
    }
    let fd = match open_log_fd() {
        Some(f) => f,
        None => {
            crate::nlog!("crash handler: kunne ikke åpne loggfil — backtrace vil gå til stderr");
            libc::STDERR_FILENO
        }
    };
    let _ = LOG_FD.set(fd);

    /* SIGSEGV/SIGABRT/SIGBUS = de tre mest sannsynlige fra mpv/NVDEC. */
    for sig in [libc::SIGSEGV, libc::SIGABRT, libc::SIGBUS, libc::SIGFPE, libc::SIGILL] {
        unsafe {
            libc::signal(sig, handler as *const () as libc::sighandler_t);
        }
    }

    /* Rust panic → samme logg, med backtrace. */
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::io::stderr().flush();
        crate::nlog!("=== RUST PANIC === {info}");
        let bt = std::backtrace::Backtrace::force_capture();
        crate::nlog!("backtrace:\n{bt}");
        default(info);
    }));

    crate::nlog!("crash handler installed (SIGSEGV/SIGABRT/SIGBUS/SIGFPE/SIGILL + panic)");
}
