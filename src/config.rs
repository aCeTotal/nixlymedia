/* aceclan.no peker på egen offentlig IP; ruteren mangler hairpin-NAT, så
 * domenet er utilgjengelig fra LAN. Ved oppstart races begge (se
 * api::probe) og den som svarer først brukes. */
pub const SERVER_LAN: &str = "http://10.0.0.8:8080";
pub const SERVER_REMOTE: &str = "http://aceclan.no:8080";
pub const AUTH_USER: &str = "nixly";
pub const AUTH_PASS: &str = "nixlyadmin";

pub const SUB_LANG_PREFS: &str = "nor,nob,no,eng,en";
pub const AUDIO_LANG_PREFS: &str = "nor,eng";

pub const STATUS_POLL_SECS: u64 = 5;
pub const LIBRARY_POLL_SECS: u64 = 20;

/* Hold server-disken våken mens appen er idle i menyene. En HDD som har
 * spunnet ned bruker 3-7 s på å spinne opp; første demux etter lang idle
 * traff da tom buffer → frys/krasj. Hver KEEPALIVE en bitteliten Range-read
 * av en ekte mediefil tvinger server til disk-IO (sendfile åpner+leser
 * filen) og nullstiller spindown-timeren. 60 s < typisk 5-20 min spindown.
 * Kjører kun utenfor Player-skjermen; under avspilling holder selve
 * streamen disken våken. */
pub const DISK_KEEPALIVE_SECS: u64 = 60;

pub const IPTV_BASE: &str = "http://cf.deluxe-no.top";
pub const IPTV_USER: &str = "4c9e431d29";
pub const IPTV_PASS: &str = "1b6f14b927";

pub const EPG_REFRESH_SECS: u64 = 15 * 60;

/* Sekunder lyd skal forsinkes for å treffe video. Positiv = lyd senere.
 * Wayland-compositor (nixlytile) legger typisk 1-2 frame latency på video
 * mens ALSA hw direkte gir nær null lyd-latency. 0.040 s = ~1 frame @ 24
 * fps og er konservativt startpunkt. Bruker finjusterer live med [ / ]. */
pub const AUDIO_DELAY_DEFAULT: f64 = 0.040;
