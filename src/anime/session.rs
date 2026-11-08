use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use parking_lot::Mutex;
use serde_json::{json, Value};

use super::cdp::{Cdp, ChildFds, CHROME_CMD_FD, CHROME_REPLY_FD};
use crate::input::gamepad::Action;
use crate::player::nixlytile_ipc::VideoRate;

const BROWSER: &str = "google-chrome-stable";
/* Crunchyroll anime is film-rate. */
const CONTENT_FPS: f64 = 23.976;
const POLL: Duration = Duration::from_secs(1);
const CLOSE_GRACE: Duration = Duration::from_secs(3);
const VIDEO_STATE: &str =
    "(() => { const v = document.querySelector('video'); return v ? [v.currentTime, v.duration] : null })()";

#[derive(Clone, Copy)]
enum Key {
    PlayPause,
    Back,
    Forward,
    Fullscreen,
}

impl Key {
    /* Chrome key, code and keycode. */
    fn spec(self) -> (&'static str, &'static str, u32) {
        match self {
            Key::PlayPause => (" ", "Space", 32),
            Key::Back => ("ArrowLeft", "ArrowLeft", 37),
            Key::Forward => ("ArrowRight", "ArrowRight", 39),
            Key::Fullscreen => ("f", "KeyF", 70),
        }
    }
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub episode_id: Option<String>,
    pub position: f64,
    pub duration: f64,
}

/* Official player in kiosk browser. */
pub struct Session {
    stop: Arc<AtomicBool>,
    snapshot: Arc<Mutex<Snapshot>>,
    ended: Arc<AtomicBool>,
    rate: VideoRate,
}

impl Session {
    /* Pad actions while browser focused. */
    pub fn start(url: &str, pad: Receiver<Action>) -> Result<Session> {
        let (cdp, fds) = Cdp::pipes()?;
        let child = spawn_browser(url, fds)?;
        let stop = Arc::new(AtomicBool::new(false));
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let ended = Arc::new(AtomicBool::new(false));
        let worker = Worker {
            cdp,
            child,
            page: None,
            snapshot: snapshot.clone(),
            stop: stop.clone(),
            fullscreen: false,
        };
        let done = ended.clone();
        std::thread::Builder::new()
            .name("nixlymedia-crunchyroll".into())
            .spawn(move || {
                worker.run(pad);
                done.store(true, Ordering::Release);
            })?;
        let mut rate = VideoRate::new(None);
        rate.playing(CONTENT_FPS);
        Ok(Session {
            stop,
            snapshot,
            ended,
            rate,
        })
    }

    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().clone()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.rate.stopped();
    }
}

fn spawn_browser(url: &str, fds: ChildFds) -> Result<Child> {
    let profile = dirs::home_dir()
        .unwrap_or_default()
        .join(".local/share/nixlymedia/chrome");
    let (cmd_read, reply_write) = (fds.cmd_read.as_raw_fd(), fds.reply_write.as_raw_fd());
    let mut command = Command::new(BROWSER);
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .args([
            "--kiosk",
            "--remote-debugging-pipe",
            "--no-first-run",
            "--no-default-browser-check",
            "--password-store=basic",
            "--autoplay-policy=no-user-gesture-required",
            "--ozone-platform-hint=auto",
        ])
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            place(cmd_read, CHROME_CMD_FD)?;
            place(reply_write, CHROME_REPLY_FD)
        });
    }
    let child = command.spawn()?;
    drop(fds);
    Ok(child)
}

/* Exec-safe fd placement. */
fn place(fd: i32, target: i32) -> std::io::Result<()> {
    let ok = if fd == target {
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) >= 0 }
    } else {
        unsafe { libc::dup2(fd, target) >= 0 }
    };
    if ok {
        return Ok(());
    }
    Err(std::io::Error::last_os_error())
}

struct Worker {
    cdp: Cdp,
    child: Child,
    page: Option<String>,
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    /* Player fullscreened on first frame. */
    fullscreen: bool,
}

impl Worker {
    fn run(mut self, pad: Receiver<Action>) {
        while !self.stop.load(Ordering::Acquire) {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            match pad.recv_timeout(POLL) {
                Ok(Action::Back) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(action) => self.handle(action),
                Err(RecvTimeoutError::Timeout) => self.poll(),
            }
        }
        let _ = self.cdp.call("Browser.close", json!({}), None);
        let deadline = std::time::Instant::now() + CLOSE_GRACE;
        while matches!(self.child.try_wait(), Ok(None)) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn poll(&mut self) {
        let Ok(targets) = self.cdp.call("Target.getTargets", json!({}), None) else {
            return;
        };
        let page = targets["targetInfos"]
            .as_array()
            .and_then(|all| all.iter().find(|t| t["type"] == "page"))
            .cloned();
        let Some(page) = page else { return };
        let episode_id = page["url"].as_str().and_then(episode_from_url);
        if self.page.is_none() {
            let attached = self.cdp.call(
                "Target.attachToTarget",
                json!({ "targetId": page["targetId"], "flatten": true }),
                None,
            );
            self.page = attached
                .ok()
                .and_then(|r| r["sessionId"].as_str().map(str::to_owned));
        }
        let times = self.page.as_deref().and_then(|s| {
            self.cdp
                .call(
                    "Runtime.evaluate",
                    json!({ "expression": VIDEO_STATE, "returnByValue": true }),
                    Some(s),
                )
                .ok()
        });
        let video = times
            .as_ref()
            .and_then(|r| video_times(&r["result"]["value"]));
        {
            let mut snap = self.snapshot.lock();
            if episode_id.is_some() {
                snap.episode_id = episode_id;
            }
            if let Some([pos, dur]) = video {
                snap.position = pos;
                snap.duration = dur;
            }
        }
        if video.is_some() && !std::mem::replace(&mut self.fullscreen, true) {
            self.press(Key::Fullscreen);
        }
    }

    fn handle(&self, action: Action) {
        let key = match action {
            Action::PlayPause | Action::Confirm => Key::PlayPause,
            Action::Left => Key::Back,
            Action::Right => Key::Forward,
            Action::Skip(s) if s < 0.0 => Key::Back,
            Action::Skip(_) => Key::Forward,
            _ => return,
        };
        self.press(key);
    }

    fn press(&self, key: Key) {
        let Some(session) = self.page.as_deref() else {
            return;
        };
        let (key, code, vk) = key.spec();
        for kind in ["keyDown", "keyUp"] {
            let params =
                json!({ "type": kind, "key": key, "code": code, "windowsVirtualKeyCode": vk });
            let _ = self
                .cdp
                .call("Input.dispatchKeyEvent", params, Some(session));
        }
    }
}

/* https://www.crunchyroll.com/watch/<id>/<slug> */
fn episode_from_url(url: &str) -> Option<String> {
    let rest = url.split("/watch/").nth(1)?;
    let id = rest.split(['/', '?']).next()?;
    (!id.is_empty()).then(|| id.to_string())
}

fn video_times(v: &Value) -> Option<[f64; 2]> {
    let a = v.as_array()?;
    let pos = a.first()?.as_f64()?;
    let dur = a.get(1)?.as_f64().filter(|d| d.is_finite() && *d > 0.0)?;
    Some([pos, dur])
}
