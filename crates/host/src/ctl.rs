//! `ctl`: what the app calls over SSH to read and answer the approval queue.

use crate::queue::{
    allowed_signers_trusted, is_hex, keygen_path, now, rand_hex, read_small, unlink, write_new,
    Paths, MAX_FILE, MAX_SIG,
};
use crate::sessions::list_sessions;
use serde_json::{json, Value};
use std::io::Read;
use std::time::{Duration, Instant};

const MAX_LIST: usize = 20;
const MAX_DECISIONS_PER_ID: usize = 5;
const WAIT_STEP: Duration = Duration::from_millis(400);

fn wait_max() -> Duration {
    let secs = std::env::var("SSH_APP_WAIT_MAX")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(20.0);
    Duration::from_secs_f64(secs.clamp(0.1, 20.0))
}

fn pending_id(name: &str) -> Option<&str> {
    name.strip_suffix(".json").filter(|id| is_hex(id, 32))
}

/// A valid, unexpired pending file as a JSON object, else `None`.
fn load_pending(p: &Paths, name: &str, now: i64) -> Option<Value> {
    let id = pending_id(name)?;
    let raw = read_small(&p.pending.join(name), MAX_FILE)?;
    let req: Value = serde_json::from_slice(&raw).ok()?;
    let o = req.as_object()?;
    if o.get("id")?.as_str()? != id || !matches!(o.get("kind")?.as_str()?, "approval" | "notify") {
        return None;
    }
    for k in ["ts", "expires"] {
        o.get(k)?.as_i64()?;
    }
    for k in [
        "agent",
        "event",
        "tool",
        "command",
        "cwd",
        "session",
        "nonce",
        "command_sha256",
    ] {
        o.get(k)?.as_str()?;
    }
    if o["expires"].as_i64()? <= now {
        return None;
    }
    // An approval whose hook is gone (the agent was killed or restarted) asks about nothing.
    if o["kind"] == "approval" {
        if let Some(pid) = o.get("pid").and_then(Value::as_i64).filter(|p| *p > 1) {
            if !process_alive(pid) {
                unlink(&p.pending.join(name));
                return None;
            }
        }
    }
    Some(req)
}

fn process_alive(pid: i64) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    // SAFETY: signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Every valid pending entry, oldest first (not truncated; the agent derives state from it).
pub fn all_pending(p: &Paths, now: i64) -> Vec<Value> {
    let mut items: Vec<Value> = std::fs::read_dir(&p.pending)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| load_pending(p, &e.file_name().to_string_lossy(), now))
                .collect()
        })
        .unwrap_or_default();
    items.sort_by(|a, b| {
        (a["ts"].as_i64(), a["id"].as_str()).cmp(&(b["ts"].as_i64(), b["id"].as_str()))
    });
    items
}

fn pending_items(p: &Paths, now: i64) -> Vec<Value> {
    let mut items = all_pending(p, now);
    items.truncate(MAX_LIST);
    items
}

fn read_stdin_json() -> Option<Value> {
    let mut raw = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut raw)
        .ok()?;
    if raw.len() > MAX_FILE {
        return None;
    }
    let v: Value = serde_json::from_slice(&raw).ok()?;
    Some(if v.is_object() { v } else { json!({}) })
}

fn count_pending(p: &Paths) -> usize {
    std::fs::read_dir(&p.pending).map_or(0, |rd| {
        rd.flatten()
            .filter(|e| pending_id(&e.file_name().to_string_lossy()).is_some())
            .count()
    })
}

fn ids_key(items: &[Value]) -> String {
    let mut ids: Vec<&str> = items.iter().filter_map(|r| r["id"].as_str()).collect();
    ids.sort_unstable();
    ids.join(",")
}

fn error(message: &str) -> Value {
    json!({"ok": false, "error": message})
}

pub fn run(p: &Paths, args: &[String]) -> Value {
    let sub = args.first().map_or("", String::as_str);
    let t = now();
    if sub == "status" {
        let pending = if p.queue_trusted() {
            count_pending(p)
        } else {
            0
        };
        return json!({
            "ok": true, "impl": "rust", "ssh_keygen": keygen_path().is_some(),
            "allowed_signers": allowed_signers_trusted(p), "pending": pending,
        });
    }
    if !p.queue_trusted() {
        return error("approval queue is missing or not private");
    }
    match sub {
        "sessions" => return json!({"host_time": t, "items": list_sessions(p)}),
        "auto-log" => return json!({"host_time": t, "items": crate::auto::log_items(p)}),
        "trusts" => return json!({"host_time": t, "items": crate::trust::items(p)}),
        "revoke-all" => {
            crate::trust::revoke_all(p);
            return json!({"ok": true});
        }
        "list" => return json!({"host_time": t, "items": pending_items(p, t)}),
        "wait" => {
            // Long poll: answers as soon as the set of pending ids differs from the ids the app
            // already shows (sorted, comma-separated), or after the maximum wait.
            let known = args.get(1).map_or("", String::as_str);
            let deadline = Instant::now() + wait_max();
            loop {
                let t = now();
                let items = pending_items(p, t);
                if ids_key(&items) != known || Instant::now() >= deadline {
                    return json!({"host_time": t, "items": items});
                }
                std::thread::sleep(WAIT_STEP);
            }
        }
        _ => {}
    }
    let Some(body) = read_stdin_json() else {
        return error("invalid request");
    };
    let Some(rid) = body
        .get("id")
        .and_then(Value::as_str)
        .filter(|i| is_hex(i, 32))
    else {
        return error("invalid id");
    };
    if sub == "revoke" {
        crate::trust::revoke(p, rid);
        return json!({"ok": true});
    }
    let req = load_pending(p, &format!("{rid}.json"), t);
    match sub {
        "ack" => {
            if req.is_some_and(|r| r["kind"] == "notify") {
                unlink(&p.pending.join(format!("{rid}.json")));
            }
            json!({"ok": true})
        }
        "decide" => decide(p, rid, &body, req.as_ref()),
        _ => error("unknown subcommand"),
    }
}

fn decide(p: &Paths, rid: &str, body: &Value, req: Option<&Value>) -> Value {
    if !req.is_some_and(|r| r["kind"] == "approval") {
        return error("request is gone or expired");
    }
    let approved = body.get("approved").and_then(Value::as_bool);
    let dts = body.get("ts").and_then(Value::as_i64);
    let sig = body
        .get("signature")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.chars().count() <= MAX_SIG);
    let (Some(approved), Some(dts), Some(sig)) = (approved, dts, sig) else {
        return error("invalid decision");
    };
    let prefix = format!("{rid}.");
    let existing = match std::fs::read_dir(&p.decisions) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .count(),
        Err(_) => return error("could not write the decision"),
    };
    if existing >= MAX_DECISIONS_PER_ID {
        return error("too many decisions for this request");
    }
    let mut data = json!({"v": 1, "id": rid, "approved": approved, "ts": dts, "signature": sig});
    // A trust block is verified (and stored) by the waiting hook, not here.
    if let Some(trust) = body
        .get("trust")
        .filter(|t| approved && t.is_object() && t.to_string().len() <= 12000)
    {
        data["trust"] = trust.clone();
    }
    let final_path = p.decisions.join(format!("{rid}.{}.json", rand_hex(8)));
    let tmp = p.decisions.join(format!(".tmp-{}", rand_hex(8)));
    if write_new(&tmp, data.to_string().as_bytes()).is_err()
        || std::fs::rename(&tmp, &final_path).is_err()
    {
        unlink(&tmp);
        return error("could not write the decision");
    }
    json!({"ok": true})
}

#[cfg(test)]
mod stale_tests {
    use super::process_alive;

    #[test]
    fn a_finished_hook_is_not_alive_and_this_process_is() {
        assert!(process_alive(i64::from(std::process::id())));
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = i64::from(child.id());
        child.wait().unwrap();
        assert!(!process_alive(pid));
    }
}
