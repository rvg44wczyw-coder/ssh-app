//! The approval queue on the host (port of the queue half of `ssh-app-hook.py`; the script stays
//! the reference). Requests are files in `~/.ssh-app/run/pending`, the phone answers with an
//! SSHSIG-signed decision that is verified here against the hook's OWN copy of the request.

use crate::proc::run_with_input;
use crate::pyjson::{dumps, py_str};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const MAX_COMMAND: usize = 2000;
pub const MAX_FIELD: usize = 512;
pub const MAX_FILE: usize = 16384;
pub const MAX_SIG: usize = 8192;
pub const MAX_PENDING_FILES: usize = 100;
pub const MAX_AGE: i64 = 120;
pub const NOTIFY_TTL: i64 = 600;
const NAMESPACE: &str = "ssh-app-approval";
const PRINCIPAL: &str = "ssh-app";
pub const ANTIGRAVITY_CMD_KEYS: [&str; 7] = [
    "CommandLine",
    "commandLine",
    "command",
    "Command",
    "TargetFile",
    "AbsolutePath",
    "path",
];

/// Set by SIGTERM/SIGHUP so a waiting hook removes its pending file before it exits.
pub static TERMINATED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    TERMINATED.store(true, Ordering::SeqCst);
}

pub fn install_signal_handlers() {
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGHUP, on_signal as *const () as usize);
    }
}

pub fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Clone)]
pub struct Paths {
    pub home: PathBuf,
    pub run: PathBuf,
    pub pending: PathBuf,
    pub decisions: PathBuf,
    pub allowed: PathBuf,
    pub guard: PathBuf,
    pub sessions: PathBuf,
    pub limits: PathBuf,
}

impl Paths {
    pub fn from_env() -> Paths {
        let home = std::env::var_os("SSH_APP_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into()))
                    .join(".ssh-app")
            });
        let run = home.join("run");
        Paths {
            pending: run.join("pending"),
            decisions: run.join("decisions"),
            sessions: run.join("sessions"),
            limits: run.join("limits"),
            allowed: home.join("allowed_signers"),
            guard: home.join("guard.json"),
            run,
            home,
        }
    }

    pub fn queue_trusted(&self) -> bool {
        [&self.home, &self.run, &self.pending, &self.decisions]
            .iter()
            .all(|d| private_dir(d))
    }
}

pub fn timeout_secs() -> f64 {
    std::env::var("SSH_APP_HOOK_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(120.0)
}

/// Valid text truncated to `limit` characters.
pub fn clean(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

pub fn clean_value(value: &Value, limit: usize) -> String {
    clean(&py_str(value), limit)
}

pub fn summarize(tool_input: &Value) -> String {
    if let Value::Object(map) = tool_input {
        let keys = std::iter::once("command")
            .chain(ANTIGRAVITY_CMD_KEYS)
            .chain(std::iter::once("file_path"));
        for key in keys {
            if let Some(Value::String(s)) = map.get(key) {
                if !s.is_empty() {
                    return clean(s, MAX_COMMAND);
                }
            }
        }
        return clean(&dumps(tool_input), MAX_COMMAND);
    }
    clean_value(tool_input, MAX_COMMAND)
}

/// Binds everything the user is shown; every field is length-prefixed (feature 13, 4.1).
pub fn request_digest(agent: &str, tool: &str, cwd: &str, command: &str) -> String {
    let mut h = Sha256::new();
    for field in [agent, tool, cwd, command] {
        h.update(format!("{}:", field.len()).as_bytes());
        h.update(field.as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

pub fn private_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.is_dir() && m.uid() == euid() && m.mode() & 0o077 == 0)
        .unwrap_or(false)
}

pub fn allowed_signers_trusted(p: &Paths) -> bool {
    std::fs::symlink_metadata(&p.allowed)
        .map(|m| m.is_file() && m.uid() == euid() && m.mode() & 0o022 == 0)
        .unwrap_or(false)
}

pub fn keygen_path() -> Option<PathBuf> {
    for cand in ["/usr/bin/ssh-keygen", "/usr/local/bin/ssh-keygen"] {
        if let Ok(m) = std::fs::metadata(cand) {
            if m.is_file() && m.mode() & 0o111 != 0 {
                return Some(PathBuf::from(cand));
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("ssh-keygen"))
        .find(|c| {
            std::fs::metadata(c)
                .map(|m| m.is_file() && m.mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

/// A regular file (no symlink) of ours with at most `limit` bytes; never blocks on a FIFO.
pub fn read_small(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)
        .ok()?;
    let m = file.metadata().ok()?;
    if !m.is_file() || m.uid() != euid() || m.len() > limit as u64 {
        return None;
    }
    let mut buf = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut buf).ok()?;
    Some(buf)
}

pub fn write_new(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f: File = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(data)
}

pub fn unlink(path: &Path) {
    let _ = std::fs::remove_file(path);
}

pub fn rand_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    // A failing CSPRNG must never produce a predictable nonce: panic, the caller has no opinion.
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("no randomness");
    hex::encode(buf)
}

pub fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Removes leftovers of killed hooks and unacknowledged notifications.
pub fn sweep_stale(p: &Paths, now: i64) {
    for dir in [&p.pending, &p.decisions, &p.run] {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if let Ok(m) = std::fs::symlink_metadata(&path) {
                let mtime = m.mtime();
                if m.is_file() && now - mtime > NOTIFY_TTL + MAX_AGE {
                    unlink(&path);
                }
            }
        }
    }
}

/// `Some(approved)` for a verified decision, `None` when the file is not acceptable.
pub fn verify_decision(p: &Paths, req: &Value, raw: &[u8]) -> Option<bool> {
    let dec: Value = serde_json::from_slice(raw).ok()?;
    let dec = dec.as_object()?;
    if dec.get("id") != req.get("id") {
        return None;
    }
    let approved = dec.get("approved")?.as_bool()?;
    let dts = dec.get("ts")?.as_i64()?;
    let sig = dec.get("signature")?.as_str()?;
    let req_ts = req.get("ts")?.as_i64()?;
    if sig.is_empty() || sig.chars().count() > MAX_SIG {
        return None;
    }
    if !(req_ts <= dts && dts <= now() && dts - req_ts <= MAX_AGE) {
        return None;
    }
    let keygen = keygen_path()?;
    if !allowed_signers_trusted(p) {
        return None;
    }
    let message = format!(
        "SSH_APP_APPROVAL_V1:{}:{}:{}:{}:{}",
        req["id"].as_str()?,
        req["command_sha256"].as_str()?,
        req["nonce"].as_str()?,
        dts,
        if approved { "true" } else { "false" }
    );
    let sigfile = p.run.join(format!(".sig-{}", rand_hex(8)));
    write_new(&sigfile, sig.as_bytes()).ok()?;
    let mut cmd = Command::new(keygen);
    cmd.args(["-Y", "verify", "-f"])
        .arg(&p.allowed)
        .args(["-I", PRINCIPAL, "-n", NAMESPACE, "-s"])
        .arg(&sigfile);
    let result = run_with_input(cmd, message.as_bytes(), Duration::from_secs(5));
    unlink(&sigfile);
    match result {
        Some((Some(0), _)) => Some(approved),
        _ => None,
    }
}

pub struct Base<'a> {
    pub agent: &'a str,
    pub event: &'a str,
    pub tool: &'a str,
    pub command: &'a str,
    pub cwd: &'a str,
    pub session: &'a str,
}

fn make_request(kind: &str, base: &Base, now: i64) -> Value {
    let agent = clean(base.agent, MAX_FIELD);
    let tool = clean(base.tool, MAX_FIELD);
    let cwd = clean(base.cwd, MAX_FIELD);
    let command = clean(base.command, MAX_COMMAND);
    let ttl = if kind == "approval" {
        timeout_secs() as i64
    } else {
        NOTIFY_TTL
    };
    json!({
        "v": 1, "id": rand_hex(16), "kind": kind, "agent": agent,
        "event": clean(base.event, MAX_FIELD), "tool": tool, "command": command, "cwd": cwd,
        "session": clean(base.session, MAX_FIELD), "ts": now, "nonce": rand_hex(16),
        "command_sha256": request_digest(&agent, &tool, &cwd, &command), "expires": now + ttl,
    })
}

/// Queues a request. For approvals waits for a verified decision: `Some(bool)`, else `None`.
pub fn submit(p: &Paths, kind: &str, base: &Base, wait: bool) -> Option<bool> {
    if !p.queue_trusted() {
        return None;
    }
    let t = now();
    sweep_stale(p, t);
    if std::fs::read_dir(&p.pending).ok()?.count() >= MAX_PENDING_FILES {
        return None;
    }
    let req = make_request(kind, base, t);
    let id = req["id"].as_str()?.to_string();
    let path = p.pending.join(format!("{id}.json"));
    let tmp = p.pending.join(format!(".tmp-{id}"));
    let written =
        write_new(&tmp, req.to_string().as_bytes()).and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        unlink(&tmp);
        return None;
    }
    if !wait {
        return None; // notify entries are removed by the phone (ack) or the stale sweep
    }
    let verdict = wait_for_decision(p, &req, &id);
    unlink(&path);
    if let Ok(rd) = std::fs::read_dir(&p.decisions) {
        for e in rd.flatten() {
            if e.file_name()
                .to_string_lossy()
                .starts_with(&format!("{id}."))
            {
                unlink(&e.path());
            }
        }
    }
    verdict
}

fn wait_for_decision(p: &Paths, req: &Value, id: &str) -> Option<bool> {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout_secs().max(0.0));
    let prefix = format!("{id}.");
    let mut next_scan = Instant::now();
    while Instant::now() < deadline {
        if TERMINATED.load(Ordering::SeqCst) {
            return None;
        }
        if Instant::now() >= next_scan {
            let mut names: Vec<String> = std::fs::read_dir(&p.decisions)
                .ok()?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with(&prefix))
                .collect();
            names.sort();
            for name in names {
                if !is_decision_name(&name) {
                    continue;
                }
                let dpath = p.decisions.join(&name);
                let raw = read_small(&dpath, MAX_FILE);
                unlink(&dpath);
                if let Some(raw) = raw {
                    if let Some(v) = verify_decision(p, req, &raw) {
                        return Some(v);
                    }
                }
            }
            next_scan = Instant::now() + Duration::from_millis(500);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// `<32 hex>.<16 hex>.json`
fn is_decision_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('.').collect();
    parts.len() == 3 && is_hex(parts[0], 32) && is_hex(parts[1], 16) && parts[2] == "json"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_length_prefixed() {
        assert_ne!(
            request_digest("a", "bc", "", "d"),
            request_digest("a", "b", "c", "d")
        );
    }

    #[test]
    fn summarize_prefers_command_keys_and_falls_back_to_sorted_json() {
        assert_eq!(summarize(&json!({"command": "ls", "x": 1})), "ls");
        assert_eq!(
            summarize(&json!({"b": 1, "a": "é"})),
            r#"{"a": "\u00e9", "b": 1}"#
        );
        assert_eq!(summarize(&json!("plain")), "plain");
    }

    #[test]
    fn decision_names_are_strict() {
        let id = "0".repeat(32);
        assert!(is_decision_name(&format!("{id}.{}.json", "a".repeat(16))));
        assert!(!is_decision_name(&format!("{id}.{}.json", "A".repeat(16))));
        assert!(!is_decision_name(&format!("{id}..json")));
    }
}
