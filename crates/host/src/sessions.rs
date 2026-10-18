//! Session registry: SessionStart notes which tmux session an agent session runs in, SessionEnd
//! removes it. The app reads it with `ctl sessions`.

use crate::proc::run_with_input;
use crate::queue::{
    clean, is_hex, now, private_dir, rand_hex, read_small, unlink, write_new, Paths, MAX_FIELD,
    MAX_FILE, MAX_PENDING_FILES,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn session_file(p: &Paths, agent: &str, session: &str) -> PathBuf {
    let digest = hex::encode(Sha256::digest(format!("{agent}\0{session}").as_bytes()));
    p.sessions.join(format!("{}.json", &digest[..32]))
}

fn tmux_session_name() -> String {
    if std::env::var_os("TMUX").is_none_or(|v| v.is_empty()) {
        return String::new();
    }
    let mut cmd = Command::new("tmux");
    cmd.args(["display-message", "-p", "#{session_name}"]);
    match run_with_input(cmd, b"", Duration::from_secs(3)) {
        Some((Some(0), out)) => clean(String::from_utf8_lossy(&out).trim(), 100),
        _ => String::new(),
    }
}

pub fn register_session(p: &Paths, agent: &str, event: &str, session: &str, cwd: &str) {
    if session.is_empty() || !private_dir(&p.home) {
        return;
    }
    let mut dirs = std::fs::DirBuilder::new();
    dirs.mode(0o700).recursive(true);
    if dirs.create(&p.run).is_err() || dirs.create(&p.sessions).is_err() {
        return;
    }
    if !private_dir(&p.sessions) {
        return;
    }
    let path = session_file(p, agent, session);
    if event == "SessionEnd" {
        unlink(&path);
        return;
    }
    let count = std::fs::read_dir(&p.sessions).map_or(0, Iterator::count);
    if count >= MAX_PENDING_FILES {
        return;
    }
    let record = json!({
        "v": 1, "agent": agent, "session": clean(session, MAX_FIELD), "cwd": clean(cwd, MAX_FIELD),
        "tmux": tmux_session_name(), "ts": now(),
    });
    let tmp = p.sessions.join(format!(".tmp-{}", rand_hex(8)));
    if write_new(&tmp, record.to_string().as_bytes()).is_err()
        || std::fs::rename(&tmp, &path).is_err()
    {
        unlink(&tmp);
    }
}

fn is_session_name(name: &str) -> bool {
    name.strip_suffix(".json")
        .is_some_and(|stem| is_hex(stem, 32))
}

pub fn list_sessions(p: &Paths) -> Vec<Value> {
    let mut items = Vec::new();
    if !private_dir(&p.sessions) {
        return items;
    }
    let cutoff = now() - 7 * 86400;
    let Ok(rd) = std::fs::read_dir(&p.sessions) else {
        return items;
    };
    let mut names: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names.into_iter().take(MAX_PENDING_FILES) {
        if !is_session_name(&name) {
            continue;
        }
        let path = p.sessions.join(&name);
        let rec: Option<Value> = read_small(&path, MAX_FILE)
            .filter(|raw| !raw.is_empty())
            .and_then(|raw| serde_json::from_slice(&raw).ok());
        let fresh = rec.as_ref().and_then(Value::as_object).filter(|o| {
            o.get("ts")
                .and_then(Value::as_i64)
                .is_some_and(|ts| ts >= cutoff)
        });
        if let Some(o) = fresh {
            let pick = |k: &str| o.get(k).cloned().unwrap_or(Value::Null);
            items.push(json!({
                "agent": pick("agent"), "session": pick("session"), "cwd": pick("cwd"),
                "tmux": pick("tmux"), "ts": pick("ts"),
            }));
        } else if rec.is_some() {
            unlink(&path);
        }
    }
    items
}
