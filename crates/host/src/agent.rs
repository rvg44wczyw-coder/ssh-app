//! The host agent (feature 26): lives while a phone has been in touch, exits when its lease runs
//! out. Serves a closed list of read-only methods on a private Unix socket and publishes agent
//! state changes to subscribers and to an [`EventSink`] (P3 plugs push in there).

use crate::ctl::all_pending;
use crate::queue::{euid, now, private_dir, Paths};
use crate::sessions::list_sessions;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const DEFAULT_LEASE: i64 = 12 * 3600;
pub const MAX_LEASE: i64 = 24 * 3600;
const MAX_LINE: u64 = 4096;
const MAX_CONNECTIONS: usize = 16;
const MAX_SOCKET_PATH: usize = 100;
const RESCAN: Duration = Duration::from_secs(600);
/// While a phone is subscribed the agent also re-checks the agents found in tmux this often.
const LIVE_RESCAN: Duration = Duration::from_secs(15);

/// SHA-256 of the running binary, so a newer helper can tell that the agent is an old build.
pub fn self_digest() -> String {
    use sha2::{Digest, Sha256};
    std::env::current_exe()
        .and_then(std::fs::read)
        .map(|b| hex::encode(Sha256::digest(b)))
        .unwrap_or_default()
}

pub fn socket_path(p: &Paths) -> PathBuf {
    p.run.join("hostd.sock")
}

/// What changed for one agent session. `to` is one of approval, finished, waiting, running, gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub agent: String,
    pub session: String,
    pub from: String,
    pub to: String,
    /// tmux session the agent runs in (for the push's opaque `sid`), when known.
    pub tmux: Option<String>,
}

pub trait EventSink: Send + Sync {
    fn on_event(&self, event: &Event);
}

/// P2 ships no sink; P3 adds the push one.
pub struct NoopSink;
impl EventSink for NoopSink {
    fn on_event(&self, _: &Event) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub state: &'static str,
    pub tmux: Value,
    pub cwd: Value,
    pub ts: Value,
}

pub type StateMap = BTreeMap<(String, String), Entry>;

/// State per (agent, session), derived only from the files the hooks already write.
pub fn compute_state(p: &Paths) -> (StateMap, usize) {
    let mut map = StateMap::new();
    for s in list_sessions(p) {
        let key = (
            s["agent"].as_str().unwrap_or("").to_string(),
            s["session"].as_str().unwrap_or("").to_string(),
        );
        map.insert(
            key,
            Entry {
                state: "running",
                tmux: s["tmux"].clone(),
                cwd: s["cwd"].clone(),
                ts: s["ts"].clone(),
            },
        );
    }
    let pending = all_pending(p, now());
    let count = pending.iter().filter(|r| r["kind"] == "approval").count();
    for kind in ["notify", "approval"] {
        for r in pending.iter().filter(|r| r["kind"] == kind) {
            let key = (
                r["agent"].as_str().unwrap_or("").to_string(),
                r["session"].as_str().unwrap_or("").to_string(),
            );
            let state = match (kind, r["event"].as_str()) {
                ("approval", _) => "approval",
                (_, Some("Stop")) => "finished",
                // The agent stopped without a closing answer (a failed step, a crash): it is not done,
                // it needs you ("An agent is waiting for you"), not "finished".
                (_, Some("StopFailed")) => "waiting",
                _ => "waiting",
            };
            let e = map.entry(key).or_insert(Entry {
                state,
                tmux: Value::Null,
                cwd: r["cwd"].clone(),
                ts: r["ts"].clone(),
            });
            e.state = state;
        }
    }
    (map, count)
}

pub fn diff(old: &StateMap, new: &StateMap) -> Vec<Event> {
    let mut out = Vec::new();
    for (k, v) in new {
        let from = old.get(k).map_or("none", |o| o.state);
        if from != v.state {
            // A finished agent's pending record has no tmux name: the earlier "running" entry does.
            let tmux = v
                .tmux
                .as_str()
                .filter(|t| !t.is_empty())
                .or_else(|| old.get(k).and_then(|o| o.tmux.as_str()))
                .filter(|t| !t.is_empty())
                .map(String::from);
            out.push(Event {
                agent: k.0.clone(),
                session: k.1.clone(),
                from: from.into(),
                to: v.state.into(),
                tmux,
            });
        }
    }
    for (k, o) in old {
        if !new.contains_key(k) {
            out.push(Event {
                agent: k.0.clone(),
                session: k.1.clone(),
                from: o.state.into(),
                to: "gone".into(),
                tmux: o.tmux.as_str().map(String::from),
            });
        }
    }
    out
}

/// How long a usage report counts as fresh; an older one is served at once and refreshed behind.
const USAGE_FRESH: Duration = Duration::from_secs(10);
const MAX_USAGE_CACHE: usize = 8;

struct Shared {
    usage: std::collections::HashMap<String, (std::time::Instant, Value)>,
    refreshing: std::collections::HashSet<String>,
    lease_until: i64,
    subs: Vec<Sender<String>>,
    conns: usize,
}

fn extend(shared: &Mutex<Shared>, secs: i64) -> i64 {
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    s.lease_until = s.lease_until.max(now() + secs.clamp(1, MAX_LEASE));
    s.lease_until
}

/// Whether the process on the other end of the socket runs as the same user.
#[cfg(not(target_os = "linux"))]
fn peer_is_us(stream: &UnixStream) -> bool {
    let (mut uid, mut gid) = (0 as libc::uid_t, 0 as libc::gid_t);
    // SAFETY: valid descriptor and valid out-pointers for the duration of the call.
    let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    rc == 0 && uid == euid()
}

#[cfg(target_os = "linux")]
fn peer_is_us(stream: &UnixStream) -> bool {
    // SAFETY: a zeroed `ucred` is a valid out-buffer; `len` matches its size.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            std::ptr::addr_of_mut!(cred).cast::<libc::c_void>(),
            &mut len,
        )
    };
    rc == 0 && cred.uid == euid()
}

fn reply(out: &Mutex<UnixStream>, v: &Value) -> bool {
    let mut line = v.to_string();
    line.push('\n');
    out.lock()
        .map(|mut s| s.write_all(line.as_bytes()).is_ok())
        .unwrap_or(false)
}

fn serve(p: Arc<Paths>, shared: Arc<Mutex<Shared>>, build: Arc<String>, stream: UnixStream) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let out = Arc::new(Mutex::new(writer));
    let mut reader = BufReader::new(stream);
    let mut subscribed = false;
    loop {
        let mut line = Vec::new();
        let n = (&mut reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut line);
        if n.unwrap_or(0) == 0 {
            break;
        }
        if line.len() as u64 > MAX_LINE {
            reply(&out, &json!({"ok": false, "error": "request too long"}));
            break;
        }
        let Ok(req) = serde_json::from_slice::<Value>(&line) else {
            if !reply(&out, &json!({"ok": false, "error": "invalid request"})) {
                break;
            }
            continue;
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let body = match req.get("m").and_then(Value::as_str) {
            Some("hello") => json!({
                "ok": true, "name": "ssh-app-host-agent", "version": env!("CARGO_PKG_VERSION"),
                "pid": std::process::id(), "build": build.as_str(),
                "lease_until": shared.lock().map_or(0, |s| s.lease_until),
            }),
            Some("state") => {
                let (mut map, pending) = compute_state(&p);
                merge_detected(&mut map, &shared);
                let sessions: Vec<Value> = map
                    .iter()
                    .map(|(k, e)| {
                        json!({"agent": k.0, "session": k.1, "state": e.state,
                               "tmux": e.tmux, "cwd": e.cwd, "ts": e.ts})
                    })
                    .collect();
                json!({"ok": true, "host_time": now(), "sessions": sessions, "pending": pending})
            }
            Some("subscribe") => {
                let (tx, rx) = channel::<String>();
                {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if subscribed || s.subs.len() >= MAX_CONNECTIONS {
                        drop(s);
                        reply(
                            &out,
                            &json!({"id": id, "ok": false, "error": "already subscribed or too many subscribers"}),
                        );
                        continue;
                    }
                    s.subs.push(tx);
                }
                subscribed = true;
                let sink = out.clone();
                std::thread::spawn(move || {
                    while let Ok(line) = rx.recv() {
                        let ok = sink
                            .lock()
                            .map(|mut s| s.write_all(line.as_bytes()).is_ok())
                            .unwrap_or(false);
                        if !ok {
                            break;
                        }
                    }
                });
                json!({"ok": true})
            }
            Some("quit") => {
                // Asked by a newer helper that finds this build outdated.
                reply(&out, &json!({"id": id, "ok": true}));
                let _ = std::fs::remove_file(socket_path(&p));
                std::process::exit(0);
            }
            Some("usage") => usage_reply(&shared, &req),
            Some("lease") => {
                let secs = req
                    .get("secs")
                    .and_then(Value::as_i64)
                    .unwrap_or(DEFAULT_LEASE);
                json!({"ok": true, "lease_until": extend(&shared, secs)})
            }
            _ => json!({"ok": false, "error": "unknown method"}),
        };
        let mut body = body;
        body["id"] = id;
        if !reply(&out, &body) {
            break;
        }
    }
}

/// Agents running in tmux panes that no hook has reported yet (started before the hooks were
/// installed, or by an agent without hooks), taken from the cached usage scan as `running`.
fn merge_detected(map: &mut StateMap, shared: &Arc<Mutex<Shared>>) {
    let reply = usage_reply(shared, &json!({"dirs": []}));
    let Some(sessions) = reply["report"]["sessions"].as_array() else {
        return;
    };
    for s in sessions {
        let (Some(name), Some(agent)) = (s["session"].as_str(), s["agent"].as_str()) else {
            continue;
        };
        let agent = if agent == "agy" { "antigravity" } else { agent };
        let known = map
            .iter()
            .any(|(k, e)| k.0 == agent && e.tmux.as_str() == Some(name));
        if !known {
            map.insert(
                (agent.to_string(), name.to_string()),
                Entry {
                    state: "running",
                    tmux: json!(name),
                    cwd: Value::Null,
                    ts: Value::Null,
                },
            );
        }
    }
}

/// Agents whose questions are read from the tmux screen (no hook reports them).
const SCREEN_WATCHED: &str = "antigravity";
const SCREEN_POLL: Duration = Duration::from_secs(5);

/// A running agent that shows a menu on its screen is `waiting` (the push for a question).
fn mark_questions(map: &mut StateMap) {
    for ((agent, _), e) in map.iter_mut() {
        if agent == SCREEN_WATCHED && e.state == "running" {
            let asks = e
                .tmux
                .as_str()
                .and_then(crate::screenwatch::capture)
                .is_some_and(|s| crate::screenwatch::asks(&s));
            if asks {
                e.state = "waiting";
            }
        }
    }
}

/// Usage report with stale-while-revalidate: the first request per set of config directories scans
/// (seconds), later ones are answered from memory and refreshed in the background when stale.
fn usage_reply(shared: &Arc<Mutex<Shared>>, req: &Value) -> Value {
    let dirs: Vec<String> = req
        .get("dirs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| d.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let dirs = match crate::usage::validate_dirs(&dirs) {
        Ok(d) => d,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    let key = dirs.join("\u{1f}");
    let cached = {
        let s = shared.lock().unwrap_or_else(|e| e.into_inner());
        s.usage.get(&key).cloned()
    };
    let store = |shared: &Mutex<Shared>, key: &str, report: &Value| {
        let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
        if s.usage.len() >= MAX_USAGE_CACHE && !s.usage.contains_key(key) {
            s.usage.clear();
        }
        s.usage
            .insert(key.to_string(), (std::time::Instant::now(), report.clone()));
    };
    match cached {
        Some((at, report)) => {
            let stale = at.elapsed() >= USAGE_FRESH;
            if stale {
                let start = shared
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .refreshing
                    .insert(key.clone());
                if start {
                    let shared = shared.clone();
                    std::thread::spawn(move || {
                        let fresh = crate::usage::report(&dirs);
                        store(&shared, &key, &fresh);
                        shared
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .refreshing
                            .remove(&key);
                    });
                }
            }
            json!({"ok": true, "cached": true, "stale": stale, "report": report})
        }
        None => {
            let report = crate::usage::report(&dirs);
            store(shared, &key, &report);
            json!({"ok": true, "cached": false, "stale": false, "report": report})
        }
    }
}

#[cfg(target_os = "macos")]
struct Watcher {
    kq: libc::c_int,
    _dirs: Vec<std::fs::File>,
}

#[cfg(target_os = "macos")]
impl Watcher {
    fn new(paths: &[&PathBuf]) -> Option<Watcher> {
        // SAFETY: kqueue takes no arguments.
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return None;
        }
        let mut dirs = Vec::new();
        for path in paths {
            let f = std::fs::File::open(path).ok()?;
            let ev = libc::kevent {
                ident: f.as_raw_fd() as libc::uintptr_t,
                filter: libc::EVFILT_VNODE,
                flags: libc::EV_ADD | libc::EV_CLEAR,
                fflags: libc::NOTE_WRITE
                    | libc::NOTE_EXTEND
                    | libc::NOTE_DELETE
                    | libc::NOTE_RENAME,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            // SAFETY: one valid change record, no event list requested.
            let rc = unsafe { libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
            if rc < 0 {
                return None;
            }
            dirs.push(f);
        }
        Some(Watcher { kq, _dirs: dirs })
    }

    /// Blocks until a watched directory changes or `timeout` passes.
    fn wait(&self, timeout: Duration) {
        let ts = libc::timespec {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_nsec: timeout.subsec_nanos() as libc::c_long,
        };
        // SAFETY: zeroed kevent is a valid out-buffer; ts outlives the call.
        let mut out: libc::kevent = unsafe { std::mem::zeroed() };
        unsafe { libc::kevent(self.kq, std::ptr::null(), 0, &mut out, 1, &ts) };
    }
}

#[cfg(target_os = "macos")]
impl Drop for Watcher {
    fn drop(&mut self) {
        // SAFETY: we own the descriptor.
        unsafe { libc::close(self.kq) };
    }
}

#[cfg(not(target_os = "macos"))]
struct Watcher;
#[cfg(not(target_os = "macos"))]
impl Watcher {
    fn new(_: &[&PathBuf]) -> Option<Watcher> {
        Some(Watcher)
    }
    fn wait(&self, timeout: Duration) {
        std::thread::sleep(timeout.min(Duration::from_secs(1)));
    }
}

fn prepare(p: &Paths) -> Option<()> {
    if !private_dir(&p.home) {
        return None;
    }
    let mut b = std::fs::DirBuilder::new();
    b.mode(0o700).recursive(true);
    for d in [&p.run, &p.pending, &p.decisions, &p.sessions] {
        b.create(d).ok()?;
    }
    (private_dir(&p.run) && private_dir(&p.sessions)).then_some(())
}

/// Runs the agent until its lease expires. Exit code 0 also when another agent already runs.
pub fn run(p: Paths, lease_secs: i64, sink: &dyn EventSink) -> i32 {
    let sock = socket_path(&p);
    if sock.as_os_str().len() > MAX_SOCKET_PATH || prepare(&p).is_none() {
        return 1;
    }
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(p.run.join("hostd.lock"))
    else {
        return 1;
    };
    // SAFETY: valid descriptor; the lock lives as long as `lock` does.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return 0;
    }
    let _ = std::fs::remove_file(&sock);
    let Ok(listener) = UnixListener::bind(&sock) else {
        return 1;
    };
    let _ = std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600));
    let shared = Arc::new(Mutex::new(Shared {
        usage: Default::default(),
        refreshing: Default::default(),
        lease_until: now() + lease_secs.clamp(1, MAX_LEASE),
        subs: Vec::new(),
        conns: 0,
    }));
    let paths = Arc::new(p);
    let build = Arc::new(self_digest());
    {
        let (shared, paths, build) = (shared.clone(), paths.clone(), build.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if !peer_is_us(&stream) {
                    continue;
                }
                {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if s.conns >= MAX_CONNECTIONS {
                        continue;
                    }
                    s.conns += 1;
                }
                let (shared, paths, build) = (shared.clone(), paths.clone(), build.clone());
                std::thread::spawn(move || {
                    serve(paths, shared.clone(), build, stream);
                    shared.lock().unwrap_or_else(|e| e.into_inner()).conns -= 1;
                });
            }
        });
    }
    let watcher = Watcher::new(&[&paths.home, &paths.pending, &paths.sessions]);
    let live_rescan = std::env::var("SSH_APP_LIVE_RESCAN_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(LIVE_RESCAN, Duration::from_millis);
    let observed = |shared: &Arc<Mutex<Shared>>| {
        let (mut map, pending) = compute_state(&paths);
        merge_detected(&mut map, shared);
        mark_questions(&mut map);
        (map, pending)
    };
    let (mut last, _) = observed(&shared);
    loop {
        let remaining = shared.lock().map_or(0, |s| s.lease_until) - now();
        if remaining <= 0 {
            break;
        }
        let watched = shared.lock().is_ok_and(|s| !s.subs.is_empty());
        let mut tick = if watched { live_rescan } else { RESCAN };
        // an agent without hooks: its questions show only on the screen, so look at it often
        if last.keys().any(|k| k.0 == SCREEN_WATCHED) {
            tick = tick.min(SCREEN_POLL);
        }
        let timeout = Duration::from_secs(remaining as u64).min(tick);
        match &watcher {
            Some(w) => w.wait(timeout),
            None => std::thread::sleep(timeout.min(Duration::from_secs(2))),
        }
        // The state folder was removed (uninstall, or a test's temp home): nothing left to serve.
        if !paths.home.is_dir() {
            break;
        }
        let (current, _) = observed(&shared);
        for ev in diff(&last, &current) {
            sink.on_event(&ev);
            let line = format!(
                "{}\n",
                json!({"event": "state", "agent": ev.agent, "session": ev.session,
                       "from": ev.from, "to": ev.to})
            );
            let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
            s.subs.retain(|tx| tx.send(line.clone()).is_ok());
        }
        last = current;
    }
    let _ = std::fs::remove_file(&sock);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: &'static str) -> Entry {
        Entry {
            state,
            tmux: Value::Null,
            cwd: Value::Null,
            ts: Value::Null,
        }
    }

    #[test]
    fn diff_reports_new_changed_and_gone_sessions() {
        let key = |s: &str| ("claude".to_string(), s.to_string());
        let mut old = StateMap::new();
        old.insert(key("a"), entry("running"));
        old.insert(key("b"), entry("waiting"));
        let mut new = StateMap::new();
        new.insert(key("a"), entry("approval"));
        new.insert(key("c"), entry("running"));
        let ev: Vec<(String, String, String)> = diff(&old, &new)
            .into_iter()
            .map(|e| (e.session, e.from, e.to))
            .collect();
        assert_eq!(
            ev,
            vec![
                ("a".into(), "running".into(), "approval".into()),
                ("c".into(), "none".into(), "running".into()),
                ("b".into(), "waiting".into(), "gone".into()),
            ]
        );
    }
}
