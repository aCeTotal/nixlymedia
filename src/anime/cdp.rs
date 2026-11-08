use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};

/* Chrome pipe fds: commands, replies. */
pub const CHROME_CMD_FD: i32 = 3;
pub const CHROME_REPLY_FD: i32 = 4;
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

type Waiters = Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>;

/* DevTools protocol over --remote-debugging-pipe. */
pub struct Cdp {
    tx: Mutex<File>,
    next_id: AtomicU64,
    waiters: Waiters,
}

/* Pipe ends for the browser. */
pub struct ChildFds {
    pub cmd_read: OwnedFd,
    pub reply_write: OwnedFd,
}

impl Cdp {
    pub fn pipes() -> Result<(Cdp, ChildFds)> {
        let (cmd_read, cmd_write) = pipe()?;
        let (reply_read, reply_write) = pipe()?;
        let waiters: Waiters = Arc::new(Mutex::new(HashMap::new()));
        spawn_reader(File::from(reply_read), waiters.clone())?;
        let cdp = Cdp {
            tx: Mutex::new(File::from(cmd_write)),
            next_id: AtomicU64::new(1),
            waiters,
        };
        Ok((
            cdp,
            ChildFds {
                cmd_read,
                reply_write,
            },
        ))
    }

    pub fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        let (done, wait) = mpsc::channel();
        self.waiters.lock().insert(id, done);
        let mut bytes = serde_json::to_vec(&msg)?;
        bytes.push(0);
        let sent = self.tx.lock().write_all(&bytes);
        if let Err(e) = sent {
            self.waiters.lock().remove(&id);
            return Err(e.into());
        }
        let reply = wait.recv_timeout(CALL_TIMEOUT);
        self.waiters.lock().remove(&id);
        let reply = reply.map_err(|_| anyhow!("{method}: no reply"))?;
        if let Some(err) = reply.get("error") {
            return Err(anyhow!("{method}: {err}"));
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }
}

fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/* Routes replies by id. */
fn spawn_reader(reply: File, waiters: Waiters) -> Result<()> {
    std::thread::Builder::new()
        .name("nixlymedia-cdp".into())
        .spawn(move || {
            let mut reader = BufReader::new(reply);
            let mut buf = Vec::new();
            while reader
                .read_until(0, &mut buf)
                .map(|n| n > 0)
                .unwrap_or(false)
            {
                buf.pop();
                let id = serde_json::from_slice::<Value>(&buf)
                    .ok()
                    .and_then(|v| v.get("id").and_then(Value::as_u64).map(|id| (id, v)));
                if let Some((id, v)) = id {
                    if let Some(done) = waiters.lock().remove(&id) {
                        let _ = done.send(v);
                    }
                }
                buf.clear();
            }
        })?;
    Ok(())
}
