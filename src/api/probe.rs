use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::config;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/* Velger server-URL ved oppstart: racer TCP-connect mot LAN-IP og domenet
 * parallelt og bruker den som svarer først. Hjemme svarer LAN på ~1 ms mens
 * domenet henger (hairpin-NAT); utenfor hjemmenettet er det omvendt. Svarer
 * ingen (nett ikke oppe ved boot) antas hjemme — LAN er primærbruken. */
pub fn pick_base() -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    for base in [config::SERVER_LAN, config::SERVER_REMOTE] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            if reachable(base) {
                let _ = tx.send(base);
            }
        });
    }
    drop(tx);
    /* Litt slakk over CONNECT_TIMEOUT så DNS-oppslag rekker å feile selv. */
    let base = rx
        .recv_timeout(CONNECT_TIMEOUT + Duration::from_millis(500))
        .unwrap_or(config::SERVER_LAN);
    crate::nlog!("api: valgte server-base {base}");
    base.to_string()
}

fn reachable(base: &str) -> bool {
    let hostport = base.trim_start_matches("http://");
    let Ok(addrs) = hostport.to_socket_addrs() else {
        return false;
    };
    addrs
        .into_iter()
        .any(|a| TcpStream::connect_timeout(&a, CONNECT_TIMEOUT).is_ok())
}
